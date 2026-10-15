// zfast-crt-inspired single-pass CRT approximation.
//
// This is a fresh WGSL port of zfast-crt's inexpensive pipeline: sharpened
// cubic-ish source reconstruction, brightness-dependent scanline weights, and
// a fine aperture mask. Upstream defaults are retained here (BLURSCALE_X
// 0.30, LOW_LUMA_SCAN 6, HIGH_LUMA_SCAN 8, BRIGHT_BOOST 1.25,
// MASK_DARK 0.25, MASK_FADE 0.8). The two project uniforms replace upstream
// per-preset parameters: scanline_intensity blends the complete scanline
// response, while shader_intensity blends the procedural mask from flat to
// full strength.
//
// Simplifications: this material has no separate output-size/input-size
// uniforms, so the mask uses fragment pixel coordinates and the source size
// comes from texture_size. Upstream's monochrome two-pixel FINEMASK is made a
// tileable three-subpixel RGB mask; this keeps the phosphor cue visible on
// modern displays without an extra pass or texture. Texture sampling relies
// on the screen sampler's edge clamp for cubic samples near the border.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct ScreenParams {
    shader_intensity: f32,
    scanline_intensity: f32,
    texture_size: vec2<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ScreenParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var screen_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var screen_sampler: sampler;

const BLURSCALE_X: f32 = 0.30;
const LOW_LUMA_SCAN: f32 = 6.0;
const HIGH_LUMA_SCAN: f32 = 8.0;
const BRIGHT_BOOST: f32 = 1.25;
const MASK_DARK: f32 = 0.25;
const MASK_FADE: f32 = 0.8;

fn cubicish_sample(uv: vec2<f32>) -> vec3<f32> {
    // Quilez-style cubic remap, sharpened by the original zfast-crt port.
    let source_pos = uv * params.texture_size;
    let center = floor(source_pos) + vec2<f32>(0.5);
    let fractional = source_pos - center;
    var remapped = center + 4.0 * fractional * fractional * fractional;
    remapped.x = mix(remapped.x, source_pos.x, BLURSCALE_X);
    return textureSample(
        screen_texture,
        screen_sampler,
        remapped / params.texture_size
    ).rgb;
}

fn fine_rgb_mask(pixel_x: f32) -> vec3<f32> {
    // One subdued phosphor primary per three physical display columns.
    let phase = fract(floor(pixel_x) / 3.0);
    if phase < 0.3333333 {
        return vec3<f32>(1.0, 1.0 - MASK_DARK, 1.0 - MASK_DARK);
    }
    if phase < 0.6666667 {
        return vec3<f32>(1.0 - MASK_DARK, 1.0, 1.0 - MASK_DARK);
    }
    return vec3<f32>(1.0 - MASK_DARK, 1.0 - MASK_DARK, 1.0);
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let color = cubicish_sample(mesh.uv);
    let y_fraction = fract(mesh.uv.y * params.texture_size.y) - 0.5;
    let y_squared = y_fraction * y_fraction;
    let y_fourth = y_squared * y_squared;

    // zfast-crt uses different beam profiles for dim and bright source pixels.
    let low_luma_weight = BRIGHT_BOOST -
        LOW_LUMA_SCAN * (y_squared - 2.05 * y_fourth);
    let high_luma_weight = 1.0 -
        HIGH_LUMA_SCAN * (y_fourth - 2.8 * y_fourth * y_squared);
    let brightness = clamp(
        dot(color, vec3<f32>(MASK_FADE)),
        0.0,
        1.0,
    );
    let scanline_weight = mix(low_luma_weight, high_luma_weight, brightness);
    let scanline = mix(
        1.0,
        scanline_weight,
        clamp(params.scanline_intensity, 0.0, 1.0),
    );

    let mask = mix(
        vec3<f32>(1.0),
        fine_rgb_mask(mesh.position.x),
        clamp(params.shader_intensity, 0.0, 1.0),
    );
    return vec4<f32>(color * scanline * mask, 1.0);
}
