// Smooth, non-CRT upscale -- no adjustable effect (shader_intensity/
// scanline_intensity are both unused).
//
// Manual bilinear reconstruction: this project's screen texture always
// uses nearest filtering (crisp pixel edges for every other shader,
// including the passthrough default), so there is no hardware-bilinear
// sampler to lean on here -- instead this samples the four texels around
// `mesh.uv` and blends them by hand, which is exactly what a bilinear
// sampler does internally at that point.
//
// This is a plain bilinear filter, not a literal port of libretro's
// "sharp-bilinear-simple" (Copyright rsn8887, based on TheMaister, public
// domain: https://github.com/libretro/glsl-shaders/blob/master/
// interpolation/shaders/sharp-bilinear-simple.glsl) -- that shader's real
// value is its integer-prescale-then-clamp trick, which exists to avoid
// "wobble" when the output scale is a *non-integer* multiple of the
// source resolution. This project's own window-fitting
// (`gui::video::fit_screen`) only ever applies whole-number scale
// factors, so that specific problem doesn't arise here; plain bilinear
// already gives the smooth, non-pixel-perfect look asked for.

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
    let texel = mesh.uv * params.texture_size - 0.5;
    let base = floor(texel);
    let frac = fract(texel);
    let texel_size = 1.0 / params.texture_size;
    let uv00 = (base + 0.5) * texel_size;

    let c00 = textureSample(screen_texture, screen_sampler, uv00).rgb;
    let c10 = textureSample(screen_texture, screen_sampler, uv00 + vec2<f32>(texel_size.x, 0.0)).rgb;
    let c01 = textureSample(screen_texture, screen_sampler, uv00 + vec2<f32>(0.0, texel_size.y)).rgb;
    let c11 = textureSample(screen_texture, screen_sampler, uv00 + texel_size).rgb;

    let top = mix(c00, c10, frac.x);
    let bottom = mix(c01, c11, frac.x);
    let color = mix(top, bottom, frac.y);

    return vec4<f32>(color, 1.0);
}
