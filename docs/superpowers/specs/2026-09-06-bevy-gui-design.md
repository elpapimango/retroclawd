# Bevy GUI built into `retroclawd`: live display + keyboard input

Date: 2026-09-06
Status: Approved, pending implementation plan

## Problem

Both `zx81` and `zx-spectrum` now have real keyboard matrices
(`press_key`/`release_key`) and real video output (`Machine::render_frame`),
but nothing drives them interactively. `retroclawd`'s only mode today is
batch: parse args, build a machine, run to completion (or a `--max-cycles`
cutoff), print stats, optionally dump memory/a screen PNG, exit. There is
no way to actually watch a machine run or type into it.

## Non-goals

- A separate GUI binary. This is built into `retroclawd` itself, not
  `apps/retroclawd-gui` or similar — explicitly decided against a second
  binary.
- Any button/menu/panel UI. Bevy's job here is exactly two things: show
  the current video frame, and turn keyboard events into
  `press_key`/`release_key` calls. Registers/memory/disassembly UI is a
  separate, already-logged idea (`TODO.md`'s "Debug/dev tooling: embedded
  web UI") and explicitly not this task.
- Full punctuation/symbol keyboard mapping. Letters, digits, Space, Enter,
  and Shift map concretely in this round; ZX Spectrum's SYM SHIFT-based
  symbol entry and other punctuation are a documented follow-up (a data
  table to extend, not an architecture change).
- Any change to `Machine`, `Clock`, or either machine's `render_frame`/
  keyboard code. Everything this needs is already public; see "Why no
  trait changes are needed" below.
- Real hardware video timing (NMI/HALT coupling, FLASH blinking). Each
  machine's `render_frame` is still the same static, on-demand decode
  built earlier this session — this task just calls it repeatedly (once
  per `FixedUpdate` tick) instead of once, which does not require it to
  become anything more than that.

## Why no trait changes are needed

`Machine::run()` blocks (it owns real-time pacing via `Clock`, sleeping
between ticks) and would freeze the entire Bevy app — rendering, input,
everything — until the machine halts, which for a real ROM boot with no
reachable `HALT` is forever. This was the central risk flagged going into
this design.

The fix needs no interface change at all: `step()`, `halted()`,
`clock_hz()`, and `frame_hz()` are already public, required `Machine`
trait methods. A Bevy system can replicate `Clock`'s own per-tick budget
math (`cycles_per_tick = clock_hz() / frame_hz()`) directly, driven by
Bevy's `FixedUpdate` schedule instead of `Clock`'s sleep loop —
`TODO.md`'s own Bevy section already anticipated exactly this shape. No
`Clock`, no `Machine::run()`, no trait changes.

## Mode switch: `--headless`

`retroclawd` gains a `--headless` flag. With it, behavior is **exactly**
what exists today: batch run, print stats, `--max-cycles`/
`--dump-memory`/`--dump-screen` all work unchanged. Without it (the new
default), `retroclawd --machine <m> --rom <path>` opens a live window
instead of running to completion in the terminal.

`--max-cycles`, `--dump-memory`, and `--dump-screen` require `--headless`
— they don't mean anything for an ongoing interactive session. Passing any
of them without `--headless` is a clap-validated error (`ArgGroup`/
`requires`, not a hand-rolled check), not silently ignored: this is a
likely mistake (expecting batch behavior) that deserves a clear message,
matching this project's established "clear error over silent wrong
behavior" stance.

`--org`/`<path-to-program>` still work identically in GUI mode: load the
program (or don't) before entering the interactive loop, exactly as today
before entering the batch loop.

This is a real behavior change to `retroclawd`'s default invocation —
existing scripts/docs calling `retroclawd --machine zx81 --rom <path>`
expecting batch output now get a window instead. `README.md`'s examples
need `--headless` added throughout.

## `AnyMachine`: per-machine key dispatch without touching `Machine`

New enum in `apps/retroclawd` (not a library crate — this is purely a
CLI/GUI-layer concern):

```rust
enum AnyMachine {
    Zx81(zx81::Zx81),
    ZxSpectrum(zx_spectrum::ZxSpectrum),
}

impl AnyMachine {
    fn as_machine(&self) -> &dyn machine::Machine { ... }
    fn as_machine_mut(&mut self) -> &mut dyn machine::Machine { ... }

    /// Translates one Bevy keyboard event into this machine's own
    /// press_key/release_key call, via its own key-mapping table. A key
    /// with no mapping for this machine is a no-op.
    fn key_event(&mut self, key: bevy::input::keyboard::KeyCode, pressed: bool) { ... }
}
```

Shared operations (`step`, `halted`, `render_frame`, `clock_hz`,
`frame_hz`) go through `as_machine()`/`as_machine_mut()` and the `Machine`
trait. Keyboard input is deliberately **not** on the trait (same
reasoning as ZX81's original keyboard design: keyboards don't generalize
across machines) — `AnyMachine::key_event` matches on the concrete variant
and calls that machine's own `press_key`/`release_key` with its own key
enum (`Zx81Key`/`ZxSpectrumKey`). This is the only place either key enum
is named outside its own crate.

Built once at startup from `--machine`/`--rom` (reusing the existing
`rom_size`/`build_machine`-style validation already in `main.rs`), then
held as a Bevy `Resource`.

## Stepping the machine: a `FixedUpdate` system, no `Clock`

```rust
fn step_machine(mut machine: ResMut<AnyMachine>) {
    let m = machine.as_machine_mut();
    if m.halted() {
        return;
    }
    let budget = (m.clock_hz() / m.frame_hz()).max(1);
    let mut spent = 0u64;
    while spent < budget && !m.halted() {
        spent += m.step() as u64;
    }
}
```

Scheduled on Bevy's `FixedUpdate`, with `Time<Fixed>`'s timestep set to
match the machine's own `frame_hz()` (50 or 60 Hz depending on region) so
real-time pacing is still correct — same rate `Clock` would have used, just
driven by Bevy's own scheduler instead of a sleep loop. `.max(1)` mirrors
`Clock`'s own guard against a zero budget.

## Display: sprite backed by a dynamic `Image`

A single Bevy sprite, textured with a Bevy `Image` asset created once at
startup (256×192, `TextureFormat::Rgba8UnormSrgb` or similar — `render_frame`
returns RGB, so the alpha channel is filled as opaque when copying into the
texture). Before entering the Bevy loop, call `machine.render_frame()` once
at startup: `None` is a clear startup error ("no video output implemented
for {machine} yet", matching `--dump-screen`'s existing message for the
same case), not something discovered mid-render. Both current machines
always return `Some`, so this check exists for a future machine without
`render_frame` implemented, not because it can happen today. Once past
that check, the per-tick render system treats every subsequent call as
infallible `Some` and copies the RGB bytes into the texture, inserting an
alpha byte after each pixel.

The window itself is scaled 3x (768×576) from the real 256×192 resolution,
for visibility on a modern screen — the sprite's transform scale, not the
texture resolution itself (the texture stays pixel-accurate at 256×192;
Bevy's camera/sprite scaling handles the visual upscale, keeping pixels
sharp rather than blurring through a pre-scaled texture).

## Keyboard input: Bevy `ButtonInput<KeyCode>` -> `AnyMachine::key_event`

A system reads Bevy's `ButtonInput<KeyCode>` resource each frame, checking
`just_pressed`/`just_released` for every key in a small fixed mapping
table, and calls `machine.key_event(key, true/false)` for each transition.

Concrete mapping for this round: `KeyA..KeyZ` -> the corresponding letter
key (both machines' key enums use the same `A`..`Z` naming), `Digit0`..
`Digit9` -> `Digit0`..`Digit9`, `Space` -> `Space`, `Enter` -> `Enter`,
`ShiftLeft`/`ShiftRight` -> `Zx81Key::Shift` (ZX81) or
`ZxSpectrumKey::CapsShift` (Spectrum). Any other Bevy key is a no-op this
round (not an error) — this is a data table, trivially extended later for
punctuation/SYM SHIFT without touching the surrounding architecture.

## Cargo/dependency notes

`bevy = "0.19"` (latest at design time: 0.19.1), `default-features =
false` with only the features actually needed enabled (windowing, 2D
rendering/sprites, keyboard input) — not the full default set (audio,
glTF, PBR, UI widgets, etc.), matching this project's existing "narrower
dependency, lower lifecycle cost" practice (see the `png` vs. `image`
crate choice for `--dump-screen`). The exact feature flag list is
verified empirically during implementation (built up from compile/runtime
errors as features are trimmed) rather than asserted here from memory —
Bevy's feature flags are easy to get subtly wrong without testing against
the actual crate version.

This is `retroclawd`'s first genuinely heavy dependency; expect a real
increase in compile time and binary size even for `--headless`-only use,
since `bevy` is linked into the same binary. Accepted cost of the
single-binary decision.

## Testing

Mostly manual/visual, matching this project's established practice for
anything not cheaply assertable in a unit test (see `CLAUDE.md`'s UI
testing guidance): boot each machine's real ROM in GUI mode, confirm the
window shows the actual boot screen (same content already verified via
`--dump-screen` for both machines this session), and confirm typing on
the keyboard visibly affects the running BASIC interpreter (e.g., letters
appear on screen, matching the same key producing the same character a
real machine would show).

The one thing worth an automated test: `AnyMachine::key_event`'s mapping
table itself, and the `--headless` flag's interaction with the
batch-only flags (clap's `requires`/conflict validation) — both are pure
logic with no window/rendering involved.

## Open questions

None blocking. One thing to watch during implementation: Bevy 0.19's
exact `Image`/texture-update API may have shifted from what's described
here in outline — the plan should verify the actual API against the
pinned version rather than assume this spec's sketch compiles verbatim.
