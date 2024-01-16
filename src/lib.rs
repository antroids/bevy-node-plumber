use crate::graph::{ProviderState, SubGraph, SubGraphDeployState, SubGraphPlugin};
use crate::node::compute::ComputeNode;
use crate::node::output::OutputBufferPlugin;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::utils::HashMap;
use bevy_render::render_graph::RenderGraph;
use bevy_render::{MainWorld, RenderApp};
use std::any::TypeId;
use std::borrow::Cow;
use std::fmt::Debug;
use std::marker::PhantomData;

pub mod builder;
pub mod graph;
pub mod node;
pub mod resource;

pub mod prelude {
    pub use crate::builder;
    pub use crate::NodePlumberPlugin;

    pub use crate::resource::BindResourceCreationDescriptor;
    pub use crate::resource::BindResourceCreationInfo;
    pub use crate::resource::BindResourceDirection;

    pub use crate::graph;
    pub use crate::node::compute;
    pub use crate::node::input;
    pub use crate::node::input::InputBuffer;
    pub use crate::node::output;
}

pub struct NodePlumberPlugin;

impl Plugin for NodePlumberPlugin {
    fn build(&self, app: &mut App) {
        app.init_schedule(UpdateProvidersSchedule);
        app.init_schedule(PostUpdateProvidersSchedule);
        app.init_schedule(UpdateGraphSchedule);

        app.add_plugins(OutputBufferPlugin);
        app.add_plugins(SubGraphPlugin);
        app.add_plugins(ProviderPlugin::<ComputeNode>::default());
    }

    fn finish(&self, app: &mut App) {
        let render_app = app
            .get_sub_app_mut(RenderApp)
            .expect("Cannot find Render Plugin");
        render_app.init_schedule(UpdateProvidersSchedule);
        render_app.init_schedule(PostUpdateProvidersSchedule);
        render_app.add_systems(
            ExtractSchedule,
            (
                run_update_providers_schedule,
                run_providers_schedule_in_main_world,
                run_post_update_providers_schedule,
                run_update_graph_schedule,
            )
                .chain(),
        );
    }
}

#[derive(ScheduleLabel, PartialEq, Eq, Debug, Clone, Hash)]
pub struct UpdateProvidersSchedule;

#[derive(ScheduleLabel, PartialEq, Eq, Debug, Clone, Hash)]
pub struct PostUpdateProvidersSchedule;

#[derive(ScheduleLabel, PartialEq, Eq, Debug, Clone, Hash)]
pub struct UpdateGraphSchedule;

fn run_update_providers_schedule(world: &mut World) {
    world.run_schedule(UpdateProvidersSchedule);
}

fn run_post_update_providers_schedule(world: &mut World) {
    world.run_schedule(PostUpdateProvidersSchedule);
}

fn run_update_graph_schedule(world: &mut World) {
    world.run_schedule(UpdateGraphSchedule);
}

fn run_providers_schedule_in_main_world(mut main_world: ResMut<MainWorld>) {
    main_world.run_schedule(UpdateProvidersSchedule);
    main_world.run_schedule(PostUpdateProvidersSchedule);
}

pub struct ProviderPlugin<T: Component + ProviderSystem + Sized>(PhantomData<T>);

impl<T: Component + ProviderSystem + Sized> Default for ProviderPlugin<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<T: Component + ProviderSystem + Sized> ProviderPlugin<T> {
    fn deploy_nodes_from_providers_in_render_world(
        mut main_world: ResMut<MainWorld>,
        mut render_graph: ResMut<RenderGraph>,
    ) {
        let mut queue = std::mem::replace(
            &mut main_world
                .resource_mut::<ProviderToRenderGraphQueue<T>>()
                .queue,
            default(),
        );

        for (sub_graph_name, entities) in queue.drain() {
            if let Some(sub_graph) = render_graph.get_sub_graph_mut(sub_graph_name) {
                for (node_name, provider_entity) in entities {
                    if let Ok(provider) = main_world.query::<&T>().get(&main_world, provider_entity)
                    {
                        provider.add_node_to_graph(sub_graph, node_name);
                    }
                }
            }
        }
    }

    fn update_graph_from_providers_in_main_world(
        providers_query: Query<&T, Changed<T>>,
        mut graph_components: Query<&mut SubGraph>,
        mut queue: ResMut<ProviderToRenderGraphQueue<T>>,
    ) {
        for mut graph_component in graph_components.iter_mut() {
            let graph_component_entities: Vec<Entity> =
                graph_component.providers.keys().copied().collect();
            for entity in graph_component_entities {
                let Ok(provider) = providers_query.get(entity) else {
                    // Component entity is not found in updated components
                    continue;
                };
                let sub_graph_name = graph_component.name.clone();
                if let Some(descriptor) = graph_component.providers.get_mut(&entity) {
                    if descriptor.ty == TypeId::of::<T>() {
                        let new_state = provider.state();
                        debug!(
                        "Updating sub graph {:?} component node descriptor {:?} with new state {:?}",
                        &sub_graph_name, &descriptor, &new_state
                    );
                        descriptor.state = new_state;
                    } else {
                        // ProviderDescriptor is for another Provider type and
                        // should be processed by update_providers with another generic parameter
                        continue;
                    }

                    let node_name = descriptor.name.clone();
                    if let SubGraphDeployState::Queued(_, graph) = &mut graph_component.graph {
                        provider.add_node_to_graph(graph, node_name);
                    } else {
                        queue
                            .queue
                            .entry(sub_graph_name)
                            .or_insert(default())
                            .insert(node_name, entity);
                    }
                }
            }
        }
    }
}

impl<T: Component + ProviderSystem + Sized> Plugin for ProviderPlugin<T> {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProviderToRenderGraphQueue<T>>();
        app.add_systems(
            PostUpdateProvidersSchedule,
            Self::update_graph_from_providers_in_main_world,
        );
    }

    fn finish(&self, app: &mut App) {
        let render_app = app
            .get_sub_app_mut(RenderApp)
            .expect("Cannot find Render Plugin");
        render_app.add_systems(
            PostUpdateProvidersSchedule,
            Self::deploy_nodes_from_providers_in_render_world,
        );
        T::add_systems_to_render_world(render_app);
    }
}

pub trait Provider {
    fn state(&self) -> ProviderState;
    fn add_node_to_graph(&self, graph: &mut RenderGraph, node_name: Cow<'static, str>);
}

pub trait ProviderSystem: Provider {
    fn add_systems_to_render_world(_app: &mut App) {}
}

#[derive(Resource)]
pub(crate) struct ProviderToRenderGraphQueue<T: Provider> {
    queue: HashMap<Cow<'static, str>, HashMap<Cow<'static, str>, Entity>>,
    phantom: PhantomData<T>,
}

impl<T: Provider> Default for ProviderToRenderGraphQueue<T> {
    fn default() -> Self {
        Self {
            queue: Default::default(),
            phantom: Default::default(),
        }
    }
}
