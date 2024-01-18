use crate::resource::{BindResourceCreationInfo, BindResourceDirection};
use bevy::prelude::*;
use bevy_render::render_graph;
use bevy_render::render_graph::{NodeRunError, RenderGraph, RenderGraphContext, SlotInfo};
use bevy_render::renderer::RenderContext;
use std::borrow::Cow;

pub mod compute;
pub mod input;
pub mod output;

#[derive(Default, Debug)]
struct DummyNode {
    input: Vec<SlotInfo>,
    output: Vec<SlotInfo>,
}

impl DummyNode {
    pub fn from_bind_resource_info(info: &[BindResourceCreationInfo]) -> Self {
        let mut slf = Self::default();
        for i in info {
            match &i.direction {
                BindResourceDirection::Input(input) => {
                    slf.input.push(SlotInfo::new(i.name.clone(), *input));
                }
                BindResourceDirection::Output(output) => {
                    slf.output
                        .push(SlotInfo::new(i.name.clone(), output.to_slot_type()));
                }
                BindResourceDirection::InputOutput(input_output) => {
                    let slot_info = SlotInfo::new(i.name.clone(), *input_output);
                    slf.output.push(slot_info.clone());
                    slf.input.push(slot_info);
                }
            }
        }
        slf
    }
}

impl render_graph::Node for DummyNode {
    fn input(&self) -> Vec<SlotInfo> {
        self.input.clone()
    }

    fn output(&self) -> Vec<SlotInfo> {
        self.output.clone()
    }

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        _render_context: &mut RenderContext,
        _world: &World,
    ) -> Result<(), NodeRunError> {
        error!(
            "Dummy node should not be ran! \
        It was not replaced be actual node implementation for some reason."
        );
        Ok(())
    }
}

pub(crate) fn add_or_replace_graph_node<T: render_graph::Node>(
    graph: &mut RenderGraph,
    name: Cow<'static, str>,
    node_impl: T,
) {
    if let Ok(node) = graph.get_node_state_mut(render_graph::NodeLabel::Name(name.clone())) {
        node.node = Box::new(node_impl);
        node.type_name = std::any::type_name::<T>();
    } else {
        graph.add_node(name, node_impl);
    }
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

impl DispatchWorkgroupsStrategy {
    pub(crate) fn workgroups_to_dispatch(
        &self,
        graph: &render_graph::RenderGraphContext,
    ) -> (u32, u32, u32) {
        match self {
            DispatchWorkgroupsStrategy::Static(x, y, z) => (*x, *y, *z),
            DispatchWorkgroupsStrategy::FromGraphContext(from_graph) => from_graph(graph),
        }
    }
}
