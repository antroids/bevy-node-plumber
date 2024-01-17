use bevy::prelude::*;
use bevy_node_plumber::node::compute::ComputeNode;
use bevy_node_plumber::prelude::*;
use bevy_render::main_graph::node::CAMERA_DRIVER;
use bevy_render::render_graph::RenderGraph;
use bevy_render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

fn main() {
    let mut app = App::new();
    #[cfg(debug_assertions)]
    app.add_plugins(DefaultPlugins.set(bevy::log::LogPlugin {
        level: bevy::log::Level::DEBUG,
        filter: "debug,wgpu_core=warn,wgpu_hal=warn,mygame=debug".into(),
    }));
    app.add_plugins(NodePlumberPlugin)
        .add_systems(Startup, test_startup)
        .add_systems(PreUpdate, update_node_on_shader_changed);

    #[cfg(debug_assertions)]
    {
        use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
        use bevy::diagnostic::LogDiagnosticsPlugin;
        app.add_plugins(LogDiagnosticsPlugin::default())
            .add_plugins(FrameTimeDiagnosticsPlugin::default());
    }

    app.run();
}

fn test_startup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
) {
    let mut image = Image::new_fill(
        Extent3d {
            width: 640,
            height: 480,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[255, 0, 0, 255],
        TextureFormat::Rgba8Unorm,
    );
    image.texture_descriptor.usage =
        TextureUsages::COPY_DST | TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING;
    let asset_image = images.add(image);
    let input_texture_node = input::InputTextureNode::from_image(asset_image.clone());

    commands.spawn(SpriteBundle {
        sprite: Sprite {
            custom_size: Some(Vec2::new(640f32, 480f32)),
            ..default()
        },
        texture: asset_image,
        ..default()
    });
    commands.spawn(Camera2dBundle::default());

    let fill_texture_view_node = builder::ComputeNodeBuilder::default()
        .shader(asset_server.load("shaders/example_fill_texture_view.wgsl"))
        .entry_point("main")
        .dispatch_workgroups_strategy(compute::DispatchWorkgroupsStrategy::Static(640, 480, 1));
    let fill_texture_view_node = fill_texture_view_node
        .bind_resource()
        .name("texture")
        .binding(0)
        .input()
        .texture_view()
        .add();
    let fill_texture_view_node = fill_texture_view_node.build().unwrap();
    let fill_texture_view_entity = commands.spawn(fill_texture_view_node.clone()).id();

    let trigger = graph::SubGraphTrigger::Always;

    let sub_graph = builder::SubGraphBuilder::default()
        .name("test_compute_sub_graph".into())
        .add_node("input_texture", input_texture_node)
        .add_node_provider(
            "fill_texture_view_node".into(),
            fill_texture_view_entity,
            &fill_texture_view_node,
        )
        .add_node_edge(RenderGraph::INPUT_NODE_NAME, "input_texture")
        .add_slot_edge(
            "input_texture",
            input::SLOT_NAME,
            "fill_texture_view_node",
            "texture",
        )
        .trigger(trigger.clone())
        .add_outer_output_node_edge(CAMERA_DRIVER)
        .build()
        .unwrap();

    commands.spawn((sub_graph, trigger));
}

fn update_node_on_shader_changed(
    mut events: EventReader<AssetEvent<Shader>>,
    mut query: Query<&mut ComputeNode>,
) {
    let ids: Vec<AssetId<Shader>> = events
        .read()
        .filter_map(|event| {
            if let AssetEvent::Modified { id } = event {
                Some(*id)
            } else {
                None
            }
        })
        .collect();

    for mut compute_node in query.iter_mut() {
        if ids.contains(&compute_node.pipeline_descriptor.shader.id()) {
            compute_node.set_changed();
        }
    }
}
