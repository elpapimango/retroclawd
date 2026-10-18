# New machine playbook

How any new Machine is added to retroclawd, from an 8-bit micro to a 16-bit computer or a console. A run follows this document from start to finish and **does not ask questions**: where a decision is needed it takes the default written here, and records the assumption in the final report. Terms are defined in `GLOSSARY.md`.

## Definition of done

A Machine is done when it **matches a reference emulator** (MAME or a libretro core) on the standard scenarios below. Nothing else gates completion. Anything that cannot be matched is a **known gap** (see Blockers), listed in the report.

## Blockers are recorded, never asked

When a run cannot resolve something alone (the reference looks wrong, a ROM or test file cannot be fetched, output differs and the cause is unclear):

1. Write a `known-gap` entry in the Machine's manifest: scenario, evidence (diff image, trace, what was tried), suspected cause.
2. Skip that scenario and continue.
3. List every gap in the final report.

Only a Machine that cannot boot at all stops the run. A tolerance or golden is never weakened to make a scenario pass; a tolerance is allowed only with a written reason in the manifest.

## Prerequisite: ABI readiness

The plugin ABI was audited once for 16-bit and console needs and bumped once (2026-10-18: `u64` addresses, several CPUs, stereo audio, exact frame rate, pixel aspect, analogue axes). See `docs/adr/0003-address-width-and-multi-cpu.md` and its spec. The Host side still to be written by the first Machine that needs it is listed in the spec's Status section.

## Files per Machine

- `machines/<family>/<id>.intake.toml`: inputs. Reference emulator and pinned version, hardware test suites, media formats, firmware hashes, scenario overrides. Filled in by the run in P0 from primary sources.
- `machines/<family>/<id>.manifest.toml`: outputs. Per scenario: golden hash, reference command, tolerance and reason, status (`pass` or `known-gap` with evidence path).

## Phases

Run in order.

- **P0 Intake.** Fill the intake file: Family, Core (new or reused), Chips, reference emulator and version, hardware test suites, media formats, firmware hashes, Machine keys, scenario overrides. Primary sources only (data sheets, reference emulator source). Record sources in `CREDITS.md`.
- **P1 Tools.** Make `retroclawd-tools` able to produce what the scenarios need: an assembler and disassembler target for the Core (`rcasm`, `rcdis`; round trip checked on random bytes and the Machine's ROMs), format support in `rcdisk`, `rctape` and `rcld` for its Media, symbol tables. The tools are a means, not a gate.
- **P2 Core.** New Core crate, or a variant flag on an existing one, passing its public **Core test** suite (ZEXALL/ZEXDOC for Z80; Klaus Dormann and Tom Seddon for 6502; the public instruction tests for 68000). Core tests are separate from Acid tests.
- **P3 Machine and Plugin.** Machine crate, Chips, Plugin crate, `plugin-dlls` cdylib, Options, Machine keys, overlay chip, debug contract, save state. Follow "Checklist: new machine plugin integration" in `TODO.md`.
- **P4 Media.** Format crates, attach, eject, save, fast-load traps where the reference has them.
- **P5 Golden scenarios.** Capture goldens from the reference, run the standard scenarios in `rc-suite`, fix until they match or record gaps.
- **P6 Close-out.** Update `CLAUDE.md`, `README.md`, `CREDITS.md` (sources and pinned versions) and `GLOSSARY.md` if a new term appeared. Write the final report: scenario results, known gaps, assumptions taken.

## Standard scenarios

Same for every Machine. A Machine may add scenarios in its intake file, never remove one without a recorded reason.

| ID | Scenario | Compared against |
|---|---|---|
| S1 | Cold boot to the first stable screen at frame N | Screenshot golden |
| S2 | Typed input (controller input for consoles) reaches the Machine | Screenshot after scripted keys |
| S3 | Each supported kind of Media loads and runs, using files made by our tools | Screenshot, memory or transcript golden |
| S4 | Audio: a known test sound | RMS and dominant tone (features, not sample-exact) |
| S5 | Save state round trip and reset | The Machine's own state after reload |
| S6 | Timing: cycles per Frame and frame rate | Reference numbers recorded in the manifest |
| S7 | Hardware test ROMs shipped for the platform | Their own pass/fail output |
| S8 | CPU trace of the first N instructions after reset, when the reference can emit one | Trace golden |

Scenarios are data: a `<id>.manifest.toml` in the Family repository, run by `rc-scenarios run` (crate `rc-suite` of `retroclawd-tools`, which also holds the rules: every golden pinned by hash, a tolerance only with a written reason, a known gap only with evidence, a known gap that starts passing fails the run). Kinds: `screen`, `memory`, `audio`, `state` (S5), `timing` (S6). S8 (a CPU trace) has no kind yet because the plugin ABI has no instruction trace; it stays optional.

**A picture from another emulator is compared by similarity, not pixel for pixel.** The reference draws its own border, scale and palette, so only the content can agree: both pictures are reduced to a 64x48 luminance grid over a `crop` and compared by mean absolute difference (`max_mad`, with the reason written in the manifest). Pixel-exact comparison is only for goldens this project made itself. This replaces the earlier assumption of pixel-exact comparison against the reference.

## Goldens

Captured once by a script that runs the reference emulator headless on the scenarios (`scripts/capture_mame_screen.sh` in `retroclawd-tools` for MAME pictures; not yet run), stored in RomM keyed by hash (they derive from copyrighted software, so they stay out of git like ROMs), and pinned by the manifest (reference name and version, exact command, hash). CI compares our output to the golden and never runs the reference emulator. A diff image is written when they differ. One script regenerates goldens when the reference version changes.
