// CRT look: barrel-distortion curvature + scanlines + a simple RGB
// phosphor mask, hand-ported from crt-pi (Copyright 2015-2016 davej, GNU
// General Public License v2 or later), a lightweight single-pass CRT
// shader designed for weak GPUs:
// https://github.com/libretro/glsl-shaders/blob/master/crt/shaders/crt-pi.glsl
//
// Simplified from the original: no gamma correction; curvature's
// `screenScale` factor dropped -- crt-pi's TextureSize can be a padded
// power-of-two texture, but this project's own texture is always exactly
// the native picture size, so that factor is always 1.0 here anyway.
// CURVATURE/SCANLINE_WEIGHT/SCANLINE_GAP_BRIGHTNESS/BLOOM_FACTOR/
// MASK_BRIGHTNESS below are crt-pi's own default parameter values, baked
// in rather than exposed as new CLI flags (this project's own
// --shader-intensity/--scanline-intensity are the only two knobs).
//
// shader_intensity (0..1) scales curvature amount and mask strength
// together (0 = flat, unmasked -- the passthrough look); scanline_intensity
// (0..1) scales scanline darkening/bloom, same meaning as scanlines.wgsl.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct ScreenParams {
    shader_intensity: f32,
    scanline_intensity: f32,
    texture_size: vec2<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ScreenParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var screen_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var screen_sampler: sampler;

const CURVATURE: vec2<f32> = vec2<f32>(0.10, 0.15);
const SCANLINE_WEIGHT: f32 = 6.0;
const SCANLINE_GAP_BRIGHTNESS: f32 = 0.12;
const BLOOM_FACTOR: f32 = 1.5;
const MASK_BRIGHTNESS: f32 = 0.70;

fn distort(uv_in: vec2<f32>, curvature: vec2<f32>) -> vec2<f32> {
    var uv = uv_in - vec2<f32>(0.5);
    let barrel_scale = 1.0 - (0.23 * curvature);
    let rsq = uv.x * uv.x + uv.y * uv.y;
    uv = uv + uv * (curvature * rsq);
    uv = uv * barrel_scale;
    return uv + vec2<f32>(0.5);
}

fn scan_line_weight(dy: f32) -> f32 {
    return max(1.0 - dy * dy * SCANLINE_WEIGHT, SCANLINE_GAP_BRIGHTNESS);
}

// crt-pi's own `CalcScanLine` (its `MULTISAMPLE` path): averages the
// weight at 3 sample offsets spanning one output pixel's worth of texel-Y
// coverage, rather than a single point sample -- without it, scanline
// darkening aliases into a moire pattern once the display scale isn't a
// clean multiple of the source resolution (confirmed live: an earlier,
// single-sample version of this shader showed clear concentric moire
// rings once the window was resized to a non-integer-friendly size).
// `fwidth` (a fragment-shader screen-space derivative) stands in for
// crt-pi's own precalculated `filterWidth` uniform, which this shader
// doesn't have.
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
    let curvature = CURVATURE * params.shader_intensity;
    let uv = distort(mesh.uv, curvature);

    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    var color = textureSample(screen_texture, screen_sampler, uv).rgb;

    let texel_y = uv.y * params.texture_size.y;
    let scan_weight = mix(1.0, scan_line(texel_y) * BLOOM_FACTOR, params.scanline_intensity);
    color = color * scan_weight;

    let mask_brightness = mix(1.0, MASK_BRIGHTNESS, params.shader_intensity);
    let which_mask = fract(mesh.position.x * 0.5);
    var mask: vec3<f32>;
    if which_mask < 0.5 {
        mask = vec3<f32>(mask_brightness, 1.0, mask_brightness);
    } else {
        mask = vec3<f32>(1.0, mask_brightness, 1.0);
    }
    color = color * mask;

    return vec4<f32>(color, 1.0);
}
