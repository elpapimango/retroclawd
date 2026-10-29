// A vertical blur of nine taps (a gaussian), used by crt-bloom. The input is the pass before, sampled with linear filtering, and
// the taps are one input texel apart. Same bindings as every pass shader (see `gui::video`).

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct PassParams {
    shader_intensity: f32,
    scanline_intensity: f32,
    texture_size: vec2<f32>,
    original_size: vec2<f32>,
    output_size: vec2<f32>,
    frame_count: f32,
    pass_index: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PassParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var input_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var input_sampler: sampler;

const WEIGHTS: array<f32, 5> = array<f32, 5>(0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216);

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let step = vec2<f32>(0.0, 1.0) / params.texture_size;
    var color = textureSample(input_texture, input_sampler, mesh.uv).rgb * WEIGHTS[0];
    for (var i = 1; i < 5; i = i + 1) {
        let offset = step * f32(i) * 1.5;
        color += textureSample(input_texture, input_sampler, mesh.uv + offset).rgb * WEIGHTS[i];
        color += textureSample(input_texture, input_sampler, mesh.uv - offset).rgb * WEIGHTS[i];
    }
    return vec4<f32>(color, 1.0);
}
