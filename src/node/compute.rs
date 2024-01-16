use crate::graph::ProviderState;
use crate::node::{add_or_replace_graph_node, DummyNode};
use crate::resource::{
    BindResourceCreationInfo, BindResourceDirection, OwnBindResource,
    StaticBindResourceCreationDescriptor,
};
use crate::{Provider, ProviderSystem, UpdateProvidersSchedule};
use bevy::app::App;
use bevy::log::debug;
use bevy::prelude::{default, Component, Mut, ResMut, World};
use bevy::utils::HashMap;
use bevy_render::render_graph::OutputSlotError;
use bevy_render::renderer::{RenderContext, RenderDevice};
use bevy_render::{render_graph, render_resource, MainWorld};
use std::any::type_name;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};

#[derive(Component, Clone, Debug)]
pub struct ComputeNode {
    pub(crate) label: Option<Cow<'static, str>>,
    pub(crate) bind_group_index: u32,
    pub(crate) state: ComputeNodeState,
    pub(crate) binding_resource_info: Vec<BindResourceCreationInfo>,
    pub(crate) dispatch_workgroups_strategy: DispatchWorkgroupsStrategy,
}

#[derive(Clone, Debug)]
pub(crate) enum ComputeNodeState {
    Creating {
        pipeline_descriptor: render_resource::ComputePipelineDescriptor,
    },
    PipelineQueued {
        pipeline_descriptor: render_resource::ComputePipelineDescriptor,
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

#[derive(Debug, Clone)]
pub enum DispatchWorkgroupsStrategy {
    Static(u32, u32, u32),
    FromGraphContext(fn(&render_graph::RenderGraphContext) -> (u32, u32, u32)),
}

impl Default for DispatchWorkgroupsStrategy {
    fn default() -> Self {
        Self::Static(1, 1, 1)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ComputeNodeImpl {
    label: Option<Cow<'static, str>>,
    bind_group_index: u32,
    layout: render_resource::BindGroupLayout,
    pipeline: render_resource::ComputePipeline,
    bind_resource_info: Vec<BindResourceCreationInfo>,
    bind_resource_cache:
        Arc<Mutex<HashMap<usize, (StaticBindResourceCreationDescriptor, OwnBindResource)>>>,
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
        let bind_group = self.set_bind_group(&render_device, graph, &self.layout)?;
        let workgroups = self.workgroups_to_dispatch(graph);
        self.set_output_slots(graph, &render_device)?;

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

impl ComputeNodeImpl {
    fn set_bind_group(
        &self,
        render_device: &RenderDevice,
        graph: &render_graph::RenderGraphContext,
        layout: &render_resource::BindGroupLayout,
    ) -> Result<render_resource::BindGroup, render_graph::NodeRunError> {
        let mut entries: Vec<render_resource::BindGroupEntry> = default();
        let mut output_resources: Vec<(u32, OwnBindResource)> = default();

        for (index, info) in self.bind_resource_info.iter().enumerate() {
            match &info.direction {
                BindResourceDirection::Input(_) | BindResourceDirection::InputOutput(_) => {
                    if let Ok(value) = graph.get_input(info.name.clone()) {
                        entries.push(render_resource::BindGroupEntry {
                            binding: info.binding,
                            resource: ComputeNode::slot_value_to_bind_resource(value),
                        });
                    } else {
                        return Err(render_graph::NodeRunError::InputSlotError(
                            render_graph::InputSlotError::InvalidSlot(info.name.clone().into()),
                        ));
                    }
                }
                BindResourceDirection::Output(_) => {
                    output_resources.push((
                        info.binding,
                        self.get_output_resource(index, graph, render_device)?,
                    ));
                }
            }
        }

        for (binding, output_resource) in &output_resources {
            entries.push(render_resource::BindGroupEntry {
                binding: *binding,
                resource: output_resource.as_binding_resource(),
            });
        }
        let bind_group = render_device.create_bind_group(None, layout, &entries);

        Ok(bind_group)
    }

    fn set_output_slots(
        &self,
        graph: &mut render_graph::RenderGraphContext,
        render_device: &RenderDevice,
    ) -> Result<(), render_graph::NodeRunError> {
        for (index, info) in self.bind_resource_info.iter().enumerate() {
            match info.direction {
                BindResourceDirection::Output(_) => {
                    let label: render_graph::SlotLabel = info.name.clone().into();
                    graph.set_output(
                        label,
                        self.get_output_resource(index, graph, render_device)?
                            .to_slot_value(),
                    )?;
                }
                BindResourceDirection::InputOutput(_) => {
                    let label: render_graph::SlotLabel = info.name.clone().into();
                    graph.set_output(label.clone(), graph.get_input(label)?.clone())?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn workgroups_to_dispatch(&self, graph: &render_graph::RenderGraphContext) -> (u32, u32, u32) {
        match self.dispatch_workgroups_strategy {
            DispatchWorkgroupsStrategy::Static(x, y, z) => (x, y, z),
            DispatchWorkgroupsStrategy::FromGraphContext(from_graph) => from_graph(graph),
        }
    }

    fn get_output_resource(
        &self,
        index: usize,
        graph: &render_graph::RenderGraphContext,
        render_device: &RenderDevice,
    ) -> Result<OwnBindResource, render_graph::NodeRunError> {
        let Some(BindResourceCreationInfo {
            direction: BindResourceDirection::Output(descriptor),
            ..
        }) = self.bind_resource_info.get(index)
        else {
            return Err(render_graph::NodeRunError::OutputSlotError(
                OutputSlotError::InvalidSlot(index.into()),
            ));
        };
        let mut cache = self
            .bind_resource_cache
            .lock()
            .expect("Bind Resource cache mutex is poisoned");
        let static_descriptor = descriptor.clone().into_static(graph);
        if let Some((cached_static_descriptor, cached_resource)) = cache.get(&index) {
            if cached_static_descriptor == &static_descriptor {
                debug!("Output Bind Resource {:?} found in cache", &descriptor);
                return Ok(cached_resource.clone());
            }
        };
        let resource = static_descriptor.create_resource(render_device);
        debug!(
            "Output Bind Resource {:?} missing in cache, created new: {:?}",
            &descriptor, &resource
        );
        cache.insert(index, (static_descriptor, resource));
        Ok(cache.get(&index).expect("Must be inserted").1.clone())
    }
}

impl ProviderSystem for ComputeNode {
    fn add_systems_to_render_world(app: &mut App) {
        app.add_systems(UpdateProvidersSchedule, ComputeNode::update_in_render_world);
    }
}

impl Provider for ComputeNode {
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

impl ComputeNode {
    pub fn update_in_render_world(
        mut main_world: ResMut<MainWorld>,
        mut pipeline_cache: ResMut<render_resource::PipelineCache>,
    ) {
        let mut query = main_world.query::<&mut ComputeNode>();

        for node in query.iter_mut(main_world.as_mut()) {
            Self::update_state(node, pipeline_cache.reborrow());
        }
    }

    fn update_state(mut mut_self: Mut<Self>, pipeline_cache: Mut<render_resource::PipelineCache>) {
        mut_self.state = match &mut_self.state {
            ComputeNodeState::Creating {
                pipeline_descriptor: pipeline,
            } => ComputeNodeState::PipelineQueued {
                pipeline_descriptor: pipeline.clone(),
                pipeline_id: pipeline_cache.queue_compute_pipeline(pipeline.clone()),
            },
            ComputeNodeState::PipelineQueued {
                pipeline_id,
                pipeline_descriptor: descriptor,
            } => match pipeline_cache.get_compute_pipeline_state(*pipeline_id) {
                render_resource::CachedPipelineState::Ok(
                    render_resource::Pipeline::ComputePipeline(pipeline),
                ) => {
                    let cached_pipeline = pipeline_cache
                        .get_compute_pipeline(*pipeline_id)
                        .expect("Cannot find Compute pipeline with status Ok in cache");
                    let layout = descriptor
                        .layout
                        .get(mut_self.bind_group_index as usize)
                        .map_or(
                            cached_pipeline
                                .get_bind_group_layout(mut_self.bind_group_index)
                                .into(),
                            |l| l.clone(),
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
            },
            ComputeNodeState::PipelineCached { layout, pipeline } => {
                let mut input_slots: Vec<render_graph::SlotInfo> = default();
                let mut output_slots: Vec<render_graph::SlotInfo> = default();

                for bind_resource_info in &mut_self.binding_resource_info {
                    match &bind_resource_info.direction {
                        BindResourceDirection::Input(slot_type) => {
                            input_slots.push(render_graph::SlotInfo::new(
                                bind_resource_info.name.clone(),
                                *slot_type,
                            ));
                        }
                        BindResourceDirection::Output(bind_resource_descriptor) => {
                            let slot_info = render_graph::SlotInfo::new(
                                bind_resource_info.name.clone(),
                                bind_resource_descriptor.to_slot_type(),
                            );
                            output_slots.push(slot_info);
                        }
                        BindResourceDirection::InputOutput(slot_type) => {
                            let slot_info = render_graph::SlotInfo::new(
                                bind_resource_info.name.clone(),
                                *slot_type,
                            );
                            input_slots.push(slot_info.clone());
                            output_slots.push(slot_info);
                        }
                    }
                }

                ComputeNodeState::ReadyToRun {
                    node: ComputeNodeImpl {
                        label: mut_self.label.clone(),
                        bind_group_index: mut_self.bind_group_index,
                        layout: layout.clone(),
                        pipeline: pipeline.clone(),
                        bind_resource_info: mut_self.binding_resource_info.clone(),
                        bind_resource_cache: default(),
                        input_slots,
                        output_slots,
                        dispatch_workgroups_strategy: mut_self.dispatch_workgroups_strategy.clone(),
                    },
                }
            }
            _ => {
                return;
            }
        };
        debug!("Compute node state after update: {:?}", &mut_self.state);
    }

    fn slot_value_to_bind_resource(
        slot_value: &render_graph::SlotValue,
    ) -> render_resource::BindingResource {
        match slot_value {
            render_graph::SlotValue::Buffer(buffer) => buffer.as_entire_binding(),
            render_graph::SlotValue::TextureView(texture_view) => {
                render_resource::BindingResource::TextureView(texture_view)
            }

            render_graph::SlotValue::Sampler(sampler) => {
                render_resource::BindingResource::Sampler(sampler)
            }
            render_graph::SlotValue::Entity(_) => todo!(),
        }
    }
}
