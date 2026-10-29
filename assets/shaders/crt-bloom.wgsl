// The last pass of crt-bloom: the emulated picture (`original`, sampled with nearest filtering) with barrel curvature, scanlines
// and an RGB phosphor mask, plus the glow `input` (the blurred bright parts from the passes before). Drawn on screen.
//
// shader_intensity (0..1) scales curvature, mask strength and glow; scanline_intensity (0..1) the scanline darkening. The
// scanline and mask arithmetic follows crt.wgsl (hand-ported from crt-pi, GPL v2 or later, see CREDITS.md).

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
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var bloom_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var bloom_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var original_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var original_sampler: sampler;

const CURVATURE: vec2<f32> = vec2<f32>(0.10, 0.15);
const SCANLINE_WEIGHT: f32 = 6.0;
const SCANLINE_GAP_BRIGHTNESS: f32 = 0.12;
const BLOOM_FACTOR: f32 = 1.5;
const MASK_BRIGHTNESS: f32 = 0.70;
const GLOW: f32 = 0.9;

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
    let uv = distort(mesh.uv, CURVATURE * params.shader_intensity);
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    var color = textureSample(original_texture, original_sampler, uv).rgb;
    let glow = textureSample(bloom_texture, bloom_sampler, uv).rgb;

    let texel_y = uv.y * params.original_size.y;
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
    color = color * mask + glow * GLOW * params.shader_intensity;

    return vec4<f32>(color, 1.0);
}
