// Single-pass CRT geometry approximation for libretro crt-geom.slang.
//
// This is a fresh WGSL port of the useful progressive-display path, not a
// copy of the upstream GLSL. It keeps the upstream defaults for target gamma
// (2.4), monitor gamma (2.2), geometry distance/radius (1.5/2.0), corner
// size/smoothing, scanline weight, and classic aperture layout (mask type 1).
//
// Deliberate simplifications: progressive rendering only; no tilt, interlace,
// or vertical scanlines; no full 25-layout mask selector; and no upstream
// alpha-based aperture brightness normalization. The mask below darkens its
// inactive phosphors directly, which keeps this shader single-pass and avoids
// a hidden luminance boost. The spherical inverse geometry is represented by
// a compact aspect-neutral barrel approximation because this material has no
// tilt or per-pass geometry uniforms.
//
// shader_intensity (0..1) scales curvature, corner falloff, and mask strength.
// scanline_intensity (0..1) blends the beam-weighted result with the unweighted
// result. `texture_size` is the native picture size in texels.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct ScreenParams {
    shader_intensity: f32,
    scanline_intensity: f32,
    texture_size: vec2<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ScreenParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var screen_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var screen_sampler: sampler;

const PI: f32 = 3.141592653589793;

// crt-geom.slang defaults, baked because ScreenParams is intentionally small.
const CRT_GAMMA: f32 = 2.4;
const MONITOR_GAMMA: f32 = 2.2;
const GEOMETRY_DISTANCE: f32 = 1.5;
const GEOMETRY_RADIUS: f32 = 2.0;
const CORNER_SIZE: f32 = 0.03;
const CORNER_SMOOTHNESS: f32 = 1000.0;
const MASK_STRENGTH: f32 = 0.30;
const SCANLINE_WEIGHT: f32 = 0.30;
const SCANLINE_LUMINANCE: f32 = 0.0;

fn linearize(color: vec3<f32>) -> vec3<f32> {
    let strength = clamp(params.shader_intensity, 0.0, 1.0);
    let gamma = mix(1.0, CRT_GAMMA, strength);
    return pow(max(color, vec3<f32>(0.0)), vec3<f32>(gamma));
}

fn encode_monitor(color: vec3<f32>) -> vec3<f32> {
    let strength = clamp(params.shader_intensity, 0.0, 1.0);
    let gamma = mix(1.0, 1.0 / MONITOR_GAMMA, strength);
    return pow(max(color, vec3<f32>(0.0)), vec3<f32>(gamma));
}

// Compact radial form of crt-geom's default curved surface. Radius and
// viewing distance remain explicit so changing this baked preset is obvious.
fn curve_uv(uv: vec2<f32>, strength: f32) -> vec2<f32> {
    let p = (uv - vec2<f32>(0.5)) * 2.0;
    let curvature = 0.08 * (GEOMETRY_DISTANCE / GEOMETRY_RADIUS) * strength;
    let radius_squared = dot(p, p);
    let barrel_scale = 1.0 - 0.23 * curvature;
    let curved = p * (1.0 + curvature * radius_squared) * barrel_scale;
    return curved * 0.5 + vec2<f32>(0.5);
}

// Upstream's corner equation is retained in its useful rounded-corner form.
// It fades only the corner region: straight edge centers stay visible.
fn corner_falloff(uv: vec2<f32>) -> f32 {
    let edge_distance = min(uv, vec2<f32>(1.0) - uv);
    let missing_corner = max(vec2<f32>(CORNER_SIZE) - edge_distance, vec2<f32>(0.0));
    let distance_from_round_corner = length(missing_corner);
    return clamp(
        (CORNER_SIZE - distance_from_round_corner) * CORNER_SMOOTHNESS,
        0.0,
        1.0,
    );
}

fn lanczos2(x: f32) -> f32 {
    let distance = abs(x);
    if distance >= 2.0 {
        return 0.0;
    }
    if distance < 0.0001 {
        return 1.0;
    }
    let angle = PI * x;
    return 2.0 * sin(angle) * sin(angle * 0.5) / (angle * angle);
}

fn sample_linear(uv: vec2<f32>) -> vec3<f32> {
    // Clamp tap coordinates because the four-tap kernel reaches outside the
    // first/last texel. This makes edge behavior independent of sampler mode.
    let texel = 0.5 / params.texture_size;
    let safe_uv = clamp(uv, texel, vec2<f32>(1.0) - texel);
    return linearize(textureSample(screen_texture, screen_sampler, safe_uv).rgb);
}

// Horizontal Lanczos-2 reconstruction: four taps at -1, 0, +1, +2 texels.
// Vertical reconstruction is intentionally omitted; vertical beam weighting
// below supplies the CRT scanline response instead.
fn lanczos_row(uv: vec2<f32>, row_y: f32) -> vec3<f32> {
    let texel_x = uv.x * params.texture_size.x - 0.5;
    let base_x = floor(texel_x);
    let fraction_x = texel_x - base_x;
    let one_x = 1.0 / params.texture_size.x;
    let center_x = (base_x + 0.5) * one_x;

    let weights = vec4<f32>(
        lanczos2(-1.0 - fraction_x),
        lanczos2(-fraction_x),
        lanczos2(1.0 - fraction_x),
        lanczos2(2.0 - fraction_x),
    );
    let weight_sum = max(dot(weights, vec4<f32>(1.0)), 0.0001);

    let c0 = sample_linear(vec2<f32>(center_x - one_x, row_y));
    let c1 = sample_linear(vec2<f32>(center_x, row_y));
    let c2 = sample_linear(vec2<f32>(center_x + one_x, row_y));
    let c3 = sample_linear(vec2<f32>(center_x + 2.0 * one_x, row_y));
    return clamp(
        (c0 * weights.x + c1 * weights.y + c2 * weights.z + c3 * weights.w)
            / weight_sum,
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
}

// Color-dependent beam profile follows crt-geom's non-Gaussian default. Wide
// bright beams and narrow dark beams retain separate RGB response.
fn beam_weight(distance: f32, color: vec3<f32>) -> vec3<f32> {
    let width = vec3<f32>(2.0) + 2.0 * pow(
        clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)),
        vec3<f32>(4.0),
    );
    let scaled = vec3<f32>(distance / SCANLINE_WEIGHT)
        / sqrt(vec3<f32>(0.5) * width);
    let profile = exp(-pow(scaled, width));
    return vec3<f32>(SCANLINE_LUMINANCE + 1.4) * profile
        / (vec3<f32>(0.6) + 0.2 * width);
}

fn beam_color(
    fraction_y: f32,
    current_row: vec3<f32>,
    next_row: vec3<f32>,
    filter_width: f32,
) -> vec3<f32> {
    var current_weight = beam_weight(fraction_y, current_row);
    var next_weight = beam_weight(1.0 - fraction_y, next_row);

    // Three evaluations reduce curvature/scanline moire, matching upstream's
    // OVERSAMPLE path without adding another render pass.
    let sample_a = fraction_y + filter_width;
    let sample_b = fraction_y - filter_width;
    current_weight = (
        current_weight
            + beam_weight(abs(sample_a), current_row)
            + beam_weight(abs(sample_b), current_row)
    ) / 3.0;
    next_weight = (
        next_weight
            + beam_weight(abs(1.0 - sample_a), next_row)
            + beam_weight(abs(1.0 - sample_b), next_row)
    ) / 3.0;

    return current_row * current_weight + next_row * next_weight;
}

// Fixed mask type 1: classic alternating magenta/green aperture. Unlike
// upstream, no alpha normalization or brightness boost is applied.
fn aperture_mask(uv: vec2<f32>, strength: f32) -> vec3<f32> {
    let pixel_x = floor(uv.x * params.texture_size.x);
    let stripe = floor(fract(pixel_x * 0.5) * 2.0);
    let inactive = 1.0 - strength * MASK_STRENGTH;
    let pattern = mix(
        vec3<f32>(1.0, inactive, 1.0),
        vec3<f32>(inactive, 1.0, inactive),
        stripe,
    );
    return pattern;
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let geometry_strength = clamp(params.shader_intensity, 0.0, 1.0);
    let scanline_strength = clamp(params.scanline_intensity, 0.0, 1.0);
    let curved_uv = curve_uv(mesh.uv, geometry_strength);

    if curved_uv.x < 0.0 || curved_uv.x > 1.0
        || curved_uv.y < 0.0 || curved_uv.y > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    let texel_y = curved_uv.y * params.texture_size.y - 0.5;
    let base_y = floor(texel_y);
    let fraction_y = texel_y - base_y;
    let one_y = 1.0 / params.texture_size.y;
    let current_y = (base_y + 0.5) * one_y;
    let next_y = (base_y + 1.5) * one_y;
    let current_row = lanczos_row(curved_uv, current_y);
    let next_row = lanczos_row(curved_uv, next_y);
    let filter_width = min(fwidth(texel_y) / 3.0, 0.49);
    let weighted = beam_color(fraction_y, current_row, next_row, filter_width);
    var color = mix(current_row, weighted, scanline_strength);

    let corner = corner_falloff(curved_uv);
    color = color * mix(1.0, corner, geometry_strength);
    color = color * aperture_mask(mesh.uv, geometry_strength);
    color = encode_monitor(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)));

    return vec4<f32>(color, 1.0);
}
