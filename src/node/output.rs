use bevy::log::debug;
use bevy::prelude::*;
use bevy::utils::thiserror::Error;
use bevy_render::render_graph;
use bevy_render::render_graph::{NodeRunError, RenderGraphContext, SlotInfo, SlotType};
use bevy_render::render_resource::encase::internal::{CreateFrom, Reader};
use bevy_render::render_resource::{
    encase, Buffer, BufferAddress, BufferDescriptor, BufferUsages, MapMode, ShaderType,
};
use bevy_render::renderer::{RenderContext, RenderDevice};
use std::ops::{Deref, DerefMut, RangeFull};
use std::sync::{Arc, Mutex};

pub const SLOT_NAME: &str = "in";

pub struct OutputBufferPlugin;

impl Plugin for OutputBufferPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, OutputBuffer::map_output_buffers);
    }
}

#[derive(Error, Debug)]
pub enum OutputError {
    #[error("Buffer already consumed, not mapped or not created")]
    MappedBufferNotFound,
    #[error("The state is locked and cannot be used right now")]
    CannotLock,
    #[error("An async error occurred while trying to map the buffer")]
    AsyncMapError,
    #[error("Buffer read-write error: {0}")]
    BufferReadWriteError(#[from] encase::internal::Error),
}

#[derive(Default, Debug)]
enum OutputBufferState {
    #[default]
    NotCreated,
    ReadyToMap(Buffer),
    WaitingForMap(Buffer),
    Mapped(Buffer),
    MappingError,
}

#[derive(Component, Clone, Debug, Default)]
pub struct OutputBuffer {
    state: Arc<Mutex<OutputBufferState>>,
}

impl OutputBuffer {
    pub fn take_buffer(&self) -> Result<Buffer, OutputError> {
        if let Ok(state) = self.state.try_lock().as_deref_mut() {
            if matches!(state, OutputBufferState::Mapped(_)) {
                let OutputBufferState::Mapped(buffer) =
                    std::mem::replace(state, OutputBufferState::NotCreated)
                else {
                    unreachable!()
                };
                Ok(buffer)
            } else {
                Err(OutputError::MappedBufferNotFound)
            }
        } else {
            Err(OutputError::CannotLock)
        }
    }

    pub fn take_buffer_as<T: ShaderType + CreateFrom>(&self) -> Result<T, OutputError> {
        let buffer = self.take_buffer()?;
        let mapped_range = buffer.slice(RangeFull).get_mapped_range();
        let mut reader = Reader::new::<T>(mapped_range.deref(), 0)?;
        Ok(T::create_from(&mut reader))
    }

    pub fn buffer_ready(&self) -> bool {
        self.state
            .try_lock()
            .is_ok_and(|lock| matches!(lock.deref(), OutputBufferState::Mapped(_)))
    }
}

impl render_graph::Node for OutputBuffer {
    fn input(&self) -> Vec<SlotInfo> {
        vec![SlotInfo::new(SLOT_NAME, SlotType::Buffer)]
    }

    fn run(
        &self,
        graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        _world: &World,
    ) -> Result<(), NodeRunError> {
        let input = graph.get_input_buffer(SLOT_NAME)?;
        let size = input.size();
        let mut state = self
            .state
            .lock()
            .expect("Output buffer state mutex is poisoned");

        debug!(
            "Buffer state before OutputBuffer node processed: {:?}",
            &state
        );
        let buffer = match state.deref() {
            OutputBufferState::NotCreated => None,
            OutputBufferState::Mapped(buffer) => {
                debug!(
                    "Mapped buffer `{:?}` can be reused after unmapping",
                    &buffer
                );
                buffer.unmap();
                Some(buffer)
            }
            OutputBufferState::MappingError => None,
            OutputBufferState::ReadyToMap(buffer) => Some(buffer),
            OutputBufferState::WaitingForMap(_) => None,
        };

        let buffer = if buffer.as_ref().is_some_and(|b| b.size() == size) {
            buffer.expect("Buffer must be checked for Some").clone()
        } else {
            OutputBuffer::create_output_buffer(render_context.render_device(), size)
        };

        debug!(
            "Copy buffer to buffer command added to the queue from `{:?}` to `{:?}`",
            &input, &buffer
        );
        render_context
            .command_encoder()
            .copy_buffer_to_buffer(input, 0, &buffer, 0, size);
        *state = OutputBufferState::ReadyToMap(buffer);
        Ok(())
    }
}

impl OutputBuffer {
    fn create_output_buffer(render_device: &RenderDevice, size: BufferAddress) -> Buffer {
        render_device.create_buffer(&BufferDescriptor {
            label: "output_buffer".into(),
            size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    }

    pub(crate) fn map_output_buffers(query: Query<&Self>, render_device: Res<RenderDevice>) {
        for output in query.iter() {
            let mut state_lock = output
                .state
                .lock()
                .expect("Output buffer state mutex is poisoned");
            let OutputBufferState::ReadyToMap(buffer) = state_lock.deref() else {
                continue;
            };
            let buffer = buffer.clone();
            *state_lock.deref_mut() = OutputBufferState::WaitingForMap(buffer.clone());
            render_device.map_buffer(&buffer.slice(RangeFull), MapMode::Read, {
                let state = output.state.clone();
                debug!("Waiting for map of the buffer `{:?}`", &buffer);
                move |result| {
                    let mut state = state.lock().expect("Output buffer state mutex is poisoned");
                    let OutputBufferState::WaitingForMap(buffer) =
                        std::mem::replace(state.deref_mut(), OutputBufferState::NotCreated)
                    else {
                        return;
                    };
                    debug!("Buffer `{:?}` mapped with result `{:?}`", &buffer, &result);
                    let new_state = result.map_or(OutputBufferState::MappingError, |_| {
                        OutputBufferState::Mapped(buffer)
                    });
                    let _ = std::mem::replace(state.deref_mut(), new_state);
                }
            });
        }
    }
}
