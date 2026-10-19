# ABI readiness for 16-bit machines and consoles

Prerequisite phase of `docs/new-machine-playbook.md`. Audit of `shared/crates/{plugin-api,machine}` and the host against the Atari ST, Amiga, Genesis-class and console machines, with the change that fixes it. One bump (protocol 10 to 11, ABI minor 1 to 2, profile contract 1 to 2), done before any such machine starts, so no machine-adding run stops on an ABI change. Recorded as ADR 0003.

## Audit: what breaks and what does not

| Area | Today | Problem for 16-bit and consoles | Verdict |
|---|---|---|---|
| Addresses | `u16` in `Machine` (`pc`, `read_memory`, `load`, `set_pc`), the fast-path `pc` (`MachinePcFnV1`), `StepResult.pc`, every `DebugCommand`/`MachineCommand` address, `DisassemblyLine`, breakpoints, `BitmapExplorerDefaults` | 68000 has a 24-bit bus, 68020+ and MIPS/ARM 32-bit | **Change** |
| Read length | `ReadMemory { len: u16 }` | A 64K+ window (Amiga chip RAM, cartridge ROM) cannot be read in one call | **Change** |
| `machine::Bus` | 16-bit memory and port space | A 68000 core needs wider addresses and 16-bit data accesses | **Keep**, see below |
| CPUs per Machine | One. `StepResult`, `GetRegisters`, `Breakpoints` assume a single CPU | Genesis (68000 + Z80), ST/Amiga (CPU + blitter/copper, sound CPUs on some) | **Change** (debug surface only) |
| Audio | `drain_audio` mono `f32` at fixed `AUDIO_SAMPLE_RATE` 44100 | Amiga (Paula), ST (YM2149 + DMA), Genesis, SNES are stereo; native rates differ | **Change** |
| Frame rate | `frame_hz() -> u64` (50, 60) | 59.94 / 59.92 / 49.92 Hz drift against audio over a long run | **Change** |
| Video | `render_frame -> (w, h, RGB)` | No pixel aspect ratio (ST low-res, consoles, Amiga lo-res); interlace is not modelled | **Add** aspect. Interlace stays inside the Machine (it weaves or blends before returning a frame) |
| Input | `KeyEvent{key, pressed}`, mouse `dx,dy: i8`, key names per profile | Digital only. Analogue sticks and triggers (N64, PS1, Amiga/ST analogue joysticks) have no path | **Add** `AxisEvent` |
| Media | `MediaKind` has Tape, Disk, Cartridge, Snapshot, Program, RomSet, Other(String); `MediaSlot` has Disk{drive}, Cartridge{slot}, Other | Covers consoles and CD images (`Other("cd")`) | **No change** |
| Region, options, key names, save states | generic | A keyboard-less console reports no or few key names | **No change** |
| I/O port space | `u16` ports, `WriteIo`, `SetPortWatches` | x86-style only; 68k/console have none (already an error path) | **No change** |
| `CpuFamily` | `Z80`, `Mos6502`, `Other(String)` | New cores | **Append** `M68k` and the likely next ones after `Other` |
| Registers | `RegisterField.value: u64`, `width_bits` | 68000 D0-D7, A0-A7, SR fit | **No change** |
| Address spaces | `AddressSpaceInfo { name, addr_width_bits: u8 }` and `space` on read/write | Already generic | **No change** |

## Design

### Addresses: `u64` at the boundary, per-Core inside

- `machine` gains `pub type Addr = u64;`. `Machine::{pc, read_memory, load, set_pc}` use `Addr`. Clean cutover across all four repositories and the host (guardrail: no aliases, no dual paths).
- `plugin-api`: every address in `protocol.rs` becomes `u64`; `ReadMemory`/`GetDisassembly` lengths and line counts become `u32`. bincode varint keeps small values small.
- **Fast path**: append `pc_wide: MachinePcWideFnV1 = fn(instance) -> u64` to `MachinePluginVTableV1`; `ABI_MINOR_V1` becomes 2. The old `pc` field stays in place (frozen layout) and returns the low 16 bits; the host uses `pc_wide`. The host's existing minor check refuses older plugins with the standard rebuild message.
- **Narrow machines** truncate nothing silently: an address above a Machine's widest address space is an error (as `WriteMemory` already rejects an unknown `space`).
- **`machine::Bus` is unchanged** (16-bit). The 68000 Core defines its own bus trait in its crate, with 24- or 32-bit addresses and byte, word and long accesses. This follows the guardrail "do not extract a shared component from two structurally different consumers" (the existing 8-bit buses and a 16-bit data bus differ in more than width: bus errors, odd-address faults, wait states per access size). The guardrail line "CPU-visible addresses remain `u16`" is replaced by "CPU-visible addresses are each Core's own; the Machine trait and the plugin protocol use `Addr`".

### Several CPUs

- `MachineProfile` gains `cpus: Vec<CpuInfo { id, name, family, address_space }>` (first entry is the primary CPU; a single-CPU Machine lists one).
- `Machine::step` and the raw `step`/`halted`/`pc` stay one call per Machine: the Machine schedules its own CPUs (existing Genesis decision: no widening of single-CPU Machines). `pc`/`halted` report the primary CPU.
- Debug commands that address a CPU (`GetRegisters`, `SetRegister`, `Set/Clear/ListBreakpoints`, `GetDisassembly`, `StepInstruction`, `StepOver`) gain `cpu: Option<String>`; `None` is the primary CPU, so existing callers and ZRCP are unaffected. `RegisterSnapshot` and breakpoint responses carry the CPU id.
- Co-processors that are not programmable CPUs (blitter, copper) appear as `ChipSnapshots`, as today.

### Time and sound

- `Machine::frame_rate() -> FrameRate { num: u32, den: u32 }`; `frame_hz()` stays as the rounded value for existing callers and for the pacing tests. The emu thread paces on the rational value.
- `Machine::audio_format() -> AudioFormat { channels: u8, sample_rate: u32 }`, default mono 44100, reported in the profile as well. `drain_audio` returns interleaved samples in that format. The Host resamples to the device (the resampler already exists) and mixes mono up to stereo.

### Video and input

- `Machine::pixel_aspect() -> (u16, u16)`, default `(1, 1)`, also in `MachineMetadata`. The Host applies it in `fit_screen` (integer scale on the corrected width).
- `MachineCommand::AxisEvent { axis: String, value: i16 }`; axis names come from `MachineKeyInfo`-style listing (`MachineProfile::axis_names`). The Host maps pad sticks and triggers through the same binding layers as buttons.

### Versions

`PROTOCOL_VERSION` 10 to 11 (protocol schema fingerprint re-recorded), `ABI_MINOR_V1` 1 to 2, `MACHINE_PROFILE_CONTRACT_VERSION` 1 to 2. Plugin version strings change with the family rebuild. `docs/plugin-abi.md` section 3 is the procedure.

## Work items, in order

1. `shared`: `Addr`, `FrameRate`, `AudioFormat`, `pc_wide`, protocol changes, profile `cpus`/`axis_names`, `CpuFamily::M68k`; update `abi.rs` layout test and `protocol_schema` fingerprint; update `docs/plugin-abi.md` and the machine trait docs.
2. `shared` cores and chips: no change expected (`Bus` stays); compile-fix only.
3. Stub plugin (`crates/plugins/test-machine-plugin`) and the host tests: new fields; a test that an old minor-1 library is refused with the documented message.
4. Host (`apps/retroclawd`): widen addresses in the debugger state, the disassembly window, memory viewer paging (4-digit hex assumed), bitmap explorer, `--dump-memory`, `--org`, `--capture-at`, `.pok`, symbol-table formatting (`$XXXX`/`0xXXXX` is 4 digits; make the width follow the address space), ZRCP (clamp to its 16-bit protocol for 8-bit Machines); rational pacing; audio format; aspect; axis events.
5. The three Family repositories: widen their `Machine` impls and plugin handlers at the boundary; report one CPU and mono 44100 / 1:1 aspect; rebuild; `tools/bump_pin.sh` moves all pins together.
6. Verification: protocol and ABI tests, `tools/smoke_test.sh` (all machines boot, boot screens equal to before), `rc-suite` unchanged results for every existing scenario (the change must be invisible to the current four Families).

## Status (2026-10-18)

Done and verified (shared and host tests, Family tests, smoke test of all seven current Machines through the real libraries):
- `Addr = u64` at the `Machine` and protocol boundary; `pc_wide`; protocol 11; profile contract 2; `OnCpu`, `AxisEvent`, `AudioPcm`, `CpuInfo`, exact frame rate, pixel aspect and audio format in the metadata; `CpuFamily::M68k`.
- Host: `HostedMachine` and the debugger state, the browser API and page (width-aware wrap and padding through `addr_bits`), the exact frame rate in the pacing loop and cycle budget, ZRCP kept 16-bit at its edge (a wider machine is an error there).
- A minor-1 library is refused by the minor-2 host with the documented message (test).
- A wrapped address on a narrow machine: `Machine` methods wrap to the machine's address space; the plugin handlers do not reject (the spec said they would; wrapping was chosen so nothing is dropped silently).

Consumed since 2026-10-19:
- Audio format: `HostedMachine::drain_audio` passes `AudioPcm` through interleaved; the audio queue holds the Machine's own
  `audio_format` (channels, sample rate) and `gui::audio::EmulatorAudioDecoder` resamples from that rate and plays stereo (a mono
  Machine on both sides, a stereo one kept, more channels: the first two). `--dump-audio` writes the Machine's channels and rate.
- Pixel aspect: `BaseResolution::display_size` widens the picture by it; the initial window size uses that shape (`fit_screen`
  stretches the texture to the window as before).
- `AxisEvent`: `[pads.N]` binds `left_x`/`left_y`/`right_x`/`right_y`/`left_trigger`/`right_trigger` to a Machine axis (`"-Name"`
  inverts, `"none"` unbinds); pad `n`'s left stick drives `Joy{n}X`/`Joy{n}Y` by default when reported. `PadRouter::route_axes`
  sends changes only (X right, Y down positive, -32767..=32767; triggers 0..=32767) through the emu thread's mouse/axis channel.

Not yet consumed by the Host (the ABI carries them; no current Machine needs them):
- A 68000 Core's symbol-table syntax and 24-bit operand widths in `machine::SymbolTable` and the debug-symbol tables; `--dump-memory`, `--org` and `.pok` stay 16-bit.
- The C128 reports one CPU (the active one) although it has two.

## Out of scope

No 68000 Core, no new Machine, no CD-ROM format, no interlace output, no rumble, no multi-window video. Those arrive with the first Machine that needs them, using the hooks above.

## Left to the builder

Exact names of the new types and fields, the bincode integer encoding check (varint) for the new widths, and the host UI layout for wider address columns.
