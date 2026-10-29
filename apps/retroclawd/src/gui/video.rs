use std::sync::Mutex;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::image::ImageSampler;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::{Shader, ShaderRef};
use bevy::sprite_render::{Material2d, Material2dKey};
use bevy::window::{PrimaryWindow, Window};

use super::layout::{self, BezelGeometry};
use super::shader::{Aux, Filter, ResolvedPreset, ScaleType};
use crate::emu_thread::FrameBuffer;

/// The shader selected (`--shader`, the shader hotkeys) and the two tunable intensities (see `crate::AppSettings`).
#[derive(Resource)]
pub(crate) struct ShaderSettingsRes {
    /// The selected preset's name or path (`None` = the default passthrough); the `shader_next`/`shader_prev` hotkeys move it
    /// through the bundled list.
    pub(crate) name: Option<String>,
    pub(crate) shader_intensity: f32,
    pub(crate) scanline_intensity: f32,
}

/// A preset to build the pass chain from. `rebuild_pipeline` takes it (at start-up, and when a hotkey switches the shader).
#[derive(Resource)]
pub(crate) struct PipelineRequest(pub(crate) Option<ResolvedPreset>);

/// The uniform half of every pass's bind group -- mirrored exactly (same field order and types) by the `PassParams` struct of
/// every bundled `.wgsl` file. The single-pass shaders declare only the first four fields (a buffer may be larger than the
/// struct a shader reads); `texture_size` is the size of `input` in texels.
#[derive(Clone, Copy, Default, PartialEq, ShaderType)]
pub(crate) struct PassParams {
    pub(crate) shader_intensity: f32,
    pub(crate) scanline_intensity: f32,
    /// Size of the pass's input texture.
    pub(crate) texture_size: Vec2,
    /// Size of the emulated picture, however many passes ran before.
    pub(crate) original_size: Vec2,
    /// Size of this pass's output in pixels.
    pub(crate) output_size: Vec2,
    pub(crate) frame_count: f32,
    pub(crate) pass_index: f32,
}

/// The material every pass is drawn with. Binding 1/2 is the pass's input, 3/4 the emulated picture, 5/6 the preset's extra
/// texture for the pass (a fallback image when there is none). Each pass has its own fragment shader, picked in `specialize`
/// from `slot`.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(PassKey)]
pub(crate) struct PassMaterial {
    #[uniform(0)]
    pub(crate) params: PassParams,
    #[texture(1)]
    #[sampler(2)]
    pub(crate) input: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub(crate) original: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    pub(crate) aux: Option<Handle<Image>>,
    /// Index into `PASS_SHADERS`.
    pub(crate) slot: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PassKey {
    slot: u32,
}

impl From<&PassMaterial> for PassKey {
    fn from(material: &PassMaterial) -> Self {
        Self {
            slot: material.slot,
        }
    }
}

/// The fragment shaders of every pass ever built, by slot. `Material2d::fragment_shader()` has no `&self`, so a shader chosen at
/// run time is put into the pipeline in `specialize`, by the slot in the material's key. A slot is never reused: the pipeline
/// cache is keyed by it, so a slot that changed shader would run the old one.
static PASS_SHADERS: Mutex<Vec<Handle<Shader>>> = Mutex::new(Vec::new());

fn register_shader(handle: Handle<Shader>) -> u32 {
    let mut shaders = PASS_SHADERS.lock().unwrap();
    shaders.push(handle);
    (shaders.len() - 1) as u32
}

impl Material2d for PassMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Default
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let handle = PASS_SHADERS
            .lock()
            .unwrap()
            .get(key.bind_group_data.slot as usize)
            .cloned();
        if let (Some(fragment), Some(handle)) = (descriptor.fragment.as_mut(), handle) {
            fragment.shader = handle;
        }
        Ok(())
    }
}

/// The live picture size (`Machine::render_frame`'s output, border included). Most machines never change it; CPC Gate Array mode
/// changes update it with the texture, the pass sizes and the window layout.
#[derive(Resource)]
pub(crate) struct BaseResolution {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Width:height of one picture pixel (`Machine::pixel_aspect`); `(1, 1)` is square.
    pub(crate) pixel_aspect: (u16, u16),
}

impl BaseResolution {
    /// The shape of the picture on screen: 4:3, unless the machine says otherwise (`layout::display_aspect`).
    pub(crate) fn aspect(&self) -> f32 {
        layout::display_aspect(self.width, self.height, self.pixel_aspect)
    }
}

/// Marks the quad that draws the last pass on screen, so `layout_screen` can find it without matching UI nodes.
#[derive(Component)]
pub(crate) struct ScreenSprite;

/// Marks every entity of the pass chain (cameras and quads), so a rebuild can remove them.
#[derive(Component)]
pub(crate) struct PipelineEntity;

/// Marks the bezel sprite.
#[derive(Component)]
pub(crate) struct BezelSprite;

/// The bezel shown over the picture, if any: where its screen is. `layout_screen` reads it.
#[derive(Resource, Default)]
pub(crate) struct BezelRes {
    pub(crate) geometry: Option<BezelGeometry>,
}

/// The images and the unit quad every pass shares.
#[derive(Resource)]
pub(crate) struct ScreenSurface {
    /// The emulated picture; `render_screen` writes it.
    source: Handle<Image>,
    quad: Handle<Mesh>,
    /// A 1x1 image bound where a pass has no extra texture.
    fallback: Handle<Image>,
}

/// One pass of the running chain.
struct PassRuntime {
    /// Where the pass draws; `None` for the last, which draws on screen.
    target: Option<Handle<Image>>,
    material: Handle<PassMaterial>,
    quad: Entity,
    size: UVec2,
    scale_type: ScaleType,
    scale: f32,
    absolute: UVec2,
}

/// The chain being run.
#[derive(Resource, Default)]
pub(crate) struct Pipeline {
    passes: Vec<PassRuntime>,
}

/// Holds the emu thread's latest rendered frame -- `render_screen` copies whatever is here into the source image every `Update`
/// tick, at Bevy's own pace, independent of how often the emu thread actually produces a new one.
#[derive(Resource)]
pub(crate) struct FrameBufferRes(pub(crate) FrameBuffer);

fn nearest_or_linear(filter: Filter) -> ImageSampler {
    match filter {
        Filter::Nearest => ImageSampler::nearest(),
        Filter::Linear => ImageSampler::linear(),
    }
}

/// Creates the source image at `base`'s size, black until the emu thread produces its first frame, the shared quad and the
/// fallback texture; spawns the on-screen camera.
pub(crate) fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    base: Res<BaseResolution>,
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
    // Real hardware has hard pixel edges: the picture is always sampled with nearest filtering, and the shaders that want it
    // smooth blend the texels themselves (see `sharp-bilinear.wgsl`).
    image.sampler = ImageSampler::nearest();
    let source = images.add(image);
    let fallback = images.add(Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));
    let quad = meshes.add(Rectangle::new(1.0, 1.0));

    commands.spawn(Camera2d);
    commands.insert_resource(ScreenSurface {
        source,
        quad,
        fallback,
    });
}

/// The size a pass draws at: `input` is what it reads, `viewport` the picture on screen in physical pixels.
fn pass_size(
    scale_type: ScaleType,
    scale: f32,
    absolute: UVec2,
    input: UVec2,
    viewport: UVec2,
) -> UVec2 {
    let scaled = |of: UVec2| {
        UVec2::new(
            ((of.x as f32 * scale).round() as u32).clamp(1, 8192),
            ((of.y as f32 * scale).round() as u32).clamp(1, 8192),
        )
    };
    match scale_type {
        ScaleType::Source => scaled(input),
        ScaleType::Viewport => scaled(viewport.max(UVec2::ONE)),
        ScaleType::Absolute => absolute,
    }
}

/// The size of the picture on screen in physical pixels.
fn viewport_pixels(window: &Window, base: &BaseResolution, bezel: &BezelRes) -> UVec2 {
    let placement = layout::place(window.size(), base.aspect(), bezel.geometry);
    (placement.picture_size * window.scale_factor())
        .round()
        .as_uvec2()
}

/// Builds the pass chain of `PipelineRequest`'s preset: removes the old chain, makes an image and a camera for every pass but the
/// last, a quad and a material for each, and the quad of the last pass that `layout_screen` puts in the window.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rebuild_pipeline(
    mut commands: Commands,
    mut request: ResMut<PipelineRequest>,
    mut pipeline: ResMut<Pipeline>,
    old: Query<Entity, With<PipelineEntity>>,
    mut images: ResMut<Assets<Image>>,
    mut shaders: ResMut<Assets<Shader>>,
    mut materials: ResMut<Assets<PassMaterial>>,
    surface: Res<ScreenSurface>,
    base: Res<BaseResolution>,
    settings: Res<ShaderSettingsRes>,
    bezel: Res<BezelRes>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let Some(preset) = request.0.take() else {
        return;
    };
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let viewport = windows
        .single()
        .map_or(UVec2::new(base.width, base.height), |w| {
            viewport_pixels(w, &base, &bezel)
        });
    let textures: Vec<Handle<Image>> = preset
        .textures
        .iter()
        .map(|t| {
            let mut image = Image::new(
                Extent3d {
                    width: t.width,
                    height: t.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                t.rgba.clone(),
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            image.sampler = ImageSampler::linear();
            images.add(image)
        })
        .collect();

    let count = preset.passes.len();
    let source_size = UVec2::new(base.width, base.height);
    let mut passes: Vec<PassRuntime> = Vec::with_capacity(count);
    for (index, pass) in preset.passes.iter().enumerate() {
        let last = index + 1 == count;
        let input_handle = match index {
            0 => surface.source.clone(),
            _ => passes[index - 1]
                .target
                .clone()
                .unwrap_or_else(|| surface.source.clone()),
        };
        let input_size = if index == 0 {
            source_size
        } else {
            passes[index - 1].size
        };
        let size = if last {
            viewport.max(UVec2::ONE)
        } else {
            pass_size(
                pass.scale_type,
                pass.scale,
                UVec2::new(pass.width, pass.height),
                input_size,
                viewport,
            )
        };
        let target = (!last).then(|| {
            let mut image =
                Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
            image.sampler = nearest_or_linear(pass.filter);
            images.add(image)
        });
        let aux = match pass.aux {
            Aux::None => None,
            Aux::Pass(n) => passes.get(n).and_then(|p| p.target.clone()),
            Aux::Texture(n) => textures.get(n).cloned(),
        };
        let shader = shaders.add(Shader::from_wgsl(
            pass.wgsl.clone(),
            format!("{}:pass{index}", preset.name),
        ));
        let material = materials.add(PassMaterial {
            params: PassParams {
                shader_intensity: settings.shader_intensity,
                scanline_intensity: settings.scanline_intensity,
                texture_size: input_size.as_vec2(),
                original_size: source_size.as_vec2(),
                output_size: size.as_vec2(),
                frame_count: 0.0,
                pass_index: index as f32,
            },
            input: input_handle,
            original: surface.source.clone(),
            aux: Some(aux.unwrap_or_else(|| surface.fallback.clone())),
            slot: register_shader(shader),
        });
        let quad = if let Some(target) = &target {
            // Its own layer and a camera that draws that layer into the pass's image, before the on-screen camera.
            let layer = RenderLayers::layer(index + 1);
            commands.spawn((
                Camera2d,
                Camera {
                    order: index as isize - count as isize,
                    clear_color: ClearColorConfig::Custom(Color::BLACK),
                    ..default()
                },
                RenderTarget::Image(target.clone().into()),
                Msaa::Off,
                layer.clone(),
                PipelineEntity,
            ));
            commands
                .spawn((
                    Mesh2d(surface.quad.clone()),
                    MeshMaterial2d(material.clone()),
                    Transform::from_scale(size.as_vec2().extend(1.0)),
                    layer,
                    PipelineEntity,
                ))
                .id()
        } else {
            commands
                .spawn((
                    ScreenSprite,
                    Mesh2d(surface.quad.clone()),
                    MeshMaterial2d(material.clone()),
                    Transform::default(),
                    PipelineEntity,
                ))
                .id()
        };
        passes.push(PassRuntime {
            target,
            material,
            quad,
            size,
            scale_type: pass.scale_type,
            scale: pass.scale,
            absolute: UVec2::new(pass.width, pass.height),
        });
    }
    pipeline.passes = passes;
    tracing::info!(
        "shader: {} ({count} pass{})",
        preset.name,
        if count == 1 { "" } else { "es" }
    );
}

/// Keeps the pass chain's sizes and uniforms in step with the picture and the window: the picture changing size (a CPC mode
/// change) or the window changing size (viewport-scaled passes) resizes the passes' images and quads and updates what shaders
/// read.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_pipeline(
    mut pipeline: ResMut<Pipeline>,
    request: Res<PipelineRequest>,
    base: Res<BaseResolution>,
    bezel: Res<BezelRes>,
    windows: Query<&Window, With<PrimaryWindow>>,
    settings: Res<ShaderSettingsRes>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<PassMaterial>>,
    mut transforms: Query<&mut Transform>,
) {
    // a rebuild is waiting: the chain on hand is about to go
    if request.0.is_some() || pipeline.passes.is_empty() {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let viewport = viewport_pixels(window, &base, &bezel).max(UVec2::ONE);
    let source_size = UVec2::new(base.width, base.height);
    let count = pipeline.passes.len();
    let mut input_size = source_size;
    for index in 0..count {
        let pass = &pipeline.passes[index];
        let last = index + 1 == count;
        let size = if last {
            viewport
        } else {
            pass_size(
                pass.scale_type,
                pass.scale,
                pass.absolute,
                input_size,
                viewport,
            )
        };
        if size != pass.size {
            if let Some(target) = &pass.target
                && let Some(mut image) = images.get_mut(target)
            {
                image.resize(Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                });
            }
            if !last && let Ok(mut transform) = transforms.get_mut(pass.quad) {
                transform.scale = size.as_vec2().extend(1.0);
            }
        }
        let wanted = PassParams {
            shader_intensity: settings.shader_intensity,
            scanline_intensity: settings.scanline_intensity,
            texture_size: input_size.as_vec2(),
            original_size: source_size.as_vec2(),
            output_size: size.as_vec2(),
            frame_count: 0.0,
            pass_index: index as f32,
        };
        // reading first: asking for the asset mutably marks it changed, and a changed material is bound again
        if materials
            .get(&pass.material)
            .is_some_and(|m| m.params != wanted)
            && let Some(mut material) = materials.get_mut(&pass.material)
        {
            material.params = wanted;
        }
        input_size = size;
        pipeline.passes[index].size = size;
    }
}

/// Spawns the bezel sprite (`layout_screen` sizes and places it).
pub(crate) fn spawn_bezel(commands: &mut Commands, image: Handle<Image>) {
    commands.spawn((
        BezelSprite,
        Sprite {
            image,
            custom_size: Some(Vec2::ONE),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 10.0),
    ));
}

/// Puts the picture and the bezel in the window, every frame: the picture is 4:3 (or the machine's own shape) and as large as the
/// window, or the bezel's hole, allows.
pub(crate) fn layout_screen(
    base: Res<BaseResolution>,
    bezel: Res<BezelRes>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut screen: Query<&mut Transform, (With<ScreenSprite>, Without<BezelSprite>)>,
    mut frame: Query<&mut Sprite, With<BezelSprite>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let placement = layout::place(window.size(), base.aspect(), bezel.geometry);
    if let Ok(mut transform) = screen.single_mut() {
        transform.translation = placement.picture_center.extend(0.0);
        transform.scale = placement.picture_size.max(Vec2::splat(0.0001)).extend(1.0);
    }
    if let Ok(mut sprite) = frame.single_mut() {
        sprite.custom_size = placement.bezel_size;
    }
}

/// `render_frame`'s output is RGB (3 bytes/pixel); Bevy's texture format here is RGBA (4 bytes/pixel) -- insert a fully-opaque
/// alpha byte after each pixel into a reusable output buffer.
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

/// Copies the emu thread's latest frame (`FrameBufferRes`) into the source image. Runs every `Update` tick at Bevy's own render
/// pace. If hardware changes resolution at runtime, the image and `BaseResolution` change together before the new pixels are
/// exposed; `sync_pipeline` and `layout_screen` then follow.
pub(crate) fn render_screen(
    frame_buffer: Res<FrameBufferRes>,
    mut base: ResMut<BaseResolution>,
    screen: Res<ScreenSurface>,
    mut images: ResMut<Assets<Image>>,
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

    let Some(mut image) = images.get_mut(&screen.source) else {
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
        base.width = width;
        base.height = height;
        tracing::info!("bitmap size changed: {old_width}x{old_height} -> {width}x{height}");
    }
    let rgba = image.data.get_or_insert_with(Vec::new);
    rgb_to_rgba_into(&rgb, rgba);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_is_four_by_three_unless_the_machine_says_otherwise() {
        let base = |width, height, pixel_aspect| BaseResolution {
            width,
            height,
            pixel_aspect,
        };
        assert_eq!(base(256, 192, (1, 1)).aspect(), layout::TV_ASPECT);
        assert_eq!(base(320, 224, (0, 0)).aspect(), layout::TV_ASPECT);
        assert_eq!(base(320, 200, (2, 1)).aspect(), 3.2);
    }

    #[test]
    fn pass_sizes_follow_their_scale_type() {
        let input = UVec2::new(320, 224);
        let viewport = UVec2::new(1440, 1080);
        let absolute = UVec2::new(64, 32);
        assert_eq!(
            pass_size(ScaleType::Source, 0.5, absolute, input, viewport),
            UVec2::new(160, 112)
        );
        assert_eq!(
            pass_size(ScaleType::Source, 1.0, absolute, input, viewport),
            input
        );
        assert_eq!(
            pass_size(ScaleType::Viewport, 1.0, absolute, input, viewport),
            viewport
        );
        assert_eq!(
            pass_size(ScaleType::Viewport, 0.5, absolute, input, viewport),
            UVec2::new(720, 540)
        );
        assert_eq!(
            pass_size(ScaleType::Absolute, 9.0, absolute, input, viewport),
            absolute
        );
        // never empty
        assert_eq!(
            pass_size(
                ScaleType::Source,
                0.001,
                absolute,
                UVec2::new(2, 2),
                viewport
            ),
            UVec2::ONE
        );
    }

    #[test]
    fn rgb_gets_an_opaque_alpha_byte() {
        let mut rgba = Vec::new();
        rgb_to_rgba_into(&[1, 2, 3, 4, 5, 6], &mut rgba);
        assert_eq!(rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }
}
