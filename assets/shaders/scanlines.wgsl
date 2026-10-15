// Scanline darkening only, no curvature/mask. The scanline-weight formula
// is hand-ported from crt-pi's CalcScanLineWeight/CalcScanLine (Copyright
// 2015-2016 davej, GNU General Public License v2 or later):
// https://github.com/libretro/glsl-shaders/blob/master/crt/shaders/crt-pi.glsl
// (SCANLINE_WEIGHT/SCANLINE_GAP_BRIGHTNESS below are that shader's own
// default parameter values, baked in rather than exposed as new CLI
// flags). scanline_intensity (0..1) blends between no darkening (0) and
// the full effect (1); shader_intensity is unused here -- nothing else in
// this shader to scale.
//
// Also ports crt-pi's `MULTISAMPLE` 3-tap averaging (see `scan_line`
// below) -- without it, this aliases into a visible moire pattern
// whenever the display scale isn't a clean multiple of the source
// resolution (confirmed live against an earlier, single-sample version).

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct ScreenParams {
    shader_intensity: f32,
    scanline_intensity: f32,
    texture_size: vec2<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ScreenParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var screen_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var screen_sampler: sampler;

const SCANLINE_WEIGHT: f32 = 6.0;
const SCANLINE_GAP_BRIGHTNESS: f32 = 0.12;

fn scan_line_weight(dy: f32) -> f32 {
    return max(1.0 - dy * dy * SCANLINE_WEIGHT, SCANLINE_GAP_BRIGHTNESS);
}

fn scan_line(texel_y: f32) -> f32 {
    let filter_width = fwidth(texel_y) / 3.0;
    let dy = fract(texel_y) - 0.5;
    var weight = scan_line_weight(dy);
    weight += scan_line_weight(dy - filter_width);
    weight += scan_line_weight(dy + filter_width);
    return weight * 0.3333333;
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(screen_texture, screen_sampler, mesh.uv);

    let texel_y = mesh.uv.y * params.texture_size.y;
    let weight = mix(1.0, scan_line(texel_y), params.scanline_intensity);

    return vec4<f32>(color.rgb * weight, color.a);
}
