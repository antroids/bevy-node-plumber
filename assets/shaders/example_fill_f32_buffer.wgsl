@group(0) @binding(0)
var<storage, read_write> buffer: array<f32>;

@compute @workgroup_size(1, 1, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = (global_id.x + 1u) * (global_id.y + 1u) * (global_id.z + 1u);
    buffer[index] = f32(index);
}