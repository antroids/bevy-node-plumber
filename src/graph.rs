use bevy::log::warn;
use bevy::prelude::*;
use bevy::utils::HashMap;
use bevy_render::render_graph::{NodeRunError, RenderGraph, RenderGraphContext, SlotInfo};
use bevy_render::renderer::RenderContext;
use bevy_render::RenderSet::PrepareResources;
use bevy_render::{render_graph, MainWorld, Render, RenderApp};
use std::any::TypeId;
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct SubGraphPlugin;

impl Plugin for SubGraphPlugin {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        let render_app = app
            .get_sub_app_mut(RenderApp)
            .expect("Cannot find Render Plugin");
        render_app.init_resource::<SubGraphCache>();
        render_app.add_systems(ExtractSchedule, SubGraph::extract_to_render_world);
        render_app.add_systems(
            Render,
            SubGraphCache::update_system.in_set(PrepareResources),
        );
    }
}

#[derive(Debug, Clone)]
pub struct ProviderDescriptor {
    pub(crate) name: Cow<'static, str>,
    pub(crate) ty: TypeId,
    pub(crate) state: ProviderState,
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub enum ProviderState {
    #[default]
    Created,
    Updating,
    CanCreateNode,
    Err(String),
}

#[derive(Debug, Clone)]
pub enum Edge {
    InputSlotEdge {
        output_node: render_graph::NodeLabel,
        output_slot: render_graph::SlotLabel,
        input_slot: render_graph::SlotLabel,
    },
    InputNodeEdge {
        output_node: render_graph::NodeLabel,
    },
    OutputNodeEdge {
        input_node: render_graph::NodeLabel,
    },
}

#[derive(Debug)]
pub(crate) enum SubGraphDeployState {
    Queued(Vec<Edge>, RenderGraph),
    MovedToRenderWorld,
    Deployed,
}

#[derive(Component, Debug, Clone, Default)]
pub enum SubGraphTrigger {
    #[default]
    Always,
    Manual(Arc<AtomicBool>),
}

#[derive(Component, Debug)]
pub struct SubGraph {
    pub(crate) name: Cow<'static, str>,
    pub(crate) providers: HashMap<Entity, ProviderDescriptor>,
    pub(crate) graph: SubGraphDeployState,
    pub(crate) trigger: SubGraphTrigger,
}

impl SubGraph {
    pub fn providers_state_summary(&self) -> ProviderState {
        let mut has_created = false;
        let mut has_updating = false;

        for (_, descriptor) in &self.providers {
            match &descriptor.state {
                ProviderState::Updating => {
                    has_updating = true;
                }
                ProviderState::Created => {
                    has_created = true;
                }
                ProviderState::Err(err) => return ProviderState::Err(err.clone()),
                _ => {}
            }
        }

        if has_created {
            ProviderState::Created
        } else if has_updating {
            ProviderState::Updating
        } else {
            ProviderState::CanCreateNode
        }
    }

    fn extract_to_render_world(
        mut main_world: ResMut<MainWorld>,
        mut sub_graph_cache: ResMut<SubGraphCache>,
    ) {
        let mut query = main_world.query::<(&mut Self, Entity)>();

        for (mut sub_graph, entity) in query.iter_mut(&mut main_world) {
            if matches!(sub_graph.graph, SubGraphDeployState::Queued(..)) {
                let graph = std::mem::replace(
                    &mut sub_graph.graph,
                    SubGraphDeployState::MovedToRenderWorld,
                );
                sub_graph_cache.0.insert(
                    entity,
                    SubGraph {
                        name: sub_graph.name.clone(),
                        providers: sub_graph.providers.clone(),
                        graph,
                        trigger: sub_graph.trigger.clone(),
                    },
                );
            }
        }
    }
}

#[derive(Resource, Default)]
pub struct SubGraphCache(pub(crate) HashMap<Entity, SubGraph>);

impl SubGraphCache {
    fn update_system(world: &mut World) {
        world.resource_scope(|world, mut cache: Mut<Self>| {
            cache.update(world);
        });
    }

    fn update(&mut self, world: &mut World) {
        let mut render_graph = world.resource_mut::<RenderGraph>();
        for sub_graph in self.0.values_mut() {
            if matches!(sub_graph.graph, SubGraphDeployState::Queued(..))
                && matches!(
                    sub_graph.providers_state_summary(),
                    ProviderState::CanCreateNode
                )
            {
                let queued = std::mem::replace(&mut sub_graph.graph, SubGraphDeployState::Deployed);
                let SubGraphDeployState::Queued(edges, graph) = queued else {
                    unreachable!()
                };
                let name = sub_graph.name.clone();
                let node_name = render_graph::NodeLabel::Name(name.clone());
                let runner = SubGraphRunnerNode {
                    sub_graph_name: name.clone(),
                    node_inputs: graph.input_node().input_slots.iter().cloned().collect(),
                    trigger: sub_graph.trigger.clone(),
                };
                render_graph.add_sub_graph(name.clone(), graph);
                render_graph.add_node(name.clone(), runner);
                for edge in edges {
                    match edge {
                        Edge::InputSlotEdge {
                            output_node,
                            output_slot,
                            input_slot,
                        } => render_graph.add_slot_edge(
                            output_node,
                            output_slot,
                            node_name.clone(),
                            input_slot,
                        ),
                        Edge::InputNodeEdge { output_node } => {
                            render_graph.add_node_edge(output_node, node_name.clone());
                        }
                        Edge::OutputNodeEdge { input_node } => {
                            render_graph.add_node_edge(node_name.clone(), input_node);
                        }
                    }
                }
            }
        }
    }
}

#[derive(Component, Debug, Clone)]
pub struct SubGraphRunnerNode {
    sub_graph_name: Cow<'static, str>,
    node_inputs: Vec<SlotInfo>,
    trigger: SubGraphTrigger,
}

impl render_graph::Node for SubGraphRunnerNode {
    fn input(&self) -> Vec<SlotInfo> {
        self.node_inputs.clone()
    }

    fn run(
        &self,
        graph: &mut RenderGraphContext,
        _render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), NodeRunError> {
        match &self.trigger {
            SubGraphTrigger::Manual(manual) => {
                if !manual.swap(false, Ordering::Relaxed) {
                    debug!("Manual subgraph trigger condition is not met, skipping");
                    return Ok(());
                }
            }
            SubGraphTrigger::Always => {}
        }

        let render_graph = world.resource::<RenderGraph>();

        let mut sub_graph_inputs =
            HashMap::<Cow<'static, str>, render_graph::SlotValue>::with_capacity(
                self.node_inputs.len(),
            );

        for node_input in &self.node_inputs {
            sub_graph_inputs.insert(
                node_input.name.clone(),
                graph.get_input(node_input.name.clone())?.clone(),
            );
        }

        if let Some(sub_graph) = render_graph.get_sub_graph(&self.sub_graph_name) {
            let mut input_values = Vec::with_capacity(sub_graph_inputs.len());
            // Creating an input values vector with the same order as in sub-graph because
            // mapping by input name is not supported there
            for (index, info) in sub_graph.input_node().input_slots.iter().enumerate() {
                if let Some(value) = sub_graph_inputs.remove(&info.name) {
                    input_values.push(value);
                } else {
                    return Err(NodeRunError::RunSubGraphError(
                        render_graph::RunSubGraphError::MissingInput {
                            slot_index: index,
                            slot_name: info.name.clone(),
                            graph_name: self.sub_graph_name.clone(),
                        },
                    ));
                }
            }
            graph.run_sub_graph(self.sub_graph_name.clone(), input_values, None)?;
        } else {
            warn!("Sub graph with name {} not found!", &self.sub_graph_name);
        }

        Ok(())
    }
}
