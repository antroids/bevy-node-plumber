use crate::graph::ProviderState;
use crate::node::{add_or_replace_graph_node, DispatchWorkgroupsStrategy, DummyNode};
use crate::resource::{BindResourceCreationInfo, NodeResources};
use crate::{MainWorldEntity, NodeProvider};
use bevy::ecs::query::QueryItem;
use bevy::log::debug;
use bevy::prelude::*;
use bevy_render::extract_component::ExtractComponent;
use bevy_render::render_resource::PipelineCache;
use bevy_render::renderer::RenderContext;
use bevy_render::{render_graph, render_resource};
use std::any::type_name;
use std::borrow::Cow;

#[derive(Component, Clone, Debug)]
pub struct ComputeNode {
    pub label: Option<Cow<'static, str>>,
    pub bind_group_index: u32,
    pub pipeline_descriptor: render_resource::ComputePipelineDescriptor,
    pub binding_resource_info: Vec<BindResourceCreationInfo>,
    pub dispatch_workgroups_strategy: DispatchWorkgroupsStrategy,

    pub(crate) state: ComputeNodeState,
}

#[derive(Clone, Debug)]
pub(crate) enum ComputeNodeState {
    Creating,
    PipelineQueued {
        pipeline_id: render_resource::CachedComputePipelineId,
    },
    PipelineCached {
        layout: render_resource::BindGroupLayout,
        pipeline: render_resource::ComputePipeline,
    },
    ReadyToRun {
        node: ComputeNodeImpl,
    },
    Err(String),
}

#[derive(Clone, Debug)]
pub(crate) struct ComputeNodeImpl {
    label: Option<Cow<'static, str>>,
    bind_group_index: u32,
    layout: render_resource::BindGroupLayout,
    pipeline: render_resource::ComputePipeline,
    bind_resources: NodeResources,
    input_slots: Vec<render_graph::SlotInfo>,
    output_slots: Vec<render_graph::SlotInfo>,
    dispatch_workgroups_strategy: DispatchWorkgroupsStrategy,
}

impl render_graph::Node for ComputeNodeImpl {
    fn input(&self) -> Vec<render_graph::SlotInfo> {
        self.input_slots.clone()
    }

    fn output(&self) -> Vec<render_graph::SlotInfo> {
        self.output_slots.clone()
    }

    fn run(
        &self,
        graph: &mut render_graph::RenderGraphContext,
        render_context: &mut RenderContext,
        _world: &World,
    ) -> Result<(), render_graph::NodeRunError> {
        let render_device = render_context.render_device().clone();
        let command_encoder = render_context.command_encoder();
        let bind_group = self
            .bind_resources
            .set_bind_group(&render_device, graph, &self.layout)?;
        let workgroups = self
            .dispatch_workgroups_strategy
            .workgroups_to_dispatch(graph);
        self.bind_resources
            .set_output_slots(graph, &render_device)?;

        {
            let mut pass =
                command_encoder.begin_compute_pass(&render_resource::ComputePassDescriptor {
                    label: Some(type_name::<Self>()),
                });

            pass.set_bind_group(self.bind_group_index, &bind_group, &[]);
            pass.set_pipeline(&self.pipeline);
            pass.dispatch_workgroups(workgroups.0, workgroups.1, workgroups.2);

            debug!(
                "Dispatched Compute pass {:?} with {:?} workgroups",
                &self.label, &workgroups
            );
        }
        Ok(())
    }
}

impl NodeProvider for ComputeNode {
    fn update(&mut self, _world: &mut World) {
        let pipeline_cache = _world.resource::<PipelineCache>();
        let new_state = match &self.state {
            ComputeNodeState::Creating => ComputeNodeState::PipelineQueued {
                pipeline_id: pipeline_cache
                    .queue_compute_pipeline(self.pipeline_descriptor.clone()),
            },
            ComputeNodeState::PipelineQueued { pipeline_id } => {
                match pipeline_cache.get_compute_pipeline_state(*pipeline_id) {
                    render_resource::CachedPipelineState::Ok(
                        render_resource::Pipeline::ComputePipeline(pipeline),
                    ) => {
                        let cached_pipeline = pipeline_cache
                            .get_compute_pipeline(*pipeline_id)
                            .expect("Cannot find Compute pipeline with status Ok in cache");
                        let layout = self
                            .pipeline_descriptor
                            .layout
                            .get(self.bind_group_index as usize)
                            .cloned()
                            .unwrap_or(
                                cached_pipeline
                                    .get_bind_group_layout(self.bind_group_index)
                                    .into(),
                            );
                        let pipeline = pipeline.clone();
                        ComputeNodeState::PipelineCached { layout, pipeline }
                    }
                    render_resource::CachedPipelineState::Err(err) => {
                        ComputeNodeState::Err(err.to_string())
                    }
                    _ => {
                        return;
                    }
                }
            }
            ComputeNodeState::PipelineCached { layout, pipeline } => {
                let (input_slots, output_slots) =
                    BindResourceCreationInfo::input_output_slot_info(&self.binding_resource_info);

                ComputeNodeState::ReadyToRun {
                    node: ComputeNodeImpl {
                        label: self.label.clone(),
                        bind_group_index: self.bind_group_index,
                        layout: layout.clone(),
                        pipeline: pipeline.clone(),
                        bind_resources: NodeResources::from_bind_resource_info(
                            self.binding_resource_info.clone(),
                        ),
                        input_slots,
                        output_slots,
                        dispatch_workgroups_strategy: self.dispatch_workgroups_strategy.clone(),
                    },
                }
            }
            _ => {
                return;
            }
        };
        debug!("Compute node state after update: {:?}", &new_state);
        self.state = new_state;
    }

    fn state(&self) -> ProviderState {
        match &self.state {
            ComputeNodeState::ReadyToRun { .. } => ProviderState::CanCreateNode,
            ComputeNodeState::Err(s) => ProviderState::Err(s.clone()),
            _ => ProviderState::Updating,
        }
    }

    fn add_node_to_graph(
        &self,
        graph: &mut render_graph::RenderGraph,
        node_name: Cow<'static, str>,
    ) {
        match &self.state {
            ComputeNodeState::ReadyToRun { node } => {
                let node = node.clone();
                debug!("Added node impl: {:?} {:?}", &node_name, &node);
                add_or_replace_graph_node(graph, node_name, node);
            }
            _ => {
                let node = DummyNode::from_bind_resource_info(&self.binding_resource_info);
                debug!("Added dummy node: {:?} {:?}", &node_name, &node);
                add_or_replace_graph_node(graph, node_name, node);
            }
        };
    }
}

impl ExtractComponent for ComputeNode {
    type Query = (&'static Self, Entity);
    type Filter = Changed<Self>;
    type Out = (Self, MainWorldEntity);

    fn extract_component(item: QueryItem<'_, Self::Query>) -> Option<Self::Out> {
        Some((item.0.clone(), MainWorldEntity(item.1)))
    }
}
