# Atari ST family: audit and design (Phase 0)

Status: **draft for owner approval**. No implementation code has been written. Date: 2026-10-19.

Brief: the pasted "Atari ST-family emulation" prompt (sections 1-31): ST/STF/STFM first, then Mega ST and STE, as
dynamically loaded machine plugins; plus MC68000 support in `rcasm`/`rcdis`/`rcld`, Atari executables, and floppy and
hard-disk tools.

## Claim labels

| Label | Meaning |
|---|---|
| **[R]** | Verified repository fact: read in the checked-out source on 2026-10-19 (path given) |
| **[H]** | Verified hardware fact: a source was read; the URL is given |
| **[H?]** | Hardware claim not yet checked against a primary source. It must be verified in the slice that needs it, and must never be coded from memory |
| **[F]** | Verified firmware fact: from the downloaded, hash-checked EmuTOS 1.4 artifacts |
| **[P]** | Proposal |
| **[O]** | Owner decision required |
| **[D]** | Deferred |

---

## 1. Executive summary

- **[P] New family repository `elpapimango/retroclawd-atari`** (submodule `machines/atari`), shaped like the three
  existing families. Deleting `atari_st_machine_plugin.{dll,so,dylib}` removes Atari support; the host gains no Atari code.
- **[R] Phase 1 needs no plugin ABI change.** Protocol 11 / ABI 1.2 already carries what Phase 1 needs:
  - `u64` addresses and `pc_wide`;
  - a CPU list, with `CpuFamily::M68k`;
  - exact frame rate, pixel aspect, and stereo/rate audio;
  - mouse, key names, two disk drives, and `MediaSlot::Other`.

  ABI or protocol work appears only later and is optional: memory watchpoints and an instruction trace (S8).
- **[R] Generic host fixes are needed (not ABI changes):**
  - `--program foo.prg` is hard-wired to the C64 `.prg` 2-byte header, so an Atari `.PRG` would be misread.
  - `--capture-at` reads only Z80 registers.
  - `SymbolTable` operand syntax is 16-bit.
  - `--org` and `--dump-memory` are 16-bit.
  - `--firmware auto` only knows RomM; it cannot fetch EmuTOS from SourceForge.
- **[F] EmuTOS 1.4 is pinned:**
  - Archive: `emutos-192k-1.4.zip` (ST/STf/Mega ST, plain 68000), SHA-1 `6f146a12…` as published by SourceForge.
  - Image: `etos192uk.img`, 196 608 bytes, reset PC `$00FC0030`.
  - The STE uses `emutos-256k-1.4.zip` (`etos256uk.img`, base `$E00000`).
- **[O] The 68000 core is the main owner decision.**
  - Recommendation: a new in-house core in `retroclawd-crates`, gated by SingleStepTests/680x0.
  - `m68k-rs` (MIT) would be used only as a differential oracle in dev-dependencies.
  - The alternative (adopting `m68k-rs` directly) is costed in section 7.
- **[P] Reuse instead of new chips:**
  - `ay-3-8910` (shared) already names YM2149 variants.
  - `wd1770` (retroclawd-commodore) already models a WD1772 against a rotating byte stream. It moves to `retroclawd-crates` because it gains a second consumer.
- **[P] Smallest Phase 1 slice (section 27):**
  - the dynamic `atari-st` plugin;
  - a 68000 core that passes the SingleStepTests groups EmuTOS's first instructions use;
  - a big-endian ST bus with the ROM mapped at `$FC0000` and its reset mirror at `$000000`;
  - a deterministic event scheduler with a VBL event;
  - EmuTOS fetched and verified by hash;
  - a headless run whose instruction-trace hash is identical across runs. It stops at the first unimplemented I/O register with a bounded diagnostic.

---

## 2. Verified repository audit

### 2.1 Plugin discovery and loading **[R]**

- **Where the host looks for plugins** (`apps/retroclawd/src/plugin_bridge.rs`): `RETROCLAWD_MACHINE_PLUGIN_DIRS`, else
  `<exe dir>/plugins`, `<exe dir>`, `./plugins`. `RETROCLAWD_MACHINE_PLUGIN(S)` names files explicitly.
- **Startup order:** the host first reads the machine id from the raw arguments. It then loads the libraries and
  reads each `MachineProfile`. Finally it merges the selected profile's options into clap and parses.
- **Library naming:** the implementation crate is `crates/plugins/<name>-plugin` (lib `<name>_plugin_impl`, exposes
  `entry_v1()`); the cdylib is `crates/plugin-dlls/<id>`, lib `<name>_plugin`, holding only
  `plugin_api::export_plugin!(…)`.
  - The library file is `<name>_plugin.dll` / `lib<name>_plugin.so` / `.dylib`. For example, the CPC library is `cpc_machine_plugin`.
  - Proposal: `atari_st_machine_plugin`.
- **Build and staging:** `tools/build_all.sh` builds every family listed as a submodule and stages its libraries and
  `<family>.firmware.toml` in `target/<profile>/plugins/`.

### 2.2 ABI, protocol and compatibility **[R]**

- **Versions:** `shared/crates/plugin-api/src/abi.rs` has `ABI_MAJOR_V1 = 1`, `ABI_MINOR_V1 = 2`.
  `protocol.rs` has `PROTOCOL_VERSION = 11`; the profile contract is 2.
- **Policy:** `shared/docs/plugin-abi.md`. The host refuses another major version before it reads anything else, and
  refuses an older minor version with a rebuild message.
- **Wire format:** bincode messages with a 256 MiB decode limit. A schema fingerprint test (`tests/protocol_schema.rs`)
  forces a version bump on any wire change.
- **Fast path:** raw `step` / `halted` / `pc` / `pc_wide` calls, with no serialization.
- **Panics:** contained by `plugin_api::guard`; a crashed plugin is held and reported once.

### 2.3 Interfaces **[R]**

| Interface | Where | Shape relevant to the ST |
|---|---|---|
| `machine::Machine` | `shared/crates/machine/src/lib.rs` | `clock_hz`, `frame_rate() -> FrameRate{num,den}`, `pixel_aspect`, `audio_format{channels,sample_rate}`, `step() -> u32` cycles, `pc() -> Addr(u64)`, `registers() -> Vec<(&str,u64)>`, `render_frame() -> (w,h,RGB)`, `read_memory/load/set_pc(Addr)`, `save_state/load_state`, `reset()` |
| `machine::Bus` | same | 16-bit, 8-bit data. **ADR 0003 says a 68000 core defines its own bus**; unchanged |
| Profile | `plugin-api/src/protocol.rs` `MachineProfile` | `machine_id`, `cpu_family`, `options: Vec<MachineOptionInfo>` (`Bool`, `Int`, `Text`, `Enum{choices}`, `File`, `FileSet`), `key_names`, `cpus: Vec<CpuInfo>`, `axis_names`, `media_kinds` |
| Media | `MediaKind{Tape,Disk,Cartridge,Snapshot,Program,RomSet,Other(String)}`, `MediaSlot{Disk{drive},Cartridge{slot},Other(String)}` | Floppies A/B fit `Disk{drive}`; ACSI images fit `Other("acsi0")`; the cartridge fits `Cartridge{slot:0}` |
| Input | `KeyEvent{key,pressed}`, `MouseMotion{dx:i8,dy:i8}`, `MouseButton`, `MouseWheel`, `AxisEvent` | Fits the IKBD: relative mouse, two buttons, keys by name |
| Debug | `DebugCommand::{Pause, Resume, StepInstruction, StepOver, Set/Clear/ListBreakpoints, Get/SetRegister(s), ReadMemory{len:u32}, WriteMemory, GetDisassembly, GetChipSnapshots, GetMachineMetadata, OnCpu{cpu,…}}`; `RegisterField{value:u64,width_bits}` | Covers D0-D7/A0-A7/PC/SR/USP/SSP. **No memory watchpoints in the protocol**: the host's "watches" are display expressions, and the only access breakpoints are I/O-port watches (`SetPortWatches`, an error on a machine with no port space) |
| Video | `render_frame` RGB24 plus `pixel_aspect` (consumed since 2026-10-19) | The ST draws its own border and picks its own canvas size |
| Audio | `drain_audio` interleaved `f32` in `audio_format`; the host resamples | YM2149 mono for now, DMA sound stereo later |
| Save states | `Machine::save_state() -> Vec<u8>`; families use serde + bincode v2 (`cpc/src/save_state.rs`) | The schema is the family's own |
| Headless | `--headless --frames N`, `--dump-screen`, `--dump-audio`, `--dump-state`, `--at FRAME:ACTION`, `--type`, `--capture-at ADDR:REG=FILE` (**Z80 registers only**) | `rc-suite`/`rc-scenarios` compare by hash (`exact`) or by `similarity` |

### 2.4 16-bit preparation already done **[R]**

Spec `docs/superpowers/specs/2026-10-18-abi-16bit-readiness-design.md` and ADR 0003. Listed there as not yet
consumed by the host:

- 24-bit operand widths in `machine::SymbolTable` and the debug-symbol tables;
- `--dump-memory`, `--org` and `.pok` (still 16-bit).

### 2.5 Options and firmware **[R]**

- **ROMs:** every ROM is a `MachineOptionKind::File` option (`rom`).
- **Option precedence:** CLI > game config `[options]` > `retroclawd.toml [machines.<id>]` > the plugin's default.
- **`--firmware auto`** (`apps/retroclawd/src/firmware.rs`) fills every `File` option still without a value. Inputs:
  - The `<family>.firmware.toml` `[[rom]]` entries: `machines`, `option`, `label`, `sha1 = [...]` or `parts = [...]`, and `when = {…}`.
  - Only the RomM server's `GET /api/firmware` list, matched by SHA-1. The download is verified against the hash it was chosen by.
- **No direct-URL firmware source exists.**
- **ROMs are never committed:** each family's `.gitignore` excludes `/roms/` and `/software/`.

### 2.6 Archive and URL inputs **[R]**

- **Shared input path:** every input path goes through `apps/retroclawd/src/inputs.rs` → `file_loader`:
  - an `http(s)://` source is downloaded;
  - a `.zip` is unpacked;
  - `archive.zip#entry` picks one entry.
- **So this already works today, with no hash check:**
  `--rom "https://downloads.sourceforge.net/project/emutos/emutos/1.4/emutos-192k-1.4.zip#etos192uk.img"`.
- **Downloads are cached in memory only:** the RomM cache is on disk.

### 2.7 Input, keyboard and gamepads **[R]**

- **Keys:** the host resolves positional and symbolic keyboard layouts, chords, pads and paste against the
  plugin's `key_names` and `chars`; it has no per-machine knowledge.
- **Joysticks:** a machine exposes them as `Joy1Up…`/`Joy1Fire1` key names, so the pads bind them by default.
- **Mouse:** reaches the plugin as relative `MouseMotion`.

### 2.8 Debugger, symbols, disassembly **[R]**

- **Disassembly** is asked from the plugin (`GetDisassembly`), which decodes with its CPU crate.
- **Single-instruction assembly** is chosen in the host by `CpuFamily` (`assemble_for`); there is no 68000 arm yet.
- **Symbols:** `machine::SymbolTable` annotates `$XXXX` / `0xXXXX` operands and substitutes names in assembler input.
  It needs 68000 operand syntax (`$XXXXXX`, `(d16,An)`, `#imm`).

### 2.9 Reusable crates **[R]**

| Crate | Repo | Reuse for the ST |
|---|---|---|
| `ay-3-8910` | retroclawd-crates `crates/chips/ay-3-8910` | YM2149: the crate header already names Yamaha's YM2149 variant; port A/B wiring is the machine's job |
| `wd1770` | retroclawd-commodore `crates/chips/wd1770` (930 lines) | WD1772: a command state machine over a `Floppy` trait (rotating MFM byte stream, index, track 0, write protect). Its timing is scaled by the CPU MHz passed to `new`. Datasheet-based, with VICE as a cross-check |
| `dsk` | retroclawd-crates | No (CPC EDSK) |
| `machine` | retroclawd-crates | `Addr`, `FrameRate`, `AudioFormat`, `SymbolTable` |
| `build-info` | retroclawd-crates | Plugin version tag |

### 2.10 retroclawd-tools **[R]**

- **`rcasm`** (`crates/rcasm/src/{asm,expr,cpu,text}.rs`):
  - Expressions are `i64`.
  - The location counter is `Option<u32>`, but `.org` refuses anything above 65 535 (`asm.rs:614`).
  - `cpu.rs` holds the 6502 and Z80 encoders.
- **`rcdis`:** `Options::new(cpu, org: u16)`; it decodes with the CPU crates and checks each line with rcasm.
- **`rcld`** is a placer/packager, not a relocating linker. It has no object files: the layout's sources are
  assembled together into one program, then placed and written as `.prg`/`.d64`/`.dsk`/`.tap`/`.crt`.
- **`rcdisk`:** a `Disk` trait with CBM and CP/M adapters.
- **Dependencies:** `rc-common` provides `Diag` and a no-overwrite `write_file`. The tools workspace pins
  `retroclawd-crates` at `a8cfb2ef…`, an older revision than the families' `10ba4a4`. Family format crates are pulled in
  by revision.

### 2.11 Exact change locations **[P]**

| Repository | Change |
|---|---|
| retroclawd-crates | New `crates/core/68000` (package `mc68000`); `wd1770` moved in from commodore; YM2149 levels in `ay-3-8910` (if not already present); new `mc6850` chip (see §10); `SymbolTable` 68000 operand syntax |
| retroclawd-atari (new) | `crates/machines/atari-st`, `crates/chips/{mc68901,st-shifter,st-glue-mmu,ikbd}`, `crates/formats/{st-floppy,msa,ahdi,gemdos-fat,atari-prg}`, plugin + cdylib, `firmware.toml`, manifests, `scenarios/` |
| retroclawd-commodore | Depend on the moved `wd1770` (pin bump) |
| retroclawd (host) | Submodule; `firmware.rs` URL source + hash + disk cache, and `when = { region = "ntsc" }` (today `when` only sees resolved machine options, not the host's `--region`) for O6; `--program` extension routing by machine; `--capture-at` register names from the plugin; 68000 arm in `assemble_for`; `--org`/`--dump-memory` widths |
| retroclawd-tools | `rcasm` 68000 target and 24/32-bit origin; `rcdis` 68000; `rcld` sections + Atari PRG writer; `rcdisk` ST/MSA + AHDI/GEMDOS adapters; pin bump |

### 2.12 A separate retroclawd-atari repository? **[P] Yes**

- **Precedent:** each family is its own repository (ADR 0001).
- **Requirement:** "Deleting an Atari plugin library must remove its machine support". This already holds per library;
  a separate repository also keeps Atari history and licensing (EmuTOS GPL-2.0+) apart.
- **Licence:** the existing repositories are GPL-3.0 [R], so the new one should be too.

### 2.13 Must the plugin ABI change? **[R]+[P] No, for Phases 1-4**

| Possible need | Covered by | Change? |
|---|---|---|
| 24-bit PC, 32-bit registers | `pc_wide`, `RegisterField{u64}` | No |
| Several chips | `GetChipSnapshots` | No |
| Hard disk | `MediaKind::Other("acsi")`, `MediaSlot::Other("acsi0")` | No |
| STE stereo DMA audio | `AudioFormat{2, rate}` | No |
| Jaguar pads (STE) | Key names `Joy1Pad*` | No |
| Memory watchpoints | Absent | **[D] Phase 2, optional**: a protocol 12 addition (`SetMemoryWatches`/`TakeMemoryHit`, mirroring the port pair). Alternative without a bump: keep watchpoints as plugin-side breakpoints on bus addresses, exposed through `ChipSnapshots`. Decide at Phase 2 |
| Instruction trace (S8) | Absent | **[D]** Same choice; Phase 1 uses a plugin-written trace file (option `trace_file`), so no ABI change |

---

## 3. Owner decisions (decided 2026-10-19)

| # | Decision | Owner's answer |
|---|---|---|
| O1 | 68000 core | **In-house `mc68000`** in retroclawd-crates; `m68k-rs` only as a dev-dependency test oracle (§7) |
| O2 | Family repository | **New public `elpapimango/retroclawd-atari`**, GPL-3.0, submodule `machines/atari` |
| O3 | EmuTOS delivery | **Download + cache**: `firmware.toml` URL source, archive and entry hashes checked, on-disk cache; nothing committed |
| O4 | SingleStepTests/680x0 (no licence file) | **Fetched at test time** at the pinned commit into a git-ignored folder; tests skip with a reason when absent; never committed |
| O5 | Reference emulator | **Hatari, latest stable**, installed into `Workspace\Emulators`; exact version pinned in the manifests; run as a black box only |
| O6 | Default firmware image | **`etos192uk.img` for `--region pal`, `etos192us.img` for `--region ntsc`** (a `when` condition on region) |
| O7 | ST/STF/STFM | **One id `atari-st` + `--model st|stf|stfm`** (default drive sides and metadata only); Mega ST and STE get their own ids |
| O8 | Defaults | **1 MB RAM, colour monitor** (`--ram`, `--monitor` change them) |

Everything else in this document is either a fact or a proposal the builder can settle.

---

## 4. Machine model matrix

Physical models vs emulator profiles. RAM, monitor and drive count are options, never separate machine ids.

| | ST (520ST) | STF (520/1040STF) | STFM | Mega ST (1/2/4) | STE (520/1040STE) |
|---|---|---|---|---|---|
| Proposed id | `atari-st` | `atari-st` (`--model stf`) | `atari-st` (`--model stfm`) | `atari-mega-st` | `atari-ste` |
| Phase | 1 | 1 | 1 | 4 | 4 |
| CPU | MC68000 ≈8 MHz [H?] | same | same | same | same |
| RAM (stock) | 512K [H?] | 512K / 1M [H?] | 512K / 1M | 1/2/4 MB [H?] | 512K / 1M, SIMM up to 4 MB [H?] |
| ROM | `$FC0000`, 192K [H, ST Internals "below 1.4"] | same | same | same | `$E00000`, 256K [H, ST Internals "1.4 and above"; F: etos256 PC `$E00030`] |
| EmuTOS 1.4 | `etos192*.img` [F: readme "ST / STf / Mega ST"] | same | same | same (192K) | `etos256*.img` [F: readme "STe / Mega STe"] |
| Original TOS (user-supplied) | 1.00, 1.02, 1.04 [H?] | same | same | 1.02, 1.04 [H?] | 1.06, 1.62, 2.05, 2.06 [H?] |
| PAL / NTSC | both | both | both | both | both |
| Monitors | colour (SC1224) or mono (SM124) | same | same + TV via the RF modulator | same | same |
| Video | low 320×200×16, medium 640×200×4, high 640×400×2 [H, ST Internals `$FF8260`]; 512-colour palette [H?] | same | same | same | + 4096 colours, fine scroll, `$FF820D` video-base low byte [H, ST Internals] |
| Standard chips | MFP 68901, 2× ACIA 6850, YM2149, WD1772, DMA, GLUE, MMU, Shifter, IKBD (HD6301) | same | same | + RTC, Blitter (standard), different keyboard housing [H?] | + Blitter, DMA sound, Microwire/LMC1992, enhanced joystick ports [H?] |
| Floppy | external [H?] | internal, single or double sided by sub-model [H?] | internal | internal | internal |
| Hard disk | ACSI | ACSI | ACSI | ACSI | ACSI |
| I/O | MIDI, RS-232, Centronics, cartridge, 2 joystick/mouse ports | same | same | same + internal bus | same + 2 enhanced ports |
| RTC | none (IKBD clock) | none | none | yes | none |
| Model-specific options | `ram`, `monitor`, `region`, `drives`, `drive_sides`, `accuracy` | same | same | + `rtc_mode` | + `blitter` |
| Unsupported (all models) | MegaSTE, TT, Falcon, 020+ CPUs, FPU, SCC, SCU, VME, TT-RAM, SCSI, IDE | | | | |

**[P] One machine id covers ST, STF and STFM**: the
differences are drive placement, sides and an RF modulator, none of which is visible to software. `--model st|stf|stfm`
only changes the default drive sides (single on the 520STF, double on the 1040STF and later [H?]). It is
reported in the metadata.

Mega ST and STE get their own ids, because their chipsets differ in ways software can see (RTC, Blitter, STE Shifter
and DMA sound, ROM base).

---

## 5. Hardware by phase

| Device | Phase 1 | Phase 2 | Phase 3 | Phase 4 |
|---|---|---|---|---|
| MC68000 | Full ISA, exceptions, address error, IRQ; cycle counts per instruction | Bus error from GLUE | Prefetch-exact, per-access `sync`, wait states | — |
| Bus/GLUE/MMU | RAM banks, ROM, reset mirror, I/O decode, supervisor-only I/O, bus errors | Cartridge, unmapped behaviour | Contention, 4-cycle alignment | Mega/STE differences |
| Shifter | Low/medium/high, palette, frame-based render | Raster palette (line-based), sync/res border foundations | Cycle-exact fetches, borders, Spec512 | STE Shifter |
| MFP 68901 | Timers A-D, GPIP, IRQ vectoring | USART (RS-232) | Exact timer phase | — |
| ACIA ×2 | Keyboard ACIA | MIDI ACIA | Bit-time exactness | — |
| IKBD | Keys, relative mouse, joystick 1 | Full command set, absolute mouse, joystick modes, clock | Byte timing | — |
| YM2149 | Tone/noise/envelope, port A (drive select/side), port B (Centronics) | Sample playback quality | Exact | — |
| DMA + WD1772 | Sector read via `.st` | Write, format, `.msa`, `.stx` | Rotational timing | — |
| ACSI | — | Yes | Timing | — |
| Printer / RS-232 / MIDI | — | Yes | — | — |
| RTC | — | — | — | Mega ST |
| Blitter | — | — | — | Yes |
| STE DMA sound, Microwire/LMC1992 | — | — | — | Yes |

---

## 6. Crate architecture **[P]**

General rules for every crate below:

- `#![forbid(unsafe_code)]` unless an entry says otherwise.
- Serde for state.
- No host, Bevy or OS I/O.
- Time comes only from the scheduler.

| Crate (path) | Responsibility | Public interface (sketch) | Dependencies | Timing ownership | Tests / bench / fuzz |
|---|---|---|---|---|---|
| `mc68000` (crates `core/68000`) | CPU core, disassembler, instruction table | `Cpu`, `trait M68kBus { read(addr:u32, size, fc) -> Result<u32,BusFault>; write(...); iack(level) -> IackResponse{Vector(u8), Autovector}; sync(clocks) }`, `Cpu::step(&mut bus) -> u32`, `Cpu::set_ipl(u8)`, `disassemble(bytes, pc)`, `OPCODES` table | `serde` | Returns cycles; calls `sync` per access in accurate mode | SingleStepTests gate; own tests for STOP/TRACE/IRQ/RESET; bench dispatch; fuzz `disassemble` |
| `wd1770` (moved to crates `chips/wd1770`) | WD1772 | Unchanged `Floppy` trait | `serde` | Scaled by the clock passed in | Existing tests move with it |
| `ay-3-8910` (crates) | YM2149 | Existing, plus the YM variant if missing | — | Existing | Existing + YM envelope tests |
| `mc6850` (crates `chips/mc6850`) | ACIA | `read(reg)`, `write(reg)`, `rx_byte(u8)`, `take_tx()`, `irq()`, `tick(clocks)` | `serde` | Bit-time counters from its clock | Datasheet register tests |
| `mc68901` (atari `chips/mc68901`) | MFP | Registers, `set_gpip(bit, level)`, `tick(mfp_clocks)`, `irq()`, `iack() -> vector` | `serde` | Its own clock domain (§8) | Datasheet tests; timer-phase tests |
| `st-shifter` (atari) | Video | Registers, `begin_line`/`end_line`, `render_line(ram)`, palette | — | Line events from the scheduler | Framebuffer hash tests |
| `st-glue-mmu` (atari, may be a module of the machine crate) | Decode, privilege, MMU banks, video counter, DMA address | — | — | — | Address-decoding tests |
| `ikbd` (atari) | HD6301 keyboard controller, behavioural (not a 6301 core) | Command parser, key matrix, mouse, joystick → byte stream | — | Byte pacing via the ACIA | Command tests from the IKBD protocol doc |
| `atari-st` (atari `machines/atari-st`) | Composition, scheduler, `Machine` impl | `AtariSt::new(Config)` | All of the above | Owns the scheduler | Bare-metal programs; EmuTOS milestones |
| `st-floppy` (atari `formats/`) | `.st` and `.msa` parse/write → sector model → MFM track bytes for the `Floppy` trait | `Image::parse(bytes) -> Result<Image, Error>` | — | — | Fuzz parse; round trip |
| `atari-prg` (atari `formats/`) | GEMDOS executable header, fixups, symbols | Parse, write, relocate | — | — | Fuzz; round trip |
| `ahdi` + `gemdos-fat` (atari `formats/`) | Partitions, BPB/FAT12/16 | Read-only by default | — | — | Fuzz; bounded traversal |
| Plugin + cdylib (atari) | Profile, options, key names, commands | `entry_v1` | `plugin-api` | — | Profile/key-table tests |

**Placement rule applied:**

- The 68000 core, `wd1770`, the YM2149 and the 6850 are shared: each is a generic chip with a second consumer either in
  sight or already present.
- The MFP, Shifter, GLUE and IKBD stay in the Atari repository until a second machine needs them.

---

## 7. MC68000: candidates and conformance

### 7.1 Candidates (inspected 2026-10-19: crates.io API, GitHub API, downloaded crate source)

| Candidate | Version / date | Licence | Size, unsafe | Bus model | Evidence | Fit |
|---|---|---|---|---|---|---|
| **`m68k`** (benletchford/m68k-rs) | 0.14.6, released 2026-10-15; pushed 2026-10-20; weekly releases | MIT | 63 k lines (68000-68060, FPU, MMU, Cranelift JIT behind a feature); `unsafe` in 11 files, including `core/execute.rs`, `mem_ops.rs`, `op_cache.rs`, `memory.rs` (fast-mem paths), so the "100% safe interpreter" README claim needs auditing | `AddressBus`: byte/word/long reads and writes, `try_*` variants returning `BusFault`, `sync(cpu_clocks)` before each access, IPL sampling hooks, IACK. **No function codes** on accesses | README claims all 261 894 SingleStepTests cases match cycles and bus-access totals (not re-run by us); Musashi test binaries | Strong functionally. Gaps: no FC (the ST GLUE needs supervisor/user per access for bus errors); scope far beyond the brief; rust-version 1.93 (we have 1.98.1, fine); fast API churn |
| `m68000` (Stovent) | 0.2.3, 2025-06-11 | MPL-2.0 (file-level copyleft; compatible with GPL-3 when used as a dependency) | 15.7 k lines, 0 `unsafe`; needs **nightly** (README) | `MemoryAccess` returning `Option` | No published test-suite results | Rejected: nightly, no conformance evidence |
| `r68k` (slippyex) | 0.2.2, 2026-01-28, 1 star | MIT | 16.5 k lines, 24 `unsafe`; a Musashi port | Musashi-style callbacks | Inherits Musashi | Rejected: weak maintenance evidence, Musashi-derived structure |
| Musashi / Moira (C / C++) | — | Musashi: custom permissive; Moira: no SPDX licence on GitHub | — | — | Mature | Rejected: not Rust, would need FFI + `unsafe` |
| **New `mc68000`** | — | GPL-3.0 (ours) | Target ~8-12 k lines, `forbid(unsafe)` | FC-aware `M68kBus` designed for our scheduler | Gated by SingleStepTests | **Recommended [O1]** |

### 7.2 Why recommend in-house (decision O1)

1. **Function codes and privilege per access.**
   - The ST GLUE raises a bus error for user-mode access to the I/O area and low memory [H?: to verify; commonly
     documented, behaviour TOS relies on for some protection].
   - `m68k-rs` would need an upstream change or a fork.
2. **Precedent.** `z80` and `mos6502` are in-house and validated by public suites. The tools (`rcdis`, the debugger)
   use the core crate's own `disassemble`, which we control.
3. **Scheduler fit.** The fast/accurate split is designed into the bus trait (`sync` only in accurate mode,
   monomorphised so fast mode pays nothing).
4. **Scope discipline.** No 020-060, FPU or MMU code enters the build.

Cost: the core is the largest single item of Phase 1, but it is mechanical and fully testable against SingleStepTests.

Mitigation: `m68k` (MIT) is a dev-dependency only, used for differential fuzzing (random instruction streams, compare
registers and memory), never linked into a plugin.

**Alternative accepted if the owner prefers speed to EmuTOS:**

- Wrap `m68k = "=0.14.x"` behind our `M68kBus` adapter.
- Accept that there are no user-mode bus errors until upstream supports FC.
- Audit its `unsafe` blocks.

The adapter keeps a later swap local to `atari-st`.

### 7.3 Conformance gate **[P]**

| Suite | Upstream | Version | Licence | Covers | Does not cover |
|---|---|---|---|---|---|
| SingleStepTests/680x0 `68000/v1` | github.com/SingleStepTests/680x0 | commit `e0d5ece9670205cc84a0101081837deb446f86a3` (2024-05-14, only commit) [R: GitHub API] | **none stated** (O4) | 125 files, ~8 000 tests each [R: README "a little over 1,000,000 tests"]; registers, SR, USP/SSP, the 2-word prefetch queue, RAM, total cycles and every bus transaction with function code and size, including TAS RMW [R: sample and README]. Address errors appear where pointers are odd (1% of cases) | STOP, ILLEGAL, LINE-A/F, interrupts, trace (no files for them [R: directory listing]); DTACK always immediate |
| Own tests (`mc68000/tests/`) | ours | — | GPL-3.0 | STOP, RESET instruction (124-cycle reset line), TRACE, IRQ priority/masking, autovector vs vectored IACK, spurious interrupt, double bus fault (halt), group 0 frames for address/bus errors, privilege violations, CHK/TRAPV/DIV0 frames | — |
| Differential vs `m68k` | crates.io `m68k` 0.14.6 | pinned | MIT | Random streams | — |

**Gate rules:**

- Phase 1 requires 100% of SingleStepTests on functional state.
- Cycle totals and bus-transaction order must match for every instruction; transaction order is used in Phase 3
  accurate mode.
- Expected failures are listed by test name, with a reason, in a `BASELINE` file (the CPC acid suite's convention).
- EmuTOS boot is never a substitute for any of this.

### 7.4 Instruction metadata **[P]**

- **One declarative table** `OPCODES: &[OpPattern { mask, match, mnemonic, size, ea_src: EaSet, ea_dst: EaSet,
  timing: TimingClass }]` in `mc68000`.
- **Uses:**
  1. At start-up (a `OnceLock`), it builds the 65 536-entry dispatch table of function pointers, so there is no
     generated code and no build script.
  2. `disassemble`.
  3. rcasm's encoder: the operand-form legality check and the opcode bits.
- **The hot path** is a table index plus a monomorphised handler; the metadata is not consulted per instruction.
- **Staleness test:** every one of the 65 536 words either decodes to exactly one pattern or is illegal. A test asserts
  no pattern overlap and checks the illegal set against SingleStepTests' `map` folder.

---

## 8. Master clock and scheduler **[P]**

- **Master unit: one CPU clock cycle (`u64`).** Every CPU-synchronous device is an exact integer division of the CPU
  clock **[H? to verify from the Atari hardware spec]**:
  - YM2149 = CPU/4 (2 MHz) [search result: maidavale.org says 2 MHz];
  - WD1772 = CPU (8 MHz);
  - ACIA = CPU/16 (500 kHz);
  - Shifter line = 512 CPU cycles at 50 Hz and 508 at 60 Hz;
  - mono line = 224 CPU cycles.
- **The CPU clock itself** is the master crystal ÷ 4. Forum sources give:
  - PAL: 32.084988 MHz;
  - NTSC: 32.0424 or 32.04245 MHz (atariage forum thread, unverified; the two figures disagree in the last digit, so
    both are shown here and the hardware spec must decide).

  `clock_hz` and `frame_rate` are derived from these as exact rationals.
- **The MFP has its own crystal** (2.4576 MHz [H?]), unrelated to the CPU clock. Timers run in an MFP-clock domain.
  The conversion is integer and exact: `mfp_ticks = (cpu_cycles * MFP_NUM + rem) / MFP_DEN`, with the remainder
  carried (Bresenham). This means no float accumulation and no drift; `u64` cycles overflow after ~73 000 years.
- **Event queue:** a binary heap keyed `(time: u64, class: u8, seq: u64)`.
  - `class` fixes the order of simultaneous events: video line → MFP timer → ACIA bit → FDC byte → DMA → CPU interrupt
    sampling.
  - `seq` keeps insertion order. The result is deterministic, with no hash maps in the event path.
- **Fast mode:** the CPU runs whole instructions while `now < next_event`. An instruction that crosses an event runs to
  completion; the event is then processed at its own timestamp before the next instruction boundary. This matches the
  68000's own visibility, since interrupts are sampled only at instruction boundaries [H?].
  - Device registers read mid-instruction are brought up to date first: each device has `catch_up(now)`, called by the
    bus on access, so no device state is ever stale when the CPU reads it. This is not "run past and retroactively
    apply": the device advances to the access's exact time.
- **Accurate mode:** the bus `sync(clocks)` hook advances `now` per access, and devices are caught up per bus cycle. On
  top of that come Shifter fetch slots and 4-cycle bus alignment (Phase 3). The device interfaces are the same; only
  the granularity of `catch_up` calls changes.
- **STOP:** `now` jumps to `next_event` (no busy loop); identical in both modes.
- **DMA and Blitter ownership:** a bus-request event stalls the CPU by the granted cycles. Phase 1 has no DMA bus stall
  (fast mode charges it as a lump at transfer completion). Phase 3 grants per word.
- **Wall clock:** none. The RTC and IKBD clock read host time only in the optional real-time RTC mode (§18); in tests
  the seed and start time are fixed.

---

## 9. Bus, memory, reset, endianness **[P] unless marked**

- **Memory map** (Phase 1 targets; each row is verified in the slice that implements it):

  | Range | Contents |
  |---|---|
  | `$000000-$000007` | ROM mirror during reset [H?] |
  | `$000000-$3FFFFF` | RAM by MMU bank configuration (`$FF8001` [H, ST Internals]) |
  | `$FA0000-$FBFFFF` | Cartridge [H, ST Internals] |
  | `$FC0000-$FEFFFF` | 192K ROM [H, ST Internals; F: header] |
  | `$FF8000-$FFFFFF` | I/O [H, ST Internals register list] |

  The 24-bit address bus wraps. RAM below `$800` and all I/O are supervisor-only (bus error) [H?].
- **Bus implementation:** a `match` on the high address byte into region handlers.
  - RAM is `Vec<u8>` stored in big-endian order: word reads are `u16::from_be_bytes`, with no host alignment
    assumptions and no pointer casts.
  - Odd word/long access is an address error raised by the core, before the bus.
- **Watchpoints:** a `debug_watch: bool` checked once per access in a cold branch. Fast mode pays one predictable
  branch, nothing else.
- **Endianness everywhere:** ROM files are stored big-endian as the CPU sees them [F: the first word `602E` is `BRA.S`
  `+$2E`, landing on `$30`]. The following are explicit `from_be_bytes`/`to_be_bytes` with bounds checks:
  - `.PRG` headers;
  - BPB fields (little-endian! FAT/BPB are Intel order even on the ST [H?]);
  - AHDI root sector (big-endian [H?]);
  - save states;
  - debugger memory.
- **Reset semantics:**

  | Event | CPU | RAM | Devices |
  |---|---|---|---|
  | Power-on | SSP/PC from `$0`/`$4` (ROM mirror) | Deterministic fill pattern, recorded seed (default all `$00`) [O-free: tests pin it] | Datasheet power-on states |
  | Cold reset (host F12 / `Machine::reset`) | Same | Kept (as today for other machines) | Hardware RESET line states |
  | `RESET` instruction | Unaffected; 124-cycle reset line [H?] | Kept | External devices reset (MFP, ACIA, YM, FDC, Shifter?), each per its datasheet; listed per device in the slice |
  | State restore | Whole state replaced | — | — |

  Values a datasheet leaves unspecified are recorded as such and set to a documented constant, never tuned to make
  EmuTOS boot.

---

## 10. ACIA topology

- **Two MC6850 ACIAs** [H, ST Internals]:
  - keyboard at `$FFFC00` (control/status) and `$FFFC02` (data);
  - MIDI at `$FFFC04` and `$FFFC06`.
- **Shared interrupt:** both IRQ outputs share one MFP interrupt source. ST Internals lists MFP priority 6 as "Keyboard
  and MIDI (ACIA)", vector `$118` "ACIA keyboard processor" [H].
- **[H?] To verify before coding:**
  - the shared GPIP bit (I4);
  - the 500 kHz clock and the divide ratios (keyboard ÷64 → 7812.5 baud, MIDI ÷16 → 31250 baud);
  - the reset state.
- **Implementation [P]:** two instances of shared `mc6850::Acia`. The keyboard instance is wired to the `ikbd` model
  (both directions); the MIDI instance to a MIDI port (Phase 2: loopback, or a file sink and source). The IRQ lines are
  ORed in the machine crate.

---

## 11. Fast/accurate design **[P]**

- **Option:** a plugin-declared `accuracy` option (`MachineOptionKind::Enum { choices: ["fast","accurate"] }`, default
  `fast`), giving `--accuracy fast|accurate`. Verified: no host flag named `accuracy` exists [R: `apps/retroclawd/src/main.rs`].
- **One device implementation each:**
  - The mode selects the CPU bus type parameter: `FastBus` has an empty `sync`; `AccurateBus` has `sync = advance +
    catch_up + contention`.
  - It also selects the Shifter renderer (frame or line-batch vs per-fetch).
- **Fast-mode coalescing is allowed only where the output is identical**, for example:
  - drawing whole lines when no palette or Shifter register was written during the line (write-tracked);
  - running the YM2149 in sample-sized blocks.
- **Mode switching** is allowed only at reset or state load; the state records the mode.

---

## 12. Video, audio, input **[P]**

**Video tiers:**

1. Frame/line rendering correct for GEM and most games (Phase 1 is line-based with no mid-line effects).
2. Raster palette and border foundations (Phase 2).
3. Cycle-exact fetches (Phase 3).

**Canvas:** one fixed canvas per monitor type (colour: an overscan-capable canvas with border; mono: 640×400), so the
host never sees surprise resizes. Medium resolution is drawn 640 wide, with low resolution pixel-doubled. A frame is
hashable byte for byte for `exact` goldens.

**Pixel aspect is reported:**

- low resolution: `(1,1)` on a 2× canvas;
- [H?] PAL ST pixel aspect needs a source before anything other than `(1,1)` is reported.

**Audio:**

- YM2149 → mono `f32` at 44 100 Hz, through the existing deterministic sample-accumulator pattern (cycle-driven, as the
  CPC/Spectrum do).
- STE DMA sound → stereo; `audio_format` switches to 2 channels for `atari-ste` only.
- Headless `--dump-audio` works unchanged.

**Input by phase:**

| Phase | Input |
|---|---|
| 1 | Keyboard by name (ST scan-code table from the IKBD documentation), relative mouse, joystick 1 (`Joy1Up…Fire1`) |
| 2 | Joystick 0 vs mouse modes, absolute mouse, IKBD commands; MIDI/RS-232/printer as file or loopback sinks; cartridge |
| 4 | STE enhanced ports, Jaguar pad key names |

Paste works through `chars` in the key table. Nothing in the plugin touches Bevy or the OS.

---

## 13. Floppy formats and geometry

**Layers:**

1. image parser (`st-floppy`);
2. sector model (`Track { sectors: Vec<Sector{c,h,r,n,data}> }`);
3. MFM track-byte synthesis for `wd1770::Floppy`;
4. filesystem (`gemdos-fat`, tools only);
5. WD1772;
6. drive mechanics (the `Floppy` impl: rotation, index, step);
7. persistence (atomic write-back on eject or exit, reusing the host's `DiskExitPolicy`).

| Format | Provenance to read | Representation | Phase | Notes |
|---|---|---|---|---|
| `.st` | De-facto raw sector dump (no spec document; geometry from the boot sector BPB) [H?] | Sectors only | 1 read, 2 write | **Geometry:** use the BPB (SPT, sides, total sectors) if it is self-consistent with the file size; otherwise fall back to known size table entries; if both fail or disagree → error naming both, never guess |
| `.msa` | Magic Shadow Archiver format (header: ID `$0E0F`, SPT, sides, start/end track, RLE `$E5` marker) [H? from memory: verify against a published description] | Sectors, RLE per track | 2 | Bounded decompression (track ≤ SPT×512) |
| `.dim` | FastCopy Pro image | Sectors + header | 2 [D] | Header layout to research |
| `.stx` | Pasti (protection: timing, fuzzy bits, sector ID tricks) | Track + timing | 2-3 | Documented by reverse engineering only; licence of the format docs to check |
| `.ipf` | SPS/CAPS | Flux-ish streams | [D] | Needs the closed or restrictive CAPS library or a clean reimplementation; out of the near phases |
| `.raw`, `.ctr` | Raw flux / CTRaw | Flux | [D] | — |

**Robustness:** each parser has a size cap (≤ 2 MiB for sector formats), checked arithmetic, no panics, and a fuzz
target.

---

## 14. ACSI, partitions, filesystems (Phase 2) **[P]**

**Layers:**

1. ACSI protocol through the DMA chip (command bytes via `$FF8604` with A1 low/high, DMA block transfers) [H, ST
   Internals for the registers; protocol to be sourced from the Atari ACSI/DMA documentation];
2. target/device (ID 0-7, SCSI-1-like group-0 commands: TEST UNIT READY, REQUEST SENSE, READ(6), WRITE(6), MODE SENSE,
   INQUIRY, FORMAT) [H?];
3. block backend (`trait BlockDevice { read(lba, &mut [u8]); write; len }`);
4. image (raw `.img`/`.hd`);
5. partitions: AHDI root sector (big-endian partition entries `GEM`/`BGM`/`XGM` with flag, id, start, size) [H?];
6. filesystem: GEMDOS FAT12/FAT16 with Atari logical sector sizes (BPB `BPS` larger than 512 on `BGM`) [H?].

**Read-only default:** `rcdisk` refuses to write without `--write` or `-o`. Writes go to a temp file and are renamed.

**Limits:** loop detection on extended `XGM` chains (a visited set plus a depth cap), path sanitising on extract (no
`..`, no absolute paths), extraction size caps.

IDE is out of scope.

---

## 15. rcasm / rcdis / rcld / executables **[P]**

- **rcasm:**
  - **Target:** `--cpu 68000`.
  - **Syntax:** Motorola. `.b/.w/.l` sizes; `d0-d7/a0-a7/sp/sr/ccr/usp/pc`; all 12 addressing modes; `bra.s`/`bra.w`
    explicit or automatically chosen. An *ambiguous* short/long choice in pass 1 is an error unless a size is given or
    `.opt autobranch` is enabled — never resolved silently.
  - **Directives:** `dc.b/w/l`, `ds`, `even`, `section text|data|bss`, `include`, `incbin`.
  - **Location counter:** widened to `u32` with `.org` limited by the target (24-bit for 68000).
  - **Encoder:** driven by the `mc68000::OPCODES` table.
- **rcdis:**
  - **Origin:** widened to `u32`.
  - **Output:** canonical rcasm syntax; branch targets as labels; illegal or truncated → `dc.w`.
  - **Round trip:** bytes → rcdis → rcasm → identical bytes, tested on random words plus the EmuTOS image (an in-test
    download, not committed).
- **rcld:**
  - **Sections:** text, data and bss in the layout.
  - **Relocation:** for a GEMDOS `.PRG`, the assembler tracks which long values are section-relative (relocatable);
    rcld emits the fixup table. No object files are needed, because rcld already assembles everything as one program.
- **Atari `.PRG`/`.TOS`/`.TTP` format:**
  - **Header** (28 bytes, big-endian [H?]): `$601A` magic, text/data/bss/symbol sizes, reserved, program flags,
    absolute flag.
  - **Fixups:** first long offset, then byte deltas, with `1` = +254 and `0` = end [H?].
  - **Symbols:** DRI symbol table, 14-byte entries [H?].
  - **Sources to read:** the Atari GEMDOS reference, the Atari Compendium, the TOS Hitchhiker's Guide.
  - **Tests:** fuzz the parser; round trip write→parse; the debugger loads DRI symbols as a `SymbolTable` source.
- **Host collision [R]:** `resolve_program_load` treats any `.prg` as C64 (2-byte load address).
  - **Fix [P]:** the host passes `.prg` to the plugin as `MediaKind::Program` when the selected profile declares
    `Program`, and keeps the C64 header path only for profiles that ask for it.
  - **Running a GEMDOS program** needs TOS; Phase 2 does it by writing the file onto a generated boot floppy (`AUTO`
    folder) — no GEMDOS trap emulation [D].

---

## 16. Debugger and bring-up diagnostics **[P]**

- **Registers:** `RegisterField` D0-D7, A0-A7 (A7 = the active SP), PC (24-bit), SR (with a T/S/IPL/XNZVC breakdown),
  USP, SSP.
- **Disassembly:** `GetDisassembly` through `mc68000::disassemble`, plus a host `assemble_for(M68k)` arm.
- **Breakpoints:** PC breakpoints (existing protocol).
- **Watchpoints:** §2.13.
- **Chip snapshots:** MFP, ACIA ×2, YM, FDC/DMA, Shifter, GLUE/MMU, and scheduler (next events).
- **Bring-up ring buffers** (bounded, plugin side, written by `trace_file`/`diag_file` options and on a fatal stop):
  - the last 256 PCs and opcodes;
  - the last 64 exceptions (vector, PC, SR, fault address, width, direction, FC);
  - the last 64 unimplemented I/O accesses (address, size, value, PC, time).
- **Rule:** never special-case an EmuTOS PC; a fault is fixed in the hardware model.

---

## 17. Plugin/ABI integration **[P]**

| Profile field | Value |
|---|---|
| `machine_id` | `atari-st` |
| `cpu_family` | `M68k` |
| `cpus` | `[{main, "MC68000"}]` |
| `capabilities` | Video, Audio, Disk, SaveState, Debug |
| `media_kinds` | Disk (Phase 1); Cartridge, `Other("acsi")`, Program (Phase 2) |

**Options:**

| Option | Kind |
|---|---|
| `rom` | File, required, fillable by `--firmware auto` |
| `ram` | Enum `256k,512k,1m,2m,2.5m,4m` (supported MMU combinations [H?]), default `1m` |
| `monitor` | Enum `color,mono`, default `color` |
| `model` | Enum `st,stf,stfm`, default `stfm` |
| `drives` | Int 1-2 |
| `drive_sides` | Enum `1,2` |
| `accuracy` | Enum `fast,accurate`, default `fast` |
| `trace_file`, `diag_file` | Text |

Region comes from the host's `--region pal|ntsc`.

The fast path is `step`/`halted`/`pc_wide`. The metadata carries `frame_rate` exact, `pixel_aspect` and the audio format.

---

## 18. EmuTOS acquisition and hash provenance

**Verified [F] 2026-10-19:**

- **Listing:** `https://sourceforge.net/projects/emutos/files/emutos/1.4/`. Download URL used:
  `https://downloads.sourceforge.net/project/emutos/emutos/1.4/<file>`.
- **Upstream hashes:** SourceForge's file listing publishes SHA-1 and MD5 per file. Our downloads matched them:

| Artifact | Size | SHA-1 (SourceForge-published, matched) | SHA-256 (project-pinned, local) |
|---|---|---|---|
| `emutos-192k-1.4.zip` | 2 260 230 | `6f146a126065f0671a1df3d9a273c01f962e9d06` | `59abac06a2d29b0864c5a7cfb2af65f022c337aed34188e174a9a08cc737e4bc` |
| `emutos-256k-1.4.zip` | 2 933 355 | `3cd44f8a60658859761d86170a0f17a294eba31c` | `aadd90cf0c99925d3f2943149dd51ee4deb6015aefe22ade4a5e6c04fb6f2e9d` |

| Entry | Size | SHA-1 (project-pinned) | SHA-256 (project-pinned) | Header facts |
|---|---|---|---|---|
| `emutos-192k-1.4/etos192uk.img` | 196 608 | `904ea9dc09ff8286f6e0272c12255dfe85beb17a` | `2d6f3f6f…edf0f3` | `602E 0104`, reset PC `$00FC0030`, os_beg `$00FC0000`, conf word `$0007` |
| `emutos-192k-1.4/etos192us.img` | 196 608 | `c03ae6379c88b31303dd6ebe63087054f716cba0` | `8fbbf8b4…244cfc` | conf word `$0000` |
| `emutos-256k-1.4/etos256uk.img` | 262 144 | `b9dba5346cdd237fe4ba8d89d04bc4d82e4eec72` | `3bfdc561…b0aeb0` | `602E 0206`, PC `$00E00030`, base `$00E00000` |
| `emutos-256k-1.4/etos256us.img` | 262 144 | `2e0a0160d14e5340913b330aa7fe817adcf62365` | `f1fe6836…0ed75b3` | |

**Further firmware facts:**

- **Compatibility:** the readme in the zip says 192K is for "ST / STf / Mega ST … only work with a plain 68000 CPU"
  and has no hardware autodetection; 256K is for "STe / Mega STe". Each image holds one language; `us` is NTSC and the
  others are PAL.
- **Header word:** the PAL/NTSC flag lives in the header word at offset `$1C` (`$0007` uk vs `$0000` us). This is
  consistent with "bit 0 = PAL", but the field meaning is [H?] until read in EmuTOS's `doc/version.txt` (source
  archive).
- **Licence:** GPL-2.0 (`doc/license.txt` in the zip; the "or later" wording to confirm in the source headers).
- **Hash provenance wording:**
  - The archive SHA-1s are SourceForge-published hashes (hosting-platform integrity, not an EmuTOS signature).
  - Every SHA-256 and every entry hash is **project-pinned**: an integrity identifier established by this download on
    2026-10-19, not upstream-published. Changing one requires an intentional review.

**Delivery [P, O3 option (a)]:**

1. Extend `firmware.toml` with an optional source per entry:
   `url = "...emutos-192k-1.4.zip"`, `archive_sha1 = "6f14…"`, `entry = "emutos-192k-1.4/etos192uk.img"`, and the
   entry's `sha1` as today.
2. `--firmware auto` tries RomM first, then the URL. It verifies the archive hash, then the entry hash, and caches the
   verified entry on disk in the existing RomM cache folder, keyed by SHA-1.
3. Offline with a valid cache → works. Network failure with no cache → a clear "firmware unavailable" error, which
   tests report as skipped-with-reason, not a regression.

Proprietary TOS: user-supplied through `--rom`, never fetched, bundled or required.

---

## 19. Save states **[P]**

- **Envelope:** big-endian magic `RCST`, then:
  - schema version;
  - machine id;
  - plugin `version_tag`;
  - `mc68000` state version;
  - accuracy mode;
  - firmware SHA-1;
  - per-drive media SHA-1 + dirty flag;
  - payload length;
  - CRC32;
  - then the bincode v2 payload (the families' convention).
- **On load, refuse:**
  - another machine id;
  - a newer schema;
  - an over-limit length (64 MiB cap);
  - a bad CRC;
  - a firmware hash mismatch (clear error naming both hashes).
- **Migration:** older schema numbers migrate by an explicit chain.
- **Media:** a media hash mismatch loads the state but warns, since the media is the user's. Modified (dirty) disk
  contents are stored in the state, as the CPC/C64 do.

---

## 20. Security review **[P]**

- **Untrusted inputs:** every external byte stream: ROM, `.st`/`.msa`/`.stx`, PRG, AHDI images, save states, symbol
  files, assembler sources and includes.
- **Parser rules:**
  - checked arithmetic and slice `get`;
  - caps on size, track count and RLE output;
  - recursion and chain depth caps;
  - no `unwrap` on input.
- **Writing:**
  - writes are atomic (temp + rename);
  - tools are read-only unless asked;
  - extraction refuses path traversal.
- **Fuzzing:** `cargo fuzz` targets for every parser, plus `mc68000::disassemble`.
- **Logs:** no firmware or media bytes in logs or CI artifacts (hashes and addresses only).
- **`unsafe`:** none planned. Any proposed block needs its reason, invariant and protecting test recorded in the crate
  docs.

---

## 21. Testing and benchmarking **[P]**

**Test tiers:**

1. **CPU:** SingleStepTests, own tests, differential.
2. **Chip:** datasheet-derived unit tests per crate.
3. **Bare-metal machine:** small 68000 programs assembled by `rcasm`, run without TOS, writing results to RAM or the
   screen. They cover framebuffer/palette, MFP timers, PSG, ACIA/IKBD, floppy/DMA, and later ACSI, Blitter and STE.
4. **EmuTOS milestones A-F:** each is a headless rc-suite case with a hash checkpoint.
5. **External compatibility:** demos and games, Phase 3+, not gating.

**On CI failure, write:**

- expected/actual frame hashes and the PNG;
- the configuration;
- firmware and media hashes;
- the accuracy mode;
- emulated time;
- the bring-up ring buffers.

**Benchmarks:**

- **Harness:** `criterion`-free, using the existing `cargo bench` style of each repository (to be checked when the
  first bench is added).
- **Measured:** CPU dispatch (MIPS on a NOP/ALU mix), bus, scheduler events/s, Shifter line, YM block, full frames/s on
  idle GEM.
- **Reported:** the host CPU, `rustc 1.98.1`, the release profile, median and variance over 10 runs, and fast vs
  accurate.

---

## 22. Licensing policy **[P]**

| Source | Licence | Use |
|---|---|---|
| Hatari | GPL-2.0+ | Black-box reference emulator only; no code, tables or comments copied |
| ASE | to check (not yet read) | Reference reading only |
| EmuTOS | GPL-2.0 (+?) | Firmware fetched or bundled with a source pointer; never code copied into emulator crates |
| `m68k` | MIT | Dev-dependency oracle only |
| SingleStepTests/680x0 | none stated | Fetched at test time, never committed (O4) |

Our repositories are GPL-3.0. GPL-2.0-only material could not be mixed into code; nothing is being mixed.

Every external item gets a `CREDITS.md` line with its version or commit.

---

## 23. Dependency-ordered backlog **[P]**

1. **S1:** repository + plugin skeleton + firmware URL source (host) + `mc68000` core passing SingleStepTests on the
   instruction groups below → reset execution (milestones A, B).
2. Remaining SingleStepTests groups to 100%, plus own exception tests.
3. GLUE/MMU decode, RAM banks, supervisor checks, I/O diagnostics.
4. Scheduler line/VBL/HBL events, Shifter frame render (C, D-video).
5. MFP (timers, GPIP, vectoring).
6. ACIA + IKBD (keyboard, mouse) (D-input).
7. YM2149 wiring (sound, port A drive select).
8. `wd1770` move to shared; DMA; `.st` image (F).
9. Determinism checkpoint + manifest S1/S2/S4/S5/S6 vs Hatari (E). **Phase 1 exit.**
10. rcasm/rcdis 68000 (can start in parallel after 2, since it shares the opcode table).
11. Phase 2 items in §5 order, then rcld/PRG, rcdisk, ACSI.
12. Phase 3 timing.
13. Phase 4 machines.

---

## 24. Phase exit criteria

As the brief's §28. Concretely for Phase 1:

- SingleStepTests 100% functional and cycle totals (expected-failure list empty, or each entry justified).
- Own exception, STOP and IRQ tests green.
- The EmuTOS desktop reached headless, with an identical frame hash on 3 repeated runs.
- Keyboard, mouse and joystick verified by headless `--at key_down` scenarios.
- PSG tone scenario (S4).
- `.st` read: a purpose-built floppy (made by our tools) lists in the desktop.
- Failure diagnostics demonstrated by an injected fault test.
- `tools/check_all.sh` includes the Atari family and passes.

---

## 25. Explicit deferrals

| Item | Deferred to |
|---|---|
| `.ipf`, `.raw`, `.ctr` | No phase planned |
| `.dim` | Late Phase 2 |
| Memory-watchpoint protocol addition | Phase 2 decision |
| S8 trace kind | After the plugin trace file proves useful |
| GEMDOS trap or hard-disk folder emulation | Not planned; floppy injection instead |
| MIDI to a real host port | Not planned; files and loopback only |
| Absolute-mouse integration with the host cursor | Phase 2+ |
| Everything in the brief's §5 out-of-scope list | — |

---

## 26. Risks and unresolved questions

1. **[H?] hardware facts.** Every [H?] fact needs a primary source.
   - The Atari Corp *ST Hardware Specification* (1986) PDF on cpcwiki is behind a Cloudflare challenge from this
     machine; the owner may download it manually.
   - The MC68000 PRM / user's manual, the MC68901, MC6850 and WD1772 datasheets, and the IKBD protocol document must be
     fetched.
   - ST Internals (atarimuseum transcription) is secondary and visibly garbled in places. It lists both `$064` and
     `$068` as "HBL"; the real level is to be confirmed from the 68000 autovector table plus the hardware spec.
2. **SingleStepTests licensing** (O4) and their timing assumption (immediate DTACK) — the ST adds wait states in
   accurate mode.
3. **Core effort** (O1) is the schedule risk; the oracle-backed approach bounds the correctness risk.
4. **`.stx` / Pasti documentation provenance** and licence.
5. **Host `.prg` collision** must land before anyone tests Atari programs through `--program`.
6. **retroclawd-tools pins an older `retroclawd-crates`** (`a8cfb2ef`); a bump comes before the 68000 tool work.

---

## 27. Smallest recommended Phase 1 slice ("S1: reset to first I/O")

**Repositories:** retroclawd-crates, retroclawd-atari (new), retroclawd (host).

**Acceptance criteria:**

1. **Plugin loading.**
   - `retroclawd --list-plugins` shows `atari-st` from `atari_st_machine_plugin`.
   - Removing the file removes the machine.
   - The CPC, C64 and Spectrum smoke tests are unchanged.
2. **Core.** `mc68000` passes 100% of SingleStepTests for the groups EmuTOS's reset path uses before its first I/O
   access. The group list comes from a trace, not a guess; expected groups include MOVE, MOVEA, LEA, CMP, Bcc, BSR, RTS,
   MOVEtoSR, ORItoSR, ANDItoSR, MOVEM, CLR, TST, SUB/ADD, LSL/LSR, JMP and JSR. Each group's pass count is recorded.
3. **Reset.**
   - After power-on, SSP is ROM long 0 and PC is ROM long 1 (`$00FC0030`), read through the reset mirror. Unit test
     with a synthetic 192K ROM, not EmuTOS.
   - The first instruction is fetched at `$FC0030` in supervisor mode with IPL mask 7. This is a 68000 reset fact
     [H?: from the M68000 PRM, to cite in the test].
4. **Big-endian bus.** Unit tests for byte, word and long reads and writes on RAM and ROM:
   - a word at `$FC0000` reads `$602E`;
   - odd word access → address error exception with a group 0 frame;
   - a ROM write is ignored or bus-errors per the GLUE fact (to be sourced; until sourced, logged and ignored, marked
     [H?] in code).
5. **Scheduler.**
   - A VBL event at exactly one frame period (a rational CPU-cycle count from the region) raises IPL 4, autovectored.
   - A test runs 1 000 frames and checks the total cycles equal `frames × cycles_per_frame` exactly.
6. **Firmware.**
   - `retroclawd atari-st --firmware auto` fetches `emutos-192k-1.4.zip` from SourceForge, checks SHA-1 `6f146a12…`,
     extracts `etos192uk.img`, checks SHA-1 `904ea9dc…`, and caches it.
   - A second run works offline.
   - A tampered cache entry is refused.
7. **Early execution and determinism.**
   - `retroclawd atari-st --headless --frames 5 --rom <cached> --trace-file t.txt` (plugin option `trace_file`) writes
     PCs.
   - Two runs give the same SHA-256 trace hash.
   - The run reaches the first access to an unimplemented I/O register and stops with the bounded diagnostic block (PC,
     opcode, registers, fault address/size/direction, emulated time, last 256 PCs).
   - The first 1 000 PCs are compared against a Hatari `--trace cpu_disasm` run of the same image (milestone B
     evidence; needs O5; otherwise recorded as a known gap).

**Not in S1:** MFP, Shifter, ACIA, YM, FDC (each I/O access is a logged stop), save states (a stub returning an error),
tools.

---

## Sources consulted (2026-10-19)

- EmuTOS 1.4 files and published hashes: https://sourceforge.net/projects/emutos/files/emutos/1.4/ (listing JSON
  `net.sf.files`); readme and licence inside the zips.
- SingleStepTests/680x0: https://github.com/SingleStepTests/680x0 and `68000/v1/README.md`; GitHub API (licence: none;
  commit `e0d5ece`).
- `m68k`: https://github.com/benletchford/m68k-rs, crate 0.14.6 source from crates.io. `m68000`:
  https://github.com/Stovent/m68000, crate 0.2.3. `r68k`: https://github.com/slippyex/r68k, crate 0.2.2.
- The Atari ST Internals (transcription): https://atarimuseum.ctrl-alt-rees.com/computers/16bits/STINTERN.htm — the
  memory map, vector table, MFP priorities, ACIA, PSG, Shifter, DMA registers. Secondary source.
- Clock values (unverified, forum): https://forums.atariage.com/topic/353761-mystery-of-the-sts-weird-clock-oscillator-frequency-choices;
  YM 2 MHz: https://maidavale.org/blog/ay-and-ym-clock-speeds/.
- Primary spec to obtain: https://www.cpcwiki.eu/imgs/b/b1/Atari_ST_-_Hardware_Specification.pdf (Cloudflare-blocked
  from this host).
- Not yet read: Hatari manual and docs, the ASE wiki and repository, the EmuTOS manual, Motorola and WD datasheets.
  These are required reading inside the slices that need them, and are listed in §26.

## Status (2026-10-20): slice 1 done

- **CPU**: `mc68000` (retroclawd-crates): full 68000 instruction set, exceptions (group 0/1/2), interrupts, STOP,
  trace, double fault; SingleStepTests 68000/v1 996 322 / 1 000 060 exact including clock counts (3 736 ASR cases kept
  to the PRM, 2 bad upstream cases, issue #4); bus-transaction order exact in 573 973 / 991 995 (Phase 3 work);
  differential against `m68k` 0.14.6 on 20 000 exception-free random instructions; disassembler checked against the
  core for every opcode's length.
- **Machine**: `atari-st` plugin (retroclawd-atari), bus, scheduler, diagnostics, trace; EmuTOS 1.4 stops at
  `$FF8001` after 3 instructions, identical across runs and equal to Hatari 2.6.1's first PCs.
- **Host**: `--firmware auto` URL source and cache; submodule and scripts.
- **[H?] that became [H]**: `$0-$7` mirror the ROM, user-mode bus errors below `$800` and on I/O, supervisor bus
  errors on nonexistent memory (TOS documentation 3.5); reset sequence and clock counts (UM 6.3.1, Table 8-14);
  exception frames (UM 6.2-6.3); PAL frame 512 x 313 clocks and 8.021247 MHz (two atari-forum threads).
- **Still [H?]**: NTSC crystal, RAM above the configured top, ROM and `$0-$7` writes, the empty cartridge port.
- **Deviations**: see the slice ledger summary in the final report (rulings).

## Status (2026-10-20): slice 2 done

Plan `docs/superpowers/plans/2026-10-20-atari-st-slice2.md`.

- **Devices**: MMU (`$FF8001`, row/column bank mapping), Shifter (`st-shifter`, frame-based render), MFP (`mc68901`,
  from the Motorola datasheet), HBL/VBL/MFP interrupts, exact MFP clock domain.
- **Decode policy**: `io.rs` lists every I/O address with a fact; any other address stops the machine ("no hardware
  fact"). Hatari-observed (`[O]`) rows cover single addresses only.
- **EmuTOS 1.4**: runs to its first YM2149 write; all 106 I/O accesses equal Hatari 2.6.1's; the RAM sizing writes
  Hatari's `$FF8001` values for all six `--ram` sizes; the first 5000 PCs agree.
- **Facts now sourced**: MFP register map, reset, interrupts and timers `[D]` (MC68901 datasheet, 1984); `$FF8001`,
  Shifter registers, GPIP wiring `[H]` (*Atari ST Internals*, Abacus 1985); interrupt levels and MFP channels `[H]`
  (TOS documentation); MFP clock 2.4576 MHz `[H]` (ST-Log 34).
- **Still open**: see `machines/atari/docs/hardware-facts.md` (`[H?]`), and TODO.md "Atari ST: next slices".
