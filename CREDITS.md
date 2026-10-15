# Credits

Third-party references this codebase's implementation is based on -- none
of this code is copied from these sources; each is a fresh Rust
implementation of documented behavior, cited here because that behavior
(file layouts, byte offsets, register constants) came from reading them,
not from independent derivation.


> Sources for the machine families (ZX81/Spectrum, CPC, C64/C128 and their chips and formats)
> are credited in `CREDITS.md` of `retroclawd-sinclair`, `retroclawd-amstrad` and
> `retroclawd-commodore`.


> Sources for the shared crates (AY-3-8910, uPD765A, `.dsk`, tape formats, the 6502 core) are credited in
> `CREDITS.md` of `retroclawd-crates`.

## Debugger web UI structure (`assets/debugger.html`, 2026-09-17/18)

The tabbed layout (CPU/Memory/Bitmap Explorer), dark theme with an orange
active-tab/Continue-button accent, disassembly panel's PC-arrow/
breakpoint-dot/selected-line visual language, and bottom toolbar shape
(Step/Breakpoint/Run until breakpoint/Clear breakpoints/Save listing/Back
to PC/hex-address-Go) were modeled on the Atari ST Emulator (ASE)'s own
web debugger, per the user's own reference: <https://github.com/
thebitculture/ase/wiki#12-the-debugger> ("Chapter 12: The Debugger"),
plus two screenshots of ASE's actual UI the user looked at (not redistributed here). ASE's own "Bitmap explorer"
tab -- a generic Atari-bitplane memory-to-pixel viewer, config'able start
address/bytes-width/bitplane-count/zoom, mouse-wheel scroll (one line/
Shift+wheel eight lines/Ctrl+wheel one byte to realign bitplanes),
"Wrap"/single-strip toggle, a palette swatch row, and PNG export -- was
adapted closely, all of it real behavior visible in
the screenshot's own panel text ("Reads RAM as Atari bitplanes
and paints it with the palette the machine is holding right now"), not
copied verbatim: retroclawd's own visualization palette is a fixed
16-color generic one, not each emulated machine's real hardware palette
(that would need a new cross-`AnyMachine` palette accessor, not built in
this pass -- see `TODO.md`). First draft of this tab got the whole
premise backwards -- built it as a live screenshot of the emulated
display instead (`GET /api/screen.png` off `emu_thread`'s own
`FrameBuffer`), on the mistaken read that ASE's Bitmap Explorer showed
"what's on screen." User corrected this directly after reviewing:
"screens is not supposed to be a screenshot of current screen but a
visual representation of the memory visualized as bitmaps... that is
what ase can do and what is described in chapter 12" -- the screenshot
route was removed entirely and replaced with a real raw-memory bitplane
viewer (`GET /api/memory-bytes?addr=&len=` handing back bytes, all
decode/palette/zoom/export done client-side on a `<canvas>`), same day.
The interleaved-bitplane decode (`width` bytes per bitplane per scanline,
`bitplanes` planes per row, MSB-first bit order -- the same row shape
Atari ST/Amiga planar graphics use) was verified pixel-exact against a
hand-computed expected image via Playwright's `canvas.getImageData`, for
both a 1-bitplane and a 4-bitplane case, not just eyeballed.

## CRT/scanline shader (`assets/shaders/crt.wgsl`, `assets/shaders/scanlines.wgsl`, 2026-09-20)

Curvature distortion (`distort`'s barrel-distortion formula), scanline
weighting (`scan_line_weight`, including the 3-tap `MULTISAMPLE`
averaging ported as `scan_line`), the simple two-column green/magenta
phosphor mask, and the `BLOOM_FACTOR` brightening of bright scanlines are
hand-ported from **crt-pi**, Copyright 2015-2016 davej, licensed **GNU
General Public License v2 or later**:
<https://github.com/libretro/glsl-shaders/blob/master/crt/shaders/crt-pi.glsl>.
Fetched and read directly this session, not summarized from a
description. `CURVATURE`/`SCANLINE_WEIGHT`/`SCANLINE_GAP_BRIGHTNESS`/
`BLOOM_FACTOR`/`MASK_BRIGHTNESS` are that shader's own default parameter
values. Simplified from the original: no gamma correction, and
`screenScale` dropped (always 1.0 for this project's own texture, which
is never a padded power-of-two size the way crt-pi's can be).

**License note**: GPL v2-or-later code may be combined into a GPL v3-or-later work, so these two files
stay under their own GPL-2.0-or-later notice and the project as a whole is distributed under
GPL-3.0-or-later (`LICENSE`).

## Selected libretro CRT ports (`assets/shaders/crt-geom.wgsl`, `assets/shaders/zfast-crt.wgsl`, `assets/shaders/crt-potato.wgsl`, 2026-09-24)

These are fresh WGSL ports/approximations based on the single-pass upstream
slang shaders in **libretro/slang-shaders**, not copied source:

- `crt-geom`: <https://github.com/libretro/slang-shaders/blob/master/crt/shaders/crt-geom.slang>
- `zfast-crt`: <https://github.com/libretro/slang-shaders/blob/master/crt/shaders/zfast_crt/zfast_crt_impl.inc>
- `crt-potato`: <https://github.com/libretro/slang-shaders/blob/master/crt/shaders/crt-potato/shader-files/crt-potato.slang>

The upstream shaders are GPL-licensed. The ports preserve their core
single-pass effects while fitting this renderer's one-texture `Material2d`:
`crt-geom` keeps curved geometry, beam scanlines, and a procedural aperture
mask; `zfast-crt` keeps cubic-style resampling, luminance-dependent scanlines,
and its fine mask; `crt-potato` uses a procedural RGB mask in place of the
upstream external PNG mask texture. Progressive rendering and the existing
`shader_intensity`/`scanline_intensity` controls replace upstream's larger
per-preset parameter sets. Full `crt-royale` and `mame_hlsl` remain deferred;
their presets require 12 and 11 render passes, intermediate targets, and mask
textures rather than one fragment shader.

The license note from the crt-pi entry above applies here too: the GPL-derived shader work is
distributed under GPL-3.0-or-later with the rest of the project.

## Smooth-upscale shader (`assets/shaders/sharp-bilinear.wgsl`, 2026-09-20)

Not a literal port -- `sharp-bilinear.wgsl` is a plain hand-rolled 4-tap
bilinear filter (see its own header comment for why: this project's
texture is always nearest-filtered, so there's no hardware bilinear
sampler to lean on). It was written with **sharp-bilinear-simple**,
Copyright rsn8887 (based on TheMaister), **public domain**, as the
reference for "prescale + bilinear is the standard smooth-upscale
technique" and for what problem that family of shaders solves:
<https://github.com/libretro/glsl-shaders/blob/master/interpolation/shaders/sharp-bilinear-simple.glsl>.
Fetched and read directly this session. The original shader's actual
integer-prescale-then-clamp trick (the part that avoids "wobble" at a
*non-integer* output scale) isn't reproduced -- this project's own window
fitting (`gui::video::fit_screen`) only ever applies whole-number scale
factors, so that specific problem doesn't come up here.

## Spectrum `.POK` trainer format (`crates/formats/pok`, 2026-09-24)

Record semantics (`N` trainer name, `M`/`Z` bank/address/value/original entries,
`Y` terminator) were read directly from ZEsarUX's `src/utils.c`
(`util_parse_pok_mem`) and its embedded format comment,
<https://github.com/chernandezba/zesarux>. This project contains a fresh Rust
parser and does not copy ZEsarUX code. Bank-specific entries use the existing
plugin memory-write path with a named `ram-bank:N` address space.

## Bitmap Explorer machine palettes (2026-09-24)

Debugger palette metadata reuses this project's existing machine palette
constants: ZX Spectrum's conventional normal/bright ULA levels, CPC's
hardware-numbered Gate Array palette from CPCTech, and C64's Pepto palette
from `vic-ii`. ZX81 exposes its monochrome black/white display colors. The
Bitmap Explorer receives these through `plugin-api`; it does not read or
mutate live renderer state.
