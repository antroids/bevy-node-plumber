use bevy::prelude::*;
use bevy::utils::HashMap;
use bevy_render::render_graph::OutputSlotError;
use bevy_render::render_resource::TextureViewDescriptor;
use bevy_render::renderer::RenderDevice;
use bevy_render::{render_graph, render_resource};
use std::borrow::Cow;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BindResourceCreationStrategy<T: Clone + Debug + PartialEq> {
    Static(T),
    FromGraphContext(fn(&render_graph::RenderGraphContext) -> T),
}

#[derive(Clone, Debug, PartialEq)]
pub enum BindResourceCreationDescriptor {
    Buffer(BindResourceCreationStrategy<render_resource::BufferDescriptor<'static>>),
    Sampler(BindResourceCreationStrategy<render_resource::SamplerDescriptor<'static>>),
    Texture(BindResourceCreationStrategy<render_resource::TextureDescriptor<'static>>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum StaticBindResourceCreationDescriptor {
    Buffer(render_resource::BufferDescriptor<'static>),
    Sampler(render_resource::SamplerDescriptor<'static>),
    Texture(render_resource::TextureDescriptor<'static>),
}

impl StaticBindResourceCreationDescriptor {
    pub(crate) fn create_resource(&self, render_device: &RenderDevice) -> OwnBindResource {
        match self {
            StaticBindResourceCreationDescriptor::Buffer(buffer_descriptor) => {
                OwnBindResource::Buffer(render_device.create_buffer(buffer_descriptor))
            }
            StaticBindResourceCreationDescriptor::Sampler(sampler_descriptor) => {
                OwnBindResource::Sampler(render_device.create_sampler(sampler_descriptor))
            }
            StaticBindResourceCreationDescriptor::Texture(texture_descriptor) => {
                let texture = render_device.create_texture(texture_descriptor);
                let default_view = texture.create_view(&TextureViewDescriptor::default());
                OwnBindResource::Texture(texture, default_view)
            }
        }
    }
}

impl BindResourceCreationDescriptor {
    pub(crate) fn into_static(
        self,
        graph_context: &render_graph::RenderGraphContext,
    ) -> StaticBindResourceCreationDescriptor {
        match self {
            BindResourceCreationDescriptor::Buffer(b) => {
                StaticBindResourceCreationDescriptor::Buffer(match b {
                    BindResourceCreationStrategy::Static(s) => s,
                    BindResourceCreationStrategy::FromGraphContext(f) => f(graph_context),
                })
            }
            BindResourceCreationDescriptor::Sampler(s) => {
                StaticBindResourceCreationDescriptor::Sampler(match s {
                    BindResourceCreationStrategy::Static(s) => s,
                    BindResourceCreationStrategy::FromGraphContext(f) => f(graph_context),
                })
            }
            BindResourceCreationDescriptor::Texture(t) => {
                StaticBindResourceCreationDescriptor::Texture(match t {
                    BindResourceCreationStrategy::Static(s) => s,
                    BindResourceCreationStrategy::FromGraphContext(f) => f(graph_context),
                })
            }
        }
    }

    pub(crate) fn to_slot_type(&self) -> render_graph::SlotType {
        match self {
            BindResourceCreationDescriptor::Buffer(_) => render_graph::SlotType::Buffer,
            BindResourceCreationDescriptor::Sampler(_) => render_graph::SlotType::Sampler,
            BindResourceCreationDescriptor::Texture(_) => render_graph::SlotType::TextureView,
        }
    }
}

#[derive(Clone, Debug)]
pub enum OwnBindResource {
    Buffer(render_resource::Buffer),
    Sampler(render_resource::Sampler),
    Texture(render_resource::Texture, render_resource::TextureView),
}

impl OwnBindResource {
    pub(crate) fn to_slot_value(&self) -> render_graph::SlotValue {
        match self {
            OwnBindResource::Buffer(buffer) => render_graph::SlotValue::Buffer(buffer.clone()),
            OwnBindResource::Sampler(sampler) => render_graph::SlotValue::Sampler(sampler.clone()),
            OwnBindResource::Texture(_, view) => render_graph::SlotValue::TextureView(view.clone()),
        }
    }

    pub(crate) fn as_binding_resource(&self) -> render_resource::BindingResource {
        match self {
            OwnBindResource::Buffer(buffer) => buffer.as_entire_binding(),
            OwnBindResource::Sampler(sampler) => render_resource::BindingResource::Sampler(sampler),
            OwnBindResource::Texture(_, view) => {
                render_resource::BindingResource::TextureView(view)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BindResourceDirection {
    Input(render_graph::SlotType),
    Output(BindResourceCreationDescriptor),
    InputOutput(render_graph::SlotType),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindResourceCreationInfo {
    pub name: Cow<'static, str>,
    pub binding: u32,
    pub direction: BindResourceDirection,
}

impl BindResourceCreationInfo {
    pub(crate) fn input_output_slot_info<'a>(
        iterator: impl IntoIterator<Item = &'a BindResourceCreationInfo>,
    ) -> (Vec<render_graph::SlotInfo>, Vec<render_graph::SlotInfo>) {
        let mut input_slots: Vec<render_graph::SlotInfo> = default();
        let mut output_slots: Vec<render_graph::SlotInfo> = default();

        for bind_resource_info in iterator {
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
                    let slot_info =
                        render_graph::SlotInfo::new(bind_resource_info.name.clone(), *slot_type);
                    input_slots.push(slot_info.clone());
                    output_slots.push(slot_info);
                }
            }
        }

        (input_slots, output_slots)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct NodeResources {
    bind_resource_info: Vec<BindResourceCreationInfo>,
    bind_resource_cache:
        Arc<Mutex<HashMap<usize, (StaticBindResourceCreationDescriptor, OwnBindResource)>>>,
}

impl NodeResources {
    pub(crate) fn from_bind_resource_info(
        bind_resource_info: Vec<BindResourceCreationInfo>,
    ) -> Self {
        Self {
            bind_resource_info,
            bind_resource_cache: default(),
        }
    }

    pub(crate) fn set_bind_group(
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
                            resource: slot_value_to_bind_resource(value),
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

    pub(crate) fn set_output_slots(
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

    pub(crate) fn get_output_resource(
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
