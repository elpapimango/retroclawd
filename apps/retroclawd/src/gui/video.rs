use std::sync::OnceLock;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::{Shader, ShaderRef};
use bevy::sprite_render::Material2d;
use bevy::window::{PrimaryWindow, Window};

use crate::emu_thread::FrameBuffer;

/// `--shader`'s already-resolved WGSL text plus the two tunable
/// intensities (see `crate::AppSettings`'s own doc comment) -- read once
/// by `setup`; afterwards only `switch_shader` (the shader hotkeys) changes
/// the shader.
#[derive(Resource)]
pub(crate) struct ShaderSettingsRes {
    pub(crate) source: String,
    /// The selected shader's name or path (`None` = default passthrough);
    /// the `shader_next`/`shader_prev` hotkeys move it through the bundled
    /// list and replace the shader asset's source in place.
    pub(crate) name: Option<String>,
    pub(crate) shader_intensity: f32,
    pub(crate) scanline_intensity: f32,
}

/// The uniform half of `ScreenMaterial`'s bind group -- mirrored exactly
/// (same field order/types) by every bundled `.wgsl` file's own
/// `ScreenParams` struct. `texture_size` is the live picture width/height
/// in texels. It is updated with the texture and mesh when hardware changes
/// resolution at runtime (CPC Gate Array Modes 0/1/2), so per-scanline and
/// per-texel shader math continues to address the new native grid.
#[derive(Clone, Copy, Default, ShaderType)]
pub(crate) struct ScreenParams {
    pub(crate) shader_intensity: f32,
    pub(crate) scanline_intensity: f32,
    pub(crate) texture_size: Vec2,
}

/// The material the single `ScreenSprite` entity is rendered with --
/// always a `Material2d`, even for the default/no-shader case (a plain
/// passthrough shader), so there's exactly one rendering code path rather
/// than a `Sprite`-vs-`Mesh2d` branch depending on whether a shader is
/// active.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(crate) struct ScreenMaterial {
    #[uniform(0)]
    pub(crate) params: ScreenParams,
    #[texture(1)]
    #[sampler(2)]
    pub(crate) screen_texture: Handle<Image>,
}

/// `Material2d::fragment_shader()` has no `&self` (verified against
/// `bevy_sprite_render`'s real trait definition, not guessed) -- it's
/// resolved once per `ScreenMaterial` Rust type, not per instance, so a
/// shader chosen at runtime (from `--shader`) can't be returned directly
/// from that method. This `OnceLock`, set once by `setup` before any
/// `ScreenMaterial` instance is ever rendered, is the standard workaround:
/// `fragment_shader()` just reads whatever was set here.
static SHADER_HANDLE: OnceLock<Handle<Shader>> = OnceLock::new();

impl Material2d for ScreenMaterial {
    fn fragment_shader() -> ShaderRef {
        match SHADER_HANDLE.get() {
            Some(handle) => ShaderRef::Handle(handle.clone()),
            None => ShaderRef::Default,
        }
    }
}

/// The live picture size (`Machine::render_frame`'s output, border included).
/// Most machines never change it; CPC Gate Array mode changes update it with
/// the texture, mesh, shader uniform, scaling, and window-aspect systems.
#[derive(Resource)]
pub(crate) struct BaseResolution {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Width:height of one picture pixel (`Machine::pixel_aspect`); `(1, 1)`
    /// is square.
    pub(crate) pixel_aspect: (u16, u16),
}

impl BaseResolution {
    /// The picture's shape on screen: the pixel grid with each pixel widened
    /// (or narrowed) by `pixel_aspect`. Window sizing works in this shape;
    /// `fit_screen` then stretches the texture to the window.
    pub(crate) fn display_size(&self) -> Vec2 {
        let (num, den) = self.pixel_aspect;
        let aspect = if num == 0 || den == 0 {
            1.0
        } else {
            f32::from(num) / f32::from(den)
        };
        Vec2::new(self.width as f32 * aspect, self.height as f32)
    }
}

/// Marks the single sprite entity that displays the emulated picture, so
/// `fit_screen` can find it without also matching UI/debug-overlay nodes.
#[derive(Component)]
pub(crate) struct ScreenSprite;

#[derive(Resource)]
pub(crate) struct ScreenSurface {
    texture: Handle<Image>,
    mesh: Handle<Mesh>,
    material: Handle<ScreenMaterial>,
}

/// Holds the emu thread's latest rendered frame -- `render_screen` copies
/// whatever's here into the live texture every `Update` tick, at Bevy's
/// own pace, independent of how often the emu thread actually produces a
/// new one.
#[derive(Resource)]
pub(crate) struct FrameBufferRes(pub(crate) FrameBuffer);

/// Initial window scale over the real, tiny 256x192 (or similar) picture --
/// only used to size the window at startup. After that, `fit_screen`
/// recomputes the sprite's actual scale every frame from the window's live
/// size, so this is a starting point, not a cap.
pub(crate) const INITIAL_SCALE: f32 = 3.0;

/// Creates the live texture at `base`'s size, filled with black until the
/// emu thread produces its first real frame (`render_screen` overwrites it
/// on the very next `Update` tick in practice) -- unlike before the
/// emu-thread split, there's no machine resource to read an initial frame
/// from directly here, since the machine now lives only on the emu thread.
/// Replaces the screen shader's WGSL source in place. The material keeps its
/// handle; Bevy sees the shader asset change (the same path its own shader
/// hot-reload uses) and rebuilds the render pipeline with the new source.
/// Returns `false` before `setup` has created the shader.
pub(crate) fn switch_shader(shaders: &mut Assets<Shader>, source: String) -> bool {
    let Some(handle) = SHADER_HANDLE.get() else {
        return false;
    };
    shaders
        .insert(handle.id(), Shader::from_wgsl(source, "screen_shader"))
        .is_ok()
}

pub(crate) fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ScreenMaterial>>,
    mut shaders: ResMut<Assets<Shader>>,
    base: Res<BaseResolution>,
    shader_settings: Res<ShaderSettingsRes>,
) {
    let pixel_count = base.width as usize * base.height as usize;
    let mut image = Image::new(
        Extent3d {
            width: base.width,
            height: base.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; pixel_count * 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    // Real hardware has hard pixel edges -- linear filtering (bevy's
    // per-image default) blurs them under any non-integer or magnified
    // scale. `fit_screen` only ever applies integer scale factors, but
    // nearest-neighbor is what makes each of those factors look crisp
    // rather than softened. Every bundled shader assumes this (see e.g.
    // `sharp-bilinear.wgsl`'s own doc comment on why it hand-rolls
    // bilinear instead of relying on a hardware-filtered sampler).
    image.sampler = ImageSampler::nearest();
    let handle = images.add(image);

    let shader_handle = shaders.add(Shader::from_wgsl(
        shader_settings.source.clone(),
        "screen_shader",
    ));
    // See `SHADER_HANDLE`'s own doc comment for why this global is needed
    // at all. `setup` runs in `Startup`, strictly before any render
    // extraction that could call `ScreenMaterial::fragment_shader()`, so
    // this is always set before it's ever read.
    let _ = SHADER_HANDLE.set(shader_handle);

    let material = materials.add(ScreenMaterial {
        params: ScreenParams {
            shader_intensity: shader_settings.shader_intensity,
            scanline_intensity: shader_settings.scanline_intensity,
            texture_size: Vec2::new(base.width as f32, base.height as f32),
        },
        screen_texture: handle.clone(),
    });
    let mesh = meshes.add(Rectangle::new(base.width as f32, base.height as f32));

    commands.spawn(Camera2d);
    commands.spawn((
        ScreenSprite,
        Mesh2d(mesh.clone()),
        MeshMaterial2d(material.clone()),
        Transform::from_scale(Vec3::splat(INITIAL_SCALE)),
    ));
    commands.insert_resource(ScreenSurface {
        texture: handle,
        mesh,
        material,
    });
}

/// `render_frame`'s output is RGB (3 bytes/pixel); Bevy's texture format
/// here is RGBA (4 bytes/pixel) -- insert a fully-opaque alpha byte after
/// each pixel into a reusable output buffer.
fn rgb_to_rgba_into(rgb: &[u8], rgba: &mut Vec<u8>) {
    let needed = rgb.len() / 3 * 4;
    if rgba.len() != needed {
        rgba.resize(needed, 255);
    }

    let mut src = 0usize;
    let mut dst = 0usize;
    while src < rgb.len() {
        rgba[dst] = rgb[src];
        rgba[dst + 1] = rgb[src + 1];
        rgba[dst + 2] = rgb[src + 2];
        rgba[dst + 3] = 255;
        src += 3;
        dst += 4;
    }
}

/// Copies the emu thread's latest frame (`FrameBufferRes`) into the live
/// texture. Runs every `Update` tick at Bevy's own render pace. If hardware
/// changes resolution at runtime, the texture, quad, shader uniform, and
/// `BaseResolution` are changed atomically before the new pixels are exposed;
/// the existing fit/aspect systems then consume the new base size.
pub(crate) fn render_screen(
    frame_buffer: Res<FrameBufferRes>,
    mut base: ResMut<BaseResolution>,
    screen: Res<ScreenSurface>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ScreenMaterial>>,
) {
    let Some((width, height, rgb)) = std::mem::take(&mut *frame_buffer.0.lock().unwrap()) else {
        return;
    };
    let Some(expected_len) = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(3))
        .map(|bytes| bytes as usize)
    else {
        return;
    };
    if rgb.len() != expected_len {
        return;
    }

    let Some(mut image) = images.get_mut(&screen.texture) else {
        return;
    };
    if width != base.width || height != base.height {
        let old_width = base.width;
        let old_height = base.height;
        image.resize(Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        });
        if let Some(mut mesh) = meshes.get_mut(&screen.mesh) {
            *mesh = Rectangle::new(width as f32, height as f32).into();
        }
        if let Some(mut material) = materials.get_mut(&screen.material) {
            material.params.texture_size = Vec2::new(width as f32, height as f32);
        }
        base.width = width;
        base.height = height;
        tracing::info!("bitmap size changed: {old_width}x{old_height} -> {width}x{height}");
    }
    let rgba = image.data.get_or_insert_with(Vec::new);
    rgb_to_rgba_into(&rgb, rgba);
}
/// The largest whole-number scale factor of `base` that still fits inside
/// `available` (never below `1.0`) -- used to size the *initial window*
/// (`window::apply_initial_window_size`) as an exact multiple of the
/// starting bitmap.
///
/// Runtime rendering itself is now explicitly window-driven (`fit_screen`
/// below): window size stays whatever the user (or window manager) sets,
/// and the bitmap scales to that current size every frame.
pub(crate) fn integer_scale(available: Vec2, base: Vec2) -> f32 {
    (available.x / base.x)
        .min(available.y / base.y)
        .floor()
        .max(1.0)
}

/// Recomputes the on-screen sprite's scale every frame from the window's
/// current (live) size -- the bitmap follows the window size, not the
/// other way around. X/Y are scaled independently to fill the current
/// window area exactly; no window resizing is triggered from bitmap-size
/// changes.
pub(crate) fn fit_screen(
    base: Res<BaseResolution>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut sprite: Query<&mut Transform, With<ScreenSprite>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(mut transform) = sprite.single_mut() else {
        return;
    };

    let scale_x = window.width() / base.width as f32;
    let scale_y = window.height() / base.height as f32;
    transform.scale = Vec3::new(scale_x.max(0.0001), scale_y.max(0.0001), 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_display_size_widens_the_picture_by_its_pixel_aspect() {
        let base = |pixel_aspect| BaseResolution {
            width: 320,
            height: 200,
            pixel_aspect,
        };
        assert_eq!(base((1, 1)).display_size(), Vec2::new(320.0, 200.0));
        assert_eq!(base((6, 5)).display_size(), Vec2::new(384.0, 200.0));
        assert_eq!(base((1, 2)).display_size(), Vec2::new(160.0, 200.0));
        // An unset aspect (a zero) is square.
        assert_eq!(base((0, 0)).display_size(), Vec2::new(320.0, 200.0));
    }
}
