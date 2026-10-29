// Pass 0 of crt-bloom: the bright parts of the emulated picture. The output is half the picture's size, so each output pixel
// covers a block of 2x2 source texels; the four are averaged (the source image is sampled with nearest filtering) and what is
// above the threshold is kept. Same bindings as every pass shader (see `gui::video`).

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

const THRESHOLD: f32 = 0.55;

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let texel = 1.0 / params.texture_size;
    var sum = vec3<f32>(0.0);
    sum += textureSample(input_texture, input_sampler, mesh.uv + vec2<f32>(-0.5, -0.5) * texel).rgb;
    sum += textureSample(input_texture, input_sampler, mesh.uv + vec2<f32>(0.5, -0.5) * texel).rgb;
    sum += textureSample(input_texture, input_sampler, mesh.uv + vec2<f32>(-0.5, 0.5) * texel).rgb;
    sum += textureSample(input_texture, input_sampler, mesh.uv + vec2<f32>(0.5, 0.5) * texel).rgb;
    let color = sum * 0.25;
    let luma = dot(color, vec3<f32>(0.299, 0.587, 0.114));
    let keep = max(luma - THRESHOLD, 0.0) / (1.0 - THRESHOLD);
    return vec4<f32>(color * keep, 1.0);
}
