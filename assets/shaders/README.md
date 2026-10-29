# Shader presets

`--shader` takes a bundled name, a `.toml` preset or a `.wgsl` file (see `apps/retroclawd/src/gui/shader.rs`).

Bundled: `passthrough` (default), `crt`, `scanlines`, `sharp-bilinear`, `crt-geom`, `zfast-crt`, `crt-potato` (one pass each) and
`crt-bloom` (four passes, `crt-bloom.toml`).

## A preset

```toml
name = "mine"

[[texture]]            # a PNG a pass can read as `aux`
name = "mask"
path = "mask.png"      # relative to this file

[[pass]]
shader = "bloom-prefilter"   # a bundled pass shader or a .wgsl path relative to this file
scale_type = "source"        # source (default): scale x the pass's input size; viewport: scale x the picture on screen
scale = 0.5                  # absolute: width and height instead
filter = "linear"            # how the NEXT pass samples this pass's output (linear default, nearest)
aux = "pass0" or "mask"      # optional: an earlier pass's output or a [[texture]]

[[pass]]
shader = "crt-bloom"         # the last pass draws on screen
```

Every pass but the last draws into an image; the last draws the picture on screen (4:3 inside the window, or inside the bezel).

## What a pass shader sees

```wgsl
struct PassParams {
    shader_intensity: f32,       // --shader-intensity
    scanline_intensity: f32,     // --scanline-intensity
    texture_size: vec2<f32>,     // size of `input`
    original_size: vec2<f32>,    // size of the emulated picture
    output_size: vec2<f32>,      // size this pass draws at
    frame_count: f32,
    pass_index: f32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PassParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var input_texture: texture_2d<f32>;     // the pass before (the picture for pass 0)
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var input_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var original_texture: texture_2d<f32>;  // the emulated picture, nearest filtered
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var original_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var aux_texture: texture_2d<f32>;       // `aux`, or a black pixel
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var aux_sampler: sampler;
```

A shader declares only what it uses (the single-pass shaders declare the first four fields of `PassParams` and bindings 0-2).
The vertex stage is Bevy's mesh2d one: `#import bevy_sprite::mesh2d_vertex_output::VertexOutput`, `mesh.uv`, `mesh.position`.

crt-royale and mame_hlsl are not bundled: they would have to be ported pass by pass to this format.
