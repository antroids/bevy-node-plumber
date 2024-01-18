use bevy::prelude::*;
use bevy_node_plumber::prelude::*;
use bevy_render::render_graph::RenderGraph;
use bevy_render::render_resource::BufferUsages;
use std::mem::size_of;
use std::sync::Arc;

fn main() {
    let mut app = App::new();
    #[cfg(debug_assertions)]
    app.add_plugins(DefaultPlugins.set(bevy::log::LogPlugin {
        level: bevy::log::Level::DEBUG,
        filter: "debug,wgpu_core=warn,wgpu_hal=warn,mygame=debug".into(),
    }));
    app.add_plugins(NodePlumberPlugin)
        .add_systems(Startup, test_startup)
        .add_systems(Update, print_output_buffer);

    #[cfg(debug_assertions)]
    {
        use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
        use bevy::diagnostic::LogDiagnosticsPlugin;
        app.add_plugins(LogDiagnosticsPlugin::default())
            .add_plugins(FrameTimeDiagnosticsPlugin::default());
    }

    app.run();
}

fn test_startup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let fill_buffer_node = builder::ComputeNodeBuilder::default()
        .shader(asset_server.load("shaders/example_fill_f32_buffer.wgsl"))
        .entry_point("main")
        .dispatch_workgroups_strategy(DispatchWorkgroupsStrategy::FromGraphContext(|graph| {
            let x = graph
                .get_input_buffer("buffer")
                .map_or(1, |b| b.size() / size_of::<f32>() as u64);
            (x as u32, 1, 1)
        }));
    let fill_buffer_node = fill_buffer_node
        .bind_resource()
        .name("buffer")
        .binding(0)
        .input_output()
        .buffer()
        .add();
    let fill_buffer_node = fill_buffer_node.build().unwrap();
    let fill_buffer_entity = commands.spawn(fill_buffer_node.clone()).id();

    let input_buffer = input::StorageBufferNode::default();
    let output_buffer = output::OutputBuffer::default();

    input_buffer.set(vec![0.0; 65535]);
    input_buffer.add_usages(BufferUsages::COPY_SRC);
    let trigger = graph::SubGraphTrigger::Manual(Arc::new(true.into()));

    let sub_graph = builder::SubGraphBuilder::default()
        .name("test_compute_sub_graph".into())
        .add_node("input_buffer", input_buffer.clone())
        .add_node("output_buffer", output_buffer.clone())
        .add_node_provider(
            "fill_buffer_node".into(),
            fill_buffer_entity,
            &fill_buffer_node,
        )
        .add_node_edge(RenderGraph::INPUT_NODE_NAME, "input_buffer")
        .add_slot_edge(
            "input_buffer",
            input::SLOT_NAME,
            "fill_buffer_node",
            "buffer",
        )
        .add_slot_edge(
            "fill_buffer_node",
            "buffer",
            "output_buffer",
            output::SLOT_NAME,
        )
        .trigger(trigger.clone())
        .build()
        .unwrap();

    commands.spawn((sub_graph, trigger, output_buffer, input_buffer));
}

fn print_output_buffer(query: Query<&output::OutputBuffer>) {
    for out in query.iter() {
        match out.take_buffer_as::<Vec<f32>>() {
            Ok(floats) => {
                println!("Output buffer content({}): {:?}", floats.len(), floats);
            }
            Err(err) => {
                println!("Cannot take output buffer: {:?}", err);
            }
        }
    }
}
