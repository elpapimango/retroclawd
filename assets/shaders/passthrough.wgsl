// Plain nearest-neighbor passthrough -- the default when no --shader is
// given, or --shader passthrough is given explicitly. Reproduces exactly
// what bevy's built-in Sprite shader already did before this material
// existed: one texture sample, no processing. No external source ported --
// this is the "do nothing" case.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct ScreenParams {
    shader_intensity: f32,
    scanline_intensity: f32,
    texture_size: vec2<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ScreenParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var screen_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var screen_sampler: sampler;

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(screen_texture, screen_sampler, mesh.uv);
}
