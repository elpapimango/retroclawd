// crt-potato-inspired single-pass phosphor mask.
//
// The upstream standard potato shader samples the source once and multiplies
// it by a repeating external mask texture. This port keeps that behavior;
// previous version added ultra-potato luminance shaping, which weakened the
// standard mask enough to become nearly invisible on bright game screens.
//
// Upstream potato presets bind a repeating external 2x5 PNG (thin or thick).
// Material2d has no mask texture binding here, so a tileable procedural RGB
// aperture is used instead. Its vertical beam profile approximates the PNG's
// center-bright falloff; it is not a pixel-for-pixel reconstruction. The
// procedural mask is intentionally stronger than the old approximation to
// compensate for the missing texture gradients. `scanline_intensity` blends
// the beam profile, while `shader_intensity` blends the mask from flat to full.
// No curvature or extra pass is added.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct ScreenParams {
    shader_intensity: f32,
    scanline_intensity: f32,
    texture_size: vec2<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ScreenParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var screen_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var screen_sampler: sampler;

const PI2: f32 = 6.2831853;
const MASK_DARK: f32 = 0.36;
const MASK_CENTER_BRIGHTNESS: f32 = 0.28;

fn aperture_rgb(pixel_x: f32) -> vec3<f32> {
    // RGB phosphor triplet; phase repeats every three output pixels.
    let phase = fract(floor(pixel_x) / 3.0);
    if phase < 0.3333333 {
        return vec3<f32>(1.0, 1.0 - MASK_DARK, 1.0 - MASK_DARK * 0.75);
    }
    if phase < 0.6666667 {
        return vec3<f32>(1.0 - MASK_DARK * 0.75, 1.0, 1.0 - MASK_DARK);
    }
    return vec3<f32>(1.0 - MASK_DARK, 1.0 - MASK_DARK * 0.75, 1.0);
}

fn beam_profile(pixel_y: f32) -> f32 {
    // Periodic center-bright profile stands in for potato's 2x5 mask PNG.
    let phase = fract(floor(pixel_y) / 5.0);
    return 1.0 - MASK_CENTER_BRIGHTNESS *
        (0.5 + 0.5 * cos(PI2 * phase));
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let source = textureSample(screen_texture, screen_sampler, mesh.uv).rgb;
    let strength = clamp(params.shader_intensity, 0.0, 1.0);
    let scanline_strength = clamp(params.scanline_intensity, 0.0, 1.0);

    let mask_rgb = aperture_rgb(mesh.position.x);
    let beam = mix(1.0, beam_profile(mesh.position.y), scanline_strength);
    let procedural_mask = mask_rgb * beam;

    // Standard crt-potato multiplies source color by the mask directly.
    // Keep `shader_intensity` as the only mask-strength control.
    let applied_mask = mix(vec3<f32>(1.0), procedural_mask, strength);

    return vec4<f32>(source * applied_mask, 1.0);
}
