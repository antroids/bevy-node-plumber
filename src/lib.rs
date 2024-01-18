use crate::graph::{ProviderState, SubGraphCache, SubGraphDeployState, SubGraphPlugin};
use crate::node::compute::ComputeNode;
use crate::node::output::OutputBufferPlugin;
use bevy::prelude::*;
use bevy::utils::HashMap;
use bevy_render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy_render::render_graph::RenderGraph;
use bevy_render::RenderSet::PrepareAssets;
use bevy_render::{Render, RenderApp};
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
    pub use crate::node::DispatchWorkgroupsStrategy;
}

pub struct NodePlumberPlugin;

impl Plugin for NodePlumberPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(OutputBufferPlugin);
        app.add_plugins(SubGraphPlugin);
        app.add_plugins(NodeProviderPlugin::<ComputeNode>::default());
    }
}

#[derive(Clone, Component, Debug)]
pub struct MainWorldEntity(Entity);

pub struct NodeProviderPlugin<T: Component + Sized>(PhantomData<T>);

impl<T: Component + Sized> Default for NodeProviderPlugin<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<T: NodeProvider + Sized> NodeProviderPlugin<T> {
    fn update_sub_graphs(
        providers_cache: Res<NodeProviderCache<T>>,
        mut sub_graph_cache: ResMut<SubGraphCache>,
        mut render_graph: ResMut<RenderGraph>,
    ) {
        for graph_component in sub_graph_cache.0.values_mut() {
            let graph_component_entities: Vec<Entity> =
                graph_component.providers.keys().copied().collect();
            for entity in graph_component_entities {
                let Some(provider) = providers_cache.0.get(&entity) else {
                    // Component entity is not found in updated components
                    continue;
                };
                let sub_graph_name = graph_component.name.clone();
                if let Some(descriptor) = graph_component.providers.get_mut(&entity) {
                    if descriptor.ty == TypeId::of::<T>() {
                        let new_state = provider.state();
                        if descriptor.state == new_state {
                            continue;
                        }
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
                    match &mut graph_component.graph {
                        SubGraphDeployState::Queued(_, graph) => {
                            provider.add_node_to_graph(graph, node_name);
                        }
                        SubGraphDeployState::MovedToRenderWorld => {}
                        SubGraphDeployState::Deployed => {
                            // Replace by only Node impl, dummy node should not be added to deployed graph
                            if provider.state() == ProviderState::CanCreateNode {
                                if let Some(sub_graph) =
                                    render_graph.get_sub_graph_mut(&sub_graph_name)
                                {
                                    provider.add_node_to_graph(sub_graph, node_name)
                                } else {
                                    error!("Sub graph {} not found", sub_graph_name);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

impl<T: NodeProvider + Sized> Plugin for NodeProviderPlugin<T> {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<T>::default());
        app.add_systems(PostUpdate, on_node_provider_component_changed::<T>);
    }

    fn finish(&self, app: &mut App) {
        let render_app = app
            .get_sub_app_mut(RenderApp)
            .expect("Cannot find Render Plugin");
        render_app.init_resource::<NodeProviderCache<T>>();
        render_app.add_systems(
            Render,
            NodeProviderCache::<T>::update_system.in_set(PrepareAssets),
        );
        render_app.add_systems(Render, Self::update_sub_graphs.in_set(PrepareAssets));
    }
}

fn on_node_provider_component_changed<T: NodeProvider>(mut query: Query<&mut T, Changed<T>>) {
    for mut provider in query.iter_mut() {
        provider.on_component_changed();
    }
}

pub trait NodeProvider: Component + Clone + ExtractComponent {
    fn on_component_changed(&mut self) {}
    fn update(&mut self, _world: &mut World) {}
    fn state(&self) -> ProviderState;
    fn add_node_to_graph(&self, graph: &mut RenderGraph, node_name: Cow<'static, str>);
}

#[derive(Resource)]
pub(crate) struct NodeProviderCache<T: NodeProvider>(HashMap<Entity, T>);

impl<T: NodeProvider> Default for NodeProviderCache<T> {
    fn default() -> Self {
        Self(default())
    }
}

impl<T: NodeProvider> NodeProviderCache<T> {
    fn update_system(world: &mut World) {
        world.resource_scope(|world, mut cache: Mut<Self>| {
            cache.update(world);
        });
    }

    fn update(&mut self, world: &mut World) {
        let mut query = world.query::<(&T, &MainWorldEntity)>();

        for (provider_component, entity) in query.iter(world) {
            self.0.insert(entity.0, provider_component.clone());
        }

        for provider in self.0.values_mut() {
            provider.update(world);
        }
    }
}
