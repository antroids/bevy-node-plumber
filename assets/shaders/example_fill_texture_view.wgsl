@group(0) @binding(0) var texture: texture_storage_2d<rgba8unorm, read_write>;

@compute @workgroup_size(1, 1, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let location = vec2<i32>(i32(global_id.x), i32(global_id.y));
    let color = vec4<f32>(1.0, 1.0, 0.0, 1.0);
    textureStore(texture, location, color);
}