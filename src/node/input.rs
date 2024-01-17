use bevy::core::Pod;
use bevy::log::{debug, error};
use bevy::prelude::{Component, World};
use bevy_render::render_graph::{NodeRunError, RenderGraphContext, SlotInfo, SlotType, SlotValue};
use bevy_render::render_resource::encase::internal::WriteInto;
use bevy_render::render_resource::{
    Buffer, BufferAddress, BufferUsages, BufferVec, DynamicStorageBuffer, ShaderType, StorageBuffer,
};
use bevy_render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy_render::{render_graph, render_resource};
use std::sync::{Arc, Mutex, MutexGuard};

pub const SLOT_NAME: &str = "out";

pub trait InputBuffer<T> {
    fn size(&self) -> BufferAddress;
    fn write_buffer(&self, device: &RenderDevice, queue: &RenderQueue) -> Option<Buffer>;
}

macro_rules! impl_node_for_input_buffer {
    ($name:ident $(< $( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+ >)?) => {
        impl $(< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? render_graph::Node for $name $(< $( $lt ),+ >)? {
            fn output(&self) -> Vec<SlotInfo> {
                vec![SlotInfo {
                    name: SLOT_NAME.into(),
                    slot_type: SlotType::Buffer,
                }]
            }

            fn run(
                &self,
                graph: &mut RenderGraphContext,
                render_context: &mut RenderContext,
                world: &World,
            ) -> Result<(), NodeRunError> {
                let queue = world.resource::<RenderQueue>();
                if let Some(buffer) = self.write_buffer(render_context.render_device(), queue) {
                    debug!(
                        "Setting value for input buffer output slot `{}` to `{:?}`",
                        SLOT_NAME, buffer
                    );
                    graph.set_output(SLOT_NAME, SlotValue::Buffer(buffer))?;
                } else {
                    error!("Buffer is not created on device!");
                }
                Ok(())
            }
        }
    };
}

#[derive(Clone, Component, Default)]
pub struct DynamicStorageBufferNode<T: render_resource::ShaderType> {
    inner: Arc<Mutex<DynamicStorageBuffer<T>>>,
}

impl<T: render_resource::ShaderType + WriteInto> DynamicStorageBufferNode<T> {
    pub fn push(&self, val: T) -> u32 {
        self.inner.lock().unwrap().push(val)
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().clear()
    }

    pub fn add_usages(&self, usage: BufferUsages) {
        self.inner.lock().unwrap().add_usages(usage);
    }
}

impl<T: render_resource::ShaderType + WriteInto> InputBuffer<T> for DynamicStorageBufferNode<T> {
    fn size(&self) -> BufferAddress {
        self.inner.lock().unwrap().buffer().map_or(0, |b| b.size())
    }

    fn write_buffer(&self, device: &RenderDevice, queue: &RenderQueue) -> Option<Buffer> {
        let mut lock = self.inner.lock().unwrap();
        lock.write_buffer(device, queue);
        lock.buffer().cloned()
    }
}
impl_node_for_input_buffer!(DynamicStorageBufferNode<T: ShaderType + WriteInto + 'static>);

#[derive(Clone, Component, Default)]
pub struct StorageBufferNode<T: render_resource::ShaderType> {
    inner: Arc<Mutex<StorageBuffer<T>>>,
}

impl<T: render_resource::ShaderType + WriteInto + Clone> StorageBufferNode<T> {
    pub fn set(&self, val: T) {
        self.inner.lock().unwrap().set(val);
    }

    pub fn get(&self) -> T {
        self.inner.lock().unwrap().get().clone()
    }

    pub fn lock(&self) -> MutexGuard<StorageBuffer<T>> {
        self.inner.lock().unwrap()
    }

    pub fn add_usages(&self, usage: BufferUsages) {
        self.inner.lock().unwrap().add_usages(usage);
    }
}

impl<T: render_resource::ShaderType + WriteInto> InputBuffer<T> for StorageBufferNode<T> {
    fn size(&self) -> BufferAddress {
        self.inner.lock().unwrap().buffer().map_or(0, |b| b.size())
    }

    fn write_buffer(&self, device: &RenderDevice, queue: &RenderQueue) -> Option<Buffer> {
        let mut lock = self.inner.lock().unwrap();
        lock.write_buffer(device, queue);
        lock.buffer().cloned()
    }
}
impl_node_for_input_buffer!(StorageBufferNode<T: ShaderType + WriteInto + Sync + Send + 'static>);

#[derive(Clone, Component)]
pub struct BufferVecNode<T: Pod> {
    inner: Arc<Mutex<BufferVec<T>>>,
}

impl<T: Pod> BufferVecNode<T> {
    pub fn new(usages: BufferUsages) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BufferVec::new(usages))),
        }
    }

    pub fn push(&self, val: T) -> usize {
        self.inner.lock().unwrap().push(val)
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().clear()
    }
}

impl<T: Pod> Default for BufferVecNode<T> {
    fn default() -> Self {
        Self::new(BufferUsages::COPY_DST | BufferUsages::STORAGE)
    }
}

impl<T: Pod> InputBuffer<T> for BufferVecNode<T> {
    fn size(&self) -> BufferAddress {
        self.inner.lock().unwrap().buffer().map_or(0, |b| b.size())
    }

    fn write_buffer(&self, device: &RenderDevice, queue: &RenderQueue) -> Option<Buffer> {
        let mut lock = self.inner.lock().unwrap();
        lock.write_buffer(device, queue);
        lock.buffer().cloned()
    }
}
impl_node_for_input_buffer!(BufferVecNode<T: Pod + Send + Sync + 'static>);

// pub(crate) enum InputImageState {}
//
// pub struct InputImageNode {}
