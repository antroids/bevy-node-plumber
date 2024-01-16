use bevy_render::renderer::RenderDevice;
use bevy_render::{render_graph, render_resource};
use std::borrow::Cow;
use std::fmt::Debug;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BindResourceCreationStrategy<T: Clone + Debug + PartialEq> {
    Static(T),
    FromGraphContext(fn(&render_graph::RenderGraphContext) -> T),
}

#[derive(Clone, Debug, PartialEq)]
pub enum BindResourceCreationDescriptor {
    Buffer(BindResourceCreationStrategy<render_resource::BufferDescriptor<'static>>),
    Sampler(BindResourceCreationStrategy<render_resource::SamplerDescriptor<'static>>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum StaticBindResourceCreationDescriptor {
    Buffer(render_resource::BufferDescriptor<'static>),
    Sampler(render_resource::SamplerDescriptor<'static>),
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
        }
    }

    pub(crate) fn to_slot_type(&self) -> render_graph::SlotType {
        match self {
            BindResourceCreationDescriptor::Buffer(_) => render_graph::SlotType::Buffer,
            BindResourceCreationDescriptor::Sampler(_) => render_graph::SlotType::Sampler,
        }
    }
}

#[derive(Clone, Debug)]
pub enum OwnBindResource {
    Buffer(render_resource::Buffer),
    Sampler(render_resource::Sampler),
}

impl OwnBindResource {
    pub(crate) fn to_slot_value(&self) -> render_graph::SlotValue {
        match self {
            OwnBindResource::Buffer(buffer) => render_graph::SlotValue::Buffer(buffer.clone()),
            OwnBindResource::Sampler(sampler) => render_graph::SlotValue::Sampler(sampler.clone()),
        }
    }

    pub(crate) fn as_binding_resource(&self) -> render_resource::BindingResource {
        match self {
            OwnBindResource::Buffer(buffer) => buffer.as_entire_binding(),
            OwnBindResource::Sampler(sampler) => render_resource::BindingResource::Sampler(sampler),
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
