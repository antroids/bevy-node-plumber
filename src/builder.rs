use crate::graph::{
    Edge, ProviderDescriptor, ProviderState, SubGraph, SubGraphDeployState, SubGraphTrigger,
};
use crate::prelude::compute::ComputeNodeState;
use crate::prelude::*;
use crate::resource::BindResourceCreationStrategy;
use crate::NodeProvider;
use bevy::prelude::*;
use bevy::utils::HashMap;
use bevy_render::render_graph::{
    NodeLabel, RenderGraph, RenderGraphError, SlotInfo, SlotLabel, SlotType,
};
use bevy_render::render_resource::{
    BindGroupLayout, BufferAddress, BufferUsages, ComputePipelineDescriptor, PushConstantRange,
    ShaderDefVal,
};
use bevy_render::{render_graph, render_resource};
use std::any::TypeId;
use std::borrow::Cow;
use thiserror::Error;

macro_rules! option_setter {
    ($field_name:ident: $field_type:ty) => {
        pub fn $field_name(mut self, $field_name: $field_type) -> Self {
            self.$field_name = Some($field_name);
            self
        }
    };
}

macro_rules! option_into_setter {
    ($field_name:ident: $field_type:ty) => {
        pub fn $field_name(mut self, $field_name: impl Into<$field_type>) -> Self {
            self.$field_name = Some($field_name.into());
            self
        }
    };
}

#[derive(Error, Debug)]
pub enum BuilderError {
    #[error("Builder value `{0}` is mandatory, but was not defined")]
    ValueNotDefined(&'static str),
    #[error("Builder validation error: `{0}`")]
    ValidationError(String),
    #[error("Render graph error: `{0}`")]
    RenderGraphError(#[from] RenderGraphError),
}

impl From<&'static str> for BuilderError {
    fn from(value: &'static str) -> Self {
        Self::ValueNotDefined(value)
    }
}

pub type BuildResult<T> = Result<T, BuilderError>;
pub type BuildResultFn<P, T> = Box<dyn FnOnce(P, BuildResult<T>) -> P>;
pub type BuildFn<P, T> = Box<dyn FnOnce(P, T) -> P>;

#[derive(Default)]
pub struct ComputeNodeBuilder {
    label: Option<Cow<'static, str>>,

    // Pipeline
    bind_group_index: Option<u32>,
    bind_group_layout: Option<Vec<BindGroupLayout>>,
    push_constant_ranges: Option<Vec<PushConstantRange>>,
    shader: Option<Handle<Shader>>,
    shader_defs: Option<Vec<ShaderDefVal>>,
    entry_point: Option<Cow<'static, str>>,

    bind_resources: Vec<BuildResult<BindResourceCreationInfo>>,

    dispatch_workgroups_strategy: Option<DispatchWorkgroupsStrategy>,
}

impl ComputeNodeBuilder {
    option_into_setter!(label: Cow<'static, str>);
    option_setter!(bind_group_index: u32);
    option_setter!(bind_group_layout: Vec<BindGroupLayout>);
    option_setter!(push_constant_ranges: Vec<PushConstantRange>);
    option_setter!(shader: Handle<Shader>);
    option_setter!(shader_defs: Vec<ShaderDefVal>);
    option_into_setter!(entry_point: Cow<'static, str>);
    option_setter!(dispatch_workgroups_strategy: DispatchWorkgroupsStrategy);

    pub fn bind_resource(self) -> AddBindResourceInfoBuilder<Self> {
        AddBindResourceInfoBuilder::new(
            self,
            Box::new(|mut parent, result| -> Self {
                parent.bind_resources.push(result);
                parent
            }),
        )
    }

    pub fn build(mut self) -> BuildResult<compute::ComputeNode> {
        let bind_resource: BuildResult<Vec<BindResourceCreationInfo>> =
            self.bind_resources.drain(..).collect();

        Ok(compute::ComputeNode {
            label: self.label.clone(),
            bind_group_index: self.bind_group_index.unwrap_or(0),
            pipeline_descriptor: ComputePipelineDescriptor {
                label: self.label,
                layout: self.bind_group_layout.unwrap_or_default(),
                push_constant_ranges: self.push_constant_ranges.unwrap_or_default(),
                shader: self.shader.ok_or(BuilderError::ValueNotDefined("shader"))?,
                shader_defs: self.shader_defs.unwrap_or_default(),
                entry_point: self
                    .entry_point
                    .ok_or(BuilderError::ValueNotDefined("entry_point"))?,
            },
            binding_resource_info: bind_resource?,
            dispatch_workgroups_strategy: self.dispatch_workgroups_strategy.ok_or(
                BuilderError::ValueNotDefined("dispatch_workgroups_strategy"),
            )?,
            state: ComputeNodeState::Creating,
        })
    }
}

pub struct AddBindResourceInfoBuilder<P> {
    parent: P,
    build_fn: BuildResultFn<P, BindResourceCreationInfo>,

    name: Option<Cow<'static, str>>,
    binding: Option<u32>,

    direction: Option<BuildResult<BindResourceDirection>>,
}

impl<P> AddBindResourceInfoBuilder<P> {
    fn new(parent: P, build_fn: BuildResultFn<P, BindResourceCreationInfo>) -> Self {
        Self {
            parent,
            build_fn,
            name: None,
            binding: None,
            direction: None,
        }
    }

    option_into_setter!(name: Cow<'static, str>);
    option_setter!(binding: u32);

    pub fn add(self) -> P {
        let r = || {
            Ok(BindResourceCreationInfo {
                name: self.name.ok_or(BuilderError::ValueNotDefined("name"))?,
                binding: self.binding.unwrap_or(0),
                direction: self
                    .direction
                    .ok_or(BuilderError::ValueNotDefined("direction"))??,
            })
        };

        (self.build_fn)(self.parent, r())
    }

    pub fn input(self) -> SetSlotTypeBuilder<Self> {
        SetSlotTypeBuilder {
            parent: self,
            build_fn: Box::new(|mut parent, v| -> Self {
                parent.direction = Some(Ok(BindResourceDirection::Input(v)));
                parent
            }),
        }
    }

    pub fn add_input(mut self, slot_type: SlotType) -> P {
        self.direction = Some(Ok(BindResourceDirection::Input(slot_type)));
        self.parent
    }

    pub fn input_output(self) -> SetSlotTypeBuilder<Self> {
        SetSlotTypeBuilder {
            parent: self,
            build_fn: Box::new(|mut parent, v| -> Self {
                parent.direction = Some(Ok(BindResourceDirection::InputOutput(v)));
                parent
            }),
        }
    }

    pub fn add_input_output(mut self, slot_type: SlotType) -> P {
        self.direction = Some(Ok(BindResourceDirection::InputOutput(slot_type)));
        self.parent
    }

    pub fn output(self) -> SetBindResourceDescriptorBuilder<Self> {
        SetBindResourceDescriptorBuilder {
            parent: self,
            build_fn: Box::new(|mut parent, v| -> Self {
                parent.direction = Some(v.map(BindResourceDirection::Output));
                parent
            }),
        }
    }
}

pub struct SetBindResourceDescriptorBuilder<P> {
    parent: P,
    build_fn: BuildResultFn<P, BindResourceCreationDescriptor>,
}

impl<P: 'static> SetBindResourceDescriptorBuilder<P> {
    pub fn buffer(self) -> SetBufferDescriptorBuilder<'static, P> {
        SetBufferDescriptorBuilder::new(
            self.parent,
            Box::new(|parent, v| -> P {
                (self.build_fn)(
                    parent,
                    v.map(|b| {
                        BindResourceCreationDescriptor::Buffer(
                            BindResourceCreationStrategy::Static(b),
                        )
                    }),
                )
            }),
        )
    }

    pub fn build_buffer(
        self,
        label: &'static str,
        size: BufferAddress,
        usage: BufferUsages,
        mapped_at_creation: bool,
    ) -> P {
        SetBufferDescriptorBuilder {
            parent: self.parent,
            build_fn: Box::new(|parent, v| -> P {
                (self.build_fn)(
                    parent,
                    v.map(|b| {
                        BindResourceCreationDescriptor::Buffer(
                            BindResourceCreationStrategy::Static(b),
                        )
                    }),
                )
            }),
            label: Some(label),
            size: Some(size),
            usage: Some(usage),
            mapped_at_creation: Some(mapped_at_creation),
        }
        .build()
    }

    pub fn buffer_from_graph_context(
        self,
        buffer_from_graph_context: fn(
            &render_graph::RenderGraphContext,
        ) -> render_resource::BufferDescriptor<'static>,
    ) -> P {
        (self.build_fn)(
            self.parent,
            Ok(BindResourceCreationDescriptor::Buffer(
                BindResourceCreationStrategy::FromGraphContext(buffer_from_graph_context),
            )),
        )
    }
}

pub struct SetBufferDescriptorBuilder<'a, P> {
    parent: P,
    build_fn: BuildResultFn<P, render_resource::BufferDescriptor<'a>>,

    label: Option<&'a str>,
    size: Option<BufferAddress>,
    usage: Option<BufferUsages>,
    mapped_at_creation: Option<bool>,
}

impl<'a, P> SetBufferDescriptorBuilder<'a, P> {
    fn new(parent: P, build_fn: BuildResultFn<P, render_resource::BufferDescriptor<'a>>) -> Self {
        Self {
            parent,
            build_fn,
            label: None,
            size: None,
            usage: None,
            mapped_at_creation: None,
        }
    }

    option_into_setter!(label: &'a str);
    option_setter!(size: BufferAddress);
    option_setter!(usage: BufferUsages);
    option_setter!(mapped_at_creation: bool);

    pub fn build(self) -> P {
        let d = || {
            Ok(render_resource::BufferDescriptor {
                label: Some(self.label.ok_or(BuilderError::ValueNotDefined("label"))?),
                size: self.size.unwrap_or(0),
                usage: self.usage.ok_or(BuilderError::ValueNotDefined("usage"))?,
                mapped_at_creation: self.mapped_at_creation.unwrap_or(false),
            })
        };
        (self.build_fn)(self.parent, d())
    }
}

pub struct SetSlotTypeBuilder<P> {
    parent: P,
    build_fn: BuildFn<P, SlotType>,
}

impl<P> SetSlotTypeBuilder<P> {
    pub fn buffer(self) -> P {
        (self.build_fn)(self.parent, SlotType::Buffer)
    }

    pub fn texture_view(self) -> P {
        (self.build_fn)(self.parent, SlotType::TextureView)
    }

    pub fn sampler(self) -> P {
        (self.build_fn)(self.parent, SlotType::Sampler)
    }

    pub fn entity(self) -> P {
        (self.build_fn)(self.parent, SlotType::Entity)
    }
}

#[derive(Default)]
pub struct SubGraphBuilder {
    name: Option<Cow<'static, str>>,

    graph: RenderGraph,
    providers: HashMap<Entity, ProviderDescriptor>,
    node_edges: Vec<(NodeLabel, NodeLabel)>,
    slot_edges: Vec<(NodeLabel, SlotLabel, NodeLabel, SlotLabel)>,
    graph_inputs: HashMap<Cow<'static, str>, SlotType>,
    outer_edges: Vec<Edge>,
    trigger: Option<SubGraphTrigger>,
}

impl SubGraphBuilder {
    option_setter!(name: Cow<'static, str>);
    option_setter!(trigger: SubGraphTrigger);

    pub fn add_node_provider<T: NodeProvider + 'static>(
        mut self,
        node_name: Cow<'static, str>,
        provider_entity: Entity,
        provider: &T,
    ) -> Self {
        provider.add_node_to_graph(&mut self.graph, node_name.clone());
        self.providers.insert(
            provider_entity,
            ProviderDescriptor {
                name: node_name,
                ty: TypeId::of::<T>(),
                state: ProviderState::default(),
            },
        );
        self
    }

    pub fn add_node<T: render_graph::Node>(
        mut self,
        node_name: impl Into<Cow<'static, str>>,
        node: T,
    ) -> Self {
        self.graph.add_node(node_name, node);
        self
    }

    pub fn add_node_edge(
        mut self,
        output_node: impl Into<NodeLabel>,
        input_node: impl Into<NodeLabel>,
    ) -> Self {
        self.node_edges
            .push((output_node.into(), input_node.into()));
        self
    }

    pub fn add_slot_edge(
        mut self,
        output_node: impl Into<NodeLabel>,
        output_slot: impl Into<SlotLabel>,
        input_node: impl Into<NodeLabel>,
        input_slot: impl Into<SlotLabel>,
    ) -> Self {
        self.slot_edges.push((
            output_node.into(),
            output_slot.into(),
            input_node.into(),
            input_slot.into(),
        ));
        self
    }

    pub fn add_outer_input_node_edge(mut self, output_node: impl Into<NodeLabel>) -> Self {
        self.outer_edges.push(Edge::InputNodeEdge {
            output_node: output_node.into(),
        });
        self
    }

    pub fn add_outer_input_slot_edge(
        mut self,
        output_node: impl Into<NodeLabel>,
        output_slot: impl Into<SlotLabel>,
        input_slot_name: Cow<'static, str>,
        input_slot_type: SlotType,
    ) -> Self {
        self.graph_inputs
            .insert(input_slot_name.clone(), input_slot_type);
        self.outer_edges.push(Edge::InputSlotEdge {
            output_node: output_node.into(),
            output_slot: output_slot.into(),
            input_slot: input_slot_name.into(),
        });
        self
    }

    pub fn add_outer_output_node_edge(mut self, input_node: impl Into<NodeLabel>) -> Self {
        self.outer_edges.push(Edge::OutputNodeEdge {
            input_node: input_node.into(),
        });
        self
    }

    pub fn build(mut self) -> BuildResult<SubGraph> {
        self.graph.set_input(
            self.graph_inputs
                .drain()
                .map(|(name, ty)| SlotInfo {
                    name,
                    slot_type: ty,
                })
                .collect(),
        );

        for (out_node, in_node) in &self.node_edges {
            self.graph.try_add_node_edge(out_node, in_node)?;
        }

        for (out_node, out_slot, in_node, in_slot) in &self.slot_edges {
            self.graph
                .try_add_slot_edge(out_node, out_slot, in_node, in_slot)?;
        }

        Ok(SubGraph {
            name: self.name.ok_or(BuilderError::ValueNotDefined("name"))?,
            providers: self.providers,
            graph: SubGraphDeployState::Queued(self.outer_edges, self.graph),
            trigger: self.trigger.unwrap_or_default(),
        })
    }
}
