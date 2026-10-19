# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repo.

## Project status

The `retroclawd` application plus the integration tooling. **No machine and no shared crate lives in this
repository.** Machines are shared libraries (`*_machine_plugin.so/.dll/.dylib`) loaded by the app through the
C-ABI in `plugin-api` (policy: `shared/docs/plugin-abi.md`). Four other repositories hold everything else; all are
git submodules here, and the three machine repositories depend on the shared crates by git revision:

| Repository (submodule path) | Holds |
|---|---|
| `elpapimango/retroclawd-crates` (`shared/`) | `machine`, `plugin-api`, the `z80` and `mos6502` cores, `ay-3-8910`, `nec765`, `dsk`, `zx-tape`, the plugin ABI policy. No machine, no app code. |
| `elpapimango/retroclawd-sinclair` (`machines/sinclair`) | ZX81, ZX Spectrum 48K/128K/+2/+2A/+3, snapshot formats `.z80/.sna/.szx/.p`, Sinclair keyboard |
| `elpapimango/retroclawd-amstrad` (`machines/amstrad`) | CPC464/664/6128, CPC+ / GX4000, `.cpr` cartridges, SNA snapshots, i8255 |
| `elpapimango/retroclawd-commodore` (`machines/commodore`) | C64, C128, 1541/1570/1571/1581, VIC-II, SID, CIAs, VIA, WD1770, VDC, CBM disk formats |

Stays here: the app (`apps/retroclawd`, including the debuggers `debug-ui`, `debug-zrcp`, `debug-log`: ordinary
modules behind cargo features, no longer plugins), the app-only crates (`app-config`, `game-config`, `file-loader`, `pok`), `crates/plugins/test-machine-plugin` (a
do-nothing machine used only by this repository's unit tests) and `tools/`. The app uses `shared/` by path; the
families pin a `retroclawd-crates` commit (`rev = ...`), and `tools/bump_pin.sh` moves all of them together.

The detailed per-crate notes are in each repository's `CLAUDE.md`; the pre-split status narrative is kept in
`docs/project-status-before-split.md`.

Everything at once: `tools/build_all.sh` / `tools\build_all.ps1` (checks out `shared/` and the families, builds the
app and every family, stages the libraries in `target/<profile>/plugins/`); `tools/smoke_test.sh` boots every machine
and compares the boot screens. Using one machine on its own: build the family repository (`cargo build --release
--workspace`) and put the resulting libraries in a `plugins/` folder next to the `retroclawd` executable, or point
`RETROCLAWD_MACHINE_PLUGIN_DIRS` at them. `retroclawd --list-plugins` shows what was found.

## Scenario manifests (2026-10-18)

Every machine has a `machines/<family>/<id>.manifest.toml` (zx81, zx-spectrum, zx-spectrum-128, zx-spectrum-2a3, cpc464/664/6128, c64,
c128) holding the playbook's standard scenarios as data, run by `rc-scenarios` of `retroclawd-tools` against the **release** `retroclawd`
and its plugins. References: ZEsarUX 13.0 and VICE 3.10 (the CPC Plus and GX4000 have none). Build the test media first
(`machines/<family>/scenarios/build.sh`, output in the git-ignored `software/scenarios/`), then:

```
RETROCLAWD_BIN=target/release/retroclawd RETROCLAWD_MACHINE_PLUGIN_DIRS=target/release/plugins RC_ROMS=machines   RC_GOLDENS=<folder with the goldens> rc-scenarios run machines/sinclair/zx81.manifest.toml
```

Goldens are never committed (a folder of your own, or RomM by hash); `retroclawd-tools/scripts/capture_*.py` regenerate them. Known gaps
(`status = "known-gap"`, evidence under `machines/<family>/docs/`): the C64's fast loader skipping SEARCHING/LOADING; the CPC
diagnostics' CRTC/FDC lines; the C64 dead test's clock. The S6 frame-length gaps were closed 2026-10-19: each machine reports its exact
frame through `Machine::frame_rate` / `MachineMetadata::frame_rate` (ZX81 312 x 207 T-states, its line corrected from 208 to 207; CPC
79872; C64/C128 one raster frame, 19656 PAL), and the host paces and budgets by it. Details and the compare options are in `retroclawd-tools/CLAUDE.md`.

## What this is

Zilog Z80 CPU emulator/core in Rust, prioritizing accuracy (correct
instruction semantics, flag behavior, undocumented opcodes, cycle timing) +
speed (efficient dispatch, minimal allocation in hot execution loop), plus
machine-level plumbing to run core as real computer.

## Commands (standard Rust/Cargo)

- `cargo build` / `cargo build --release` — build
- `cargo test` — run all tests
- `cargo test <name>` — run single test by name substring
- `cargo run -p retroclawd -- zx81 --rom <path>` — opens live Bevy window (default mode)
- `cargo run -p retroclawd -- zx81 --rom <path> --headless` — batch mode: run to completion, print stats, exit
- `cargo clippy -- -D warnings` — lint
- `cargo fmt` — format

## Crate layout

Machines and shared crates live in separate repositories (see "Project status"); this repository keeps the app.

```
apps/
  retroclawd/       # binary: CLI + Bevy GUI; links NO machine code, loads machine libraries from disk
crates/
  formats/
    game-config/    # per-game .toml config
    app-config/     # persistent settings .toml
    file-loader/    # zip / URL inputs, temp-dir extraction
    pok/            # Spectrum .POK trainer files
  plugins/
    test-machine-plugin/   # do-nothing machine for this repository's unit tests only
shared/             # submodule: retroclawd-crates
  crates/machine/        # CPU-agnostic plumbing: Machine trait, Clock, MemoryBus, Bus (everything depends on it)
  crates/plugin-api/     # C-ABI plugin boundary (policy: docs/plugin-abi.md)
  crates/core/{z80,6502}/            # the CPU cores (6502 is package `mos6502`)
  crates/chips/{ay-3-8910,nec765}/   # chips shared by sinclair + amstrad
  crates/formats/{zx-tape,dsk}/      # tape and disk-image formats shared by sinclair + amstrad
machines/           # submodules: the machine families (same directory layout as before the split)
  sinclair/   machines/{zx81,zx-spectrum}, formats/{zx81,z80,sna,szx}-snapshot, chips/sinclair-keyboard, plugins + plugin-dlls
  amstrad/    machines/cpc, formats/{cpc-cartridge,cpc-snapshot}, chips/i8255, plugin + plugin-dll
  commodore/  machines/{c64,c128,1541,drive-link}, formats/{c64-tape,c64-cartridge,d64,d71,d81,g64,cbm-dos},
              chips/{sid,vic-ii,via-6522,wd1770,cia-6526,acia-6551,cs8900a,vdc-8563}, plugins + plugin-dlls
```

Grouped by role (not left flat) so "where does X go" stays self-answering
as more machines/cores/chips/formats get added later (e.g. `machines/cpc`,
`chips/z80-pio`) — same reasoning as the `crates/` vs `apps/` split below,
one level deeper. `machine` alone stays directly under `crates/`, not
inside any group, since every other crate (regardless of group) depends
on it.

Dependency arrows point one way: `machine` has no path deps (CPU-agnostic
leaf), `z80`/`mos6502` both depend on `machine` and nothing else,
`sinclair-keyboard` has no path deps either (pure matrix logic, doesn't
depend on `machine`), the two machine crates depend on
`machine`/`z80`/`sinclair-keyboard`, `retroclawd` depends on
`machine`/`zx81`/`zx-spectrum`, never touches `z80` directly. Keep it
that way. This was the actual first real test of "non-Z80 CPU crate later
must not require `machine` know about any CPU": `mos6502` needed exactly
one change to `machine` (see `machine`'s own `Bus` notes below) and it's
a backward-compatible default-method addition, not a redesign. Library
crates live under `crates/`; binary lives under `apps/` — keep split as
more apps (e.g. future GUI) added, not mixed back into `crates/`.
`z80-snapshot`/`zx81-snapshot` sit next to `z80` — deliberately no path dep
on `machine` or either machine crate (see their own doc comments): a
snapshot-format crate only parses file bytes into a plain data struct
(`z80-snapshot` depends on `z80` since `.z80` fully encodes register state,
so it hands back a real `z80::Cpu`; `zx81-snapshot` needs no dep at all,
since neither ZX81 format encodes registers). *Applying* a parsed snapshot
onto a running machine (`ZxSpectrum::load_snapshot`, `Zx81::
load_p_snapshot`) is each machine crate's own inherent method, same
reasoning as `set_border`/keyboard mapping below — keeps the format crates
reusable by some future machine w/o dragging this machine's specifics
along. Tape/floppy formats later should follow the same split once their
actual shape is known — no shared `Loader` trait yet since 2 data points
isn't enough to know its real shape, and inventing one now would be
guessing.

- **`game-config`** — parses per-game `.toml` config files: `parse(text) ->
  Result<GameConfig, toml::de::Error>`, `GameConfig` = `machine`/`rom`/
  `region` (raw `Option<String>` -- turning these into real
  `MachineKind`/`RegionArg` is `retroclawd`'s own job, not this crate's) +
  `media: Option<String>` (one path field, not a tagged union -- kind
  inferred from the path's own extension, exactly like `--snapshot`/
  `--tape`/`--disk` already do) + `tape_speed: Option<u32>`.
  `#[serde(deny_unknown_fields)]` so a typo'd key is a clear parse error,
  not silently ignored -- same "error clearly, don't best-effort"
  convention as `UnsupportedHardwareMode`/`Missing2A3PagingByte`. No
  dependency on `machine`, `clap`, or either concrete machine crate --
  same format-crate-stays-pure split as `z80-snapshot`/`zx81-snapshot`.
- **`retroclawd`** (`apps/retroclawd`) — `clap`-derived arg parsing, size
  validation, then either live Bevy window (default) or batch run
  (`--headless`). Zero args at all (e.g. double-clicked on Windows, no
  console attached) prints `--help`, exits `0` rather than erroring on a
  missing machine -- deliberate, bare double-click shouldn't look like a
  crash. Any actual bad invocation (missing machine/ROM, unknown value,
  batch-only flag without `--headless`, etc.) = real clap error: exits
  `2`.

  **CLI syntax is `retroclawd <machine> [options]`** -- `machine` is a
  positional `CommonArgs` field (`#[arg(index = 1)]`), not a `--machine`
  flag (removed entirely, clean cutover, no alias). Two-phase parse
  (`main()`'s own top section): `prescan_machine_arg`/
  `prescan_plugin_values` read raw argv directly (no clap yet) to find the
  candidate machine id and any `--plugin machine=<path>` overrides --
  needed because which machine was selected decides which plugin-declared
  options the real `Command` must merge in, and an explicit `--plugin`
  path may be what makes that machine discoverable at all. `build_command`
  starts from `CommonArgs::command()` (the derive-generated base, common
  flags only) and appends one `clap::Arg` per that machine's declared
  `MachineOptionInfo` (`machine_option_arg`) before the real
  `cmd.get_matches_from(normalized_args)` parse -- `retroclawd cpc6128
  --help` therefore shows CPC's own options merged with the common ones,
  and `retroclawd cpc464 --ram-banks 2` is a clean clap "unexpected
  argument" error (not a hardcoded runtime rejection) since `cpc464`'s
  profile never declares `ram_banks` in the first place. Needs `clap`'s
  `"string"` feature explicitly (`Id`/`Str`'s `From<String>` impls are
  feature-gated) -- dynamically-built `Arg`s can't use `&'static str`.

  **Machine-owned options** (`plugin_api::MachineOptionInfo`/
  `MachineOptionKind`, declared per-`MachineProfile` not per-plugin, since
  one plugin crate commonly serves several `machine_id`s with different
  applicable options -- e.g. `ram_banks`/`rom_slot` only on
  `cpc6128`/`cpc6128plus`, never `gx4000`; `turbosound` only on
  `zx-spectrum-128`/`zx-spectrum-2a3`, never plain `zx-spectrum`): every
  ROM path (`rom`, or C64's `rom_kernal`/`rom_basic`/`rom_char`) is a
  `MachineOptionKind::File` option now, read from disk by the host into a
  `BinaryBlob` keyed by the option's own `key`; CPC's upper-ROM slots
  (`--rom-slot N=path`, repeatable) are `FileSet { min_index: 1,
  max_index: 15 }`, keyed `"{key}{N}"`. Scalar options (CPC's `tape_mode`/
  `ram_banks`/`run`, ZX Spectrum's `speaker_type`/`joystick_type`/
  `fast_tape_load`/`turbosound`) are `Bool`/`Int{min,max}`/`Text`/
  `Enum{choices}`, sent to the plugin as `NamedValue`s in
  `MachineCreateRequest::options` at creation time -- the old
  post-creation `MachineCommand::SetSpeakerType`/`SetJoystickType`/
  `SetTurbosound`/`SetFastTapeLoad` variants (and their
  `plugin_bridge.rs`/`HostSpeakerType`/`HostJoystickType` plumbing) are
  deleted entirely, since none of them were ever called more than once,
  immediately after creation -- one channel, not two. `--run`'s own
  machine gating used to be a hardcoded `cpc_machine_id(machine_id:
  &str) -> bool` string-match function; now it's simply whether the
  selected machine's profile declares a `run` key at all (CPC profiles
  with `has_internal_rom()`, and now `c64` too, unconditionally -- see
  `c64`'s own auto-run paragraph above) -- the function is gone, the
  flag itself doesn't exist in a machine's parsed `Command` whose
  profile never declares it. `resolve_auto_run_profile` (`main.rs`)
  picks the actual `gui::AutoRunProfile` from whichever media is
  attached (`--program`/`--tape`/`--disk`/`--game-config`'s `media`, in
  that precedence order, erroring if more than one is given) purely by
  file extension -- `.cdt`/`.tzx`→`CpcTape`, `.dsk`/`.edsk`→`CpcDisk`,
  `.d64`→`C64Load`, `.prg`/`.t64`→`C64Run`; a real-time `.tap` on
  `--machine c64` is a clear, explicit error (no cheap way to detect
  when a real-time load has finished, unlike the near-instant `.d64`
  fast-loader path) rather than silently doing nothing. `gui::input`'s
  `AutoRunState`/`auto_run_actions` (previously CPC-only) now build a
  `VecDeque<(AutoRunKey, bool)>` instead of raw `KeyCode` pairs --
  `AutoRunKey::Host(KeyCode)` (resolved through `KeyRouter::plain_names`, the same bindings real
  keyboard input uses) for every existing CPC action unchanged, plus
  `AutoRunKey::Named(&'static str)` for a stable machine-side key name
  with no natural host key at all (the C64's own dedicated `Asterisk`
  key). Both C64 profiles also gained a readiness wait before typing
  anything (`auto_run_ready`/`auto_run_needs_readiness_wait`, generalizing
  the wait mechanism `CpcTape` already had), needed after a live GUI run
  under software rendering showed early keystrokes getting silently
  dropped before real BASIC's own input loop was actually polling yet --
  see `c64`'s own auto-run paragraph above for the full story and the
  live end-to-end verification.

  Resolution precedence for every declared option (`resolve_one_machine_
  option`/`resolve_machine_options`): CLI flag > `--game-config`'s own
  `[options]` toml table > `retroclawd.toml`'s `[machines.<id>]` section
  (options flat beside the host settings) > the plugin's own declared default. A `Bool`-kind CLI flag can
  only ever force `true` (a bare switch, same limitation the old
  hardcoded `bool` fields already had) -- `false` has to come from toml or
  the default. `game_config::GameConfig` has a `options: HashMap<String,
  toml::Value>` field (`[options]` table, `[serde(default)]`) and
  `app_config::MachineSettings` flattens the same map (`#[serde(flatten)]`)
  so a machine's options sit **directly in `[machines.<id>]`**, beside the
  typed host settings (`shader`, `shader_intensity`, `scanline_intensity`,
  `fullscreen`, `vsync`, `max_fps`; no nested `options` table -- a leftover
  `[machines.<id>.options]` is a clear error saying so). serde cannot combine
  `deny_unknown_fields` with `flatten`, so for `app-config` every non-host key
  lands in the map and the app rejects strays: `validate_app_config` refuses any
  such key under `[default]`, and the selected machine's own keys are validated
  against its declared schema at runtime
  (`validate_declared_option_keys`), producing "unknown option \"x\" ...
  (available: a, b, c, <host settings>)" rather than a bare parse error --
  deliberately chosen over keeping closed typed fields, since a closed field
  would mean every new plugin-declared option needs an `app-config`/`game-config`
  crate change, defeating the entire point of options living in the
  plugin. `game_config::GameConfig` lost its own dedicated `rom: Option
  <String>` field entirely (folded into the generic `options` table, same
  shape as every other plugin-declared option) -- a real breaking format
  change, not additive; `[options] rom = "..."` replaces the old bare
  top-level `rom = "..."`. A `FileSet` option's toml shape is a nested
  slot-number table (`[machines.cpc6128.rom_slot] 7 =
  "amsdos.rom"`), not a single value, mirroring the CLI's own repeatable
  `--rom-slot N=path`.

  GX4000 accepts raw 16K cartridge banks or a `.cpr` RIFF/AMS! container
  as its `rom` option value; a game-config `media` path ending in `.cpr`
  still gets promoted into that same slot automatically
  (`promote_gx4000_cpr_media`, unchanged mechanism, just adapted to read/
  write the new `ResolvedMachineOptions.rom_paths` shape instead of the
  old flat `Resolved.rom_blob_paths` list). Every classic/Plus CPC with
  internal firmware validates its combined dump size inside the plugin
  itself (CPC464/464Plus 32K, CPC664/6128/6128Plus 48K) -- unchanged,
  just now reached via the generic `File`-option round trip instead of a
  hardcoded `--rom` flag. `--snapshot *.sna` / `--dump-snapshot *.sna`
  still route CPC SNA v1/v2/v3 through the plugin; Plus ASIC fields still
  live only in the internal `.state` format. `--disk` (common) still
  accepts standard/Extended DSK on Spectrum +2A/+3, CPC664, CPC6128, and
  6128Plus (CPC464/464Plus/GX4000 still reject it clearly), and now also
  accepts a real `.d64`/`.d71`/`.d81` for `--machine c64` -- serviced by
  `C64::try_fast_kernal_load_trap` via `AttachedDisk`, see `c64`'s own disk
  paragraph and `d71`/`d81`'s own bullet above.
  `--tape` (common) still accepts `.tap`/`.tzx`/`.cdt`/`.pzx`/`.csw`/
  `.voc` for ZX Spectrum/CPC, and now also C64's own, differently-shaped
  `.tap` plus `.t64`.

  Every remaining common (host-owned, not plugin-declared) flag keeps its
  old shape: `--region <pal|ntsc|japan>` (default `pal`), `--headless`
  (batch mode: run to completion in terminal instead of opening window),
  `--dump-memory 0xSTART:LEN` (alias `-dm`), `--max-cycles N` (alias
  `-mc`; stop after N instructions even if never halted), `--dump-screen
  <path>` (writes `HostedMachine::render_frame()`'s output as PNG),
  `--dump-snapshot <path>` (format by extension -- `.z80`/`.sna` the
  save-side counterpart to `--snapshot`), `--dump-state <path>` (this
  project's own internal `.state` format, the save-side counterpart to
  `--load-state`) -- these 5 all `requires = "headless"` in clap, only
  make sense for a finite batch run; `--program <path>` (a plain flag now,
  not a bare positional -- index 1 is the machine now) w/ `--org 0xNNNN`
  (load address, default `0x4000`) works both modes, loads a standalone
  test program instead of booting the ROM, `conflicts_with_all =
  ["snapshot", "tape", "load_state"]` same as before. A `.prg` path
  (case-insensitive, first piece of the C64/1541 media track's item 3,
  see `TODO.md`) is auto-detected by `resolve_program_load`: its own real
  leading 2-byte little-endian load address is parsed and used instead of
  `--org`, and only the bytes after that header get loaded -- every other
  extension keeps loading verbatim at `--org`, unchanged. Fully
  machine-agnostic (`--program` never was C64-specific) and needed no new
  plugin command -- the exact same generic `Machine::load`/`set_pc` calls
  every other `--program` file already went through still drive it, just
  with a file-derived address instead of the flag's. Deliberately doesn't
  patch BASIC's own pointers or inject a `RUN` keystroke -- a real KERNAL
  LOAD loads a `.prg` into memory at its own address and stops there too;
  actually running it (`RUN`/`SYS`) is still the user's own next step, not
  simulated. A `.prg` shorter than 2 bytes (too short to hold a real load
  address) is a clean error, not a silent zero-address load. `--snapshot <path>`
  loads a snapshot file instead of booting the ROM (still
  `conflicts_with` `--program` via that same list); format picked by file
  extension, validated against the selected machine in
  `HostedMachine::apply_snapshot` (`.z80`/`.sna`/`.szx` for
  `zx-spectrum*`, `.p`/`.p81`/`.81` only for `zx81`) -- a mismatch is a
  real runtime error, not a silent best-effort. `--load-state <path>`
  (`conflicts_with = "snapshot"`) is `--snapshot`'s save-state counterpart,
  applied after `--tape`/`--disk` so a save state's tape-position/FDC
  restore has an already-installed tape/disk to restore onto. `--tape
  <path>` (`conflicts_with = "snapshot"`) loads a tape image via
  `HostedMachine::apply_tape`; media stays installed and advances in real
  time rather than a replace-state-once one -- except `--machine c64`'s
  own `.t64`, which has no real-time signal to advance at all (see `c64`'s
  own tape paragraph above) and is poked into memory once instead.
  `--tape-speed <N>`
  (`Option<u32>`, defaults to `1` inside `resolve()`) is the turbo
  multiplier, common across every tape-capable machine regardless of
  family (a no-op for C64's `.t64`, which isn't played back at all).

  `--game-config <path>` loads a per-game `.toml` file (`game-config`
  crate, `GameConfig`) that can supply `machine`/`region`/`media`/
  `tape_speed` plus the `[options]` table described above --
  `--no-border`/`--fullscreen`, `--debug*`/`--dump-*`, `--max-cycles`,
  `--org` stay CLI-only, deliberately not toml-configurable. Layered, not
  mutual exclusion: any of `machine`/`region`/`media`/`tape_speed` given
  directly on the CLI overrides the toml's value for that setting. The
  merge itself is a small pure function, `resolve(args: &CommonArgs,
  config: &GameConfig) -> Result<Resolved, String>` (its own unit tests,
  no file IO/clap involved) -- `Resolved` no longer carries ROM paths at
  all (that's `resolve_machine_options`'s job now, using the selected
  machine's declared schema plus `--game-config`'s `options` and
  `retroclawd.toml`'s per-machine `options`); `machine`/`region` prefer
  `args`, fall back to `config` (`region` defaults to `pal` if neither
  gives it); `media` is `config.media` unless an explicit
  `--snapshot`/`--tape`/`--disk` is given, in which case it's fully
  replaced by that; `tape_speed` prefers `args.tape_speed`, falls back to
  `config.tape_speed`, defaults to `1`. `resolved.media` (when present) is
  read and dispatched through `HostedMachine::apply_media(extension,
  bytes, tape_speed)` -- extension-sniffs the path (`.z80`/`.sna`/`.szx`/
  `.p`/`.p81`/`.81` → `apply_snapshot`, `.tap`/`.tzx`/`.cdt`/`.pzx`/`.csw`/
  `.voc`/`.t64` → `apply_tape`, `.dsk`/`.edsk` → `apply_disk`) so one toml
  field covers what the CLI itself spreads across three flags. A standalone
  `--program` together with `--game-config` is a manual runtime check in
  `main()` (clap's `conflicts_with_all` can't see into a value merged from
  a toml file, unlike the `--snapshot`/`--tape` conflicts above which
  clap itself enforces).
  `--debug` (stdout) / `--debug-file <path>`
  (`apps/retroclawd/src/debug_log.rs`, `--debug-file` implies `--debug`)
  log keyboard events and HALT transitions, off by default. `--show-overlay`
  (`-so`, legacy alias `--debug-info`; GUI only, `conflicts_with =
  "headless"`) shows a semi-transparent overlay, redrawn every
  `DEBUG_OVERLAY_INTERVAL` (500ms). Takes an optional colon-delimited panel
  list (`gui::debug_overlay::OverlayPanels`, clap `num_args = 0..=1` +
  `default_missing_value = "cpu"` so a bare flag means `cpu`-only):
  `cpu` (FPS/register/HALT -- the 6502 view renders A/P/PC/S/X/Y at native
  widths plus `NV-BDIZC`, uppercase=set/lowercase=clear/`-`=unused bit 5;
  other CPU register layouts retain the generic two-column view), `mem`/
  `memory` (16-byte hex window at PC, read the same per-byte-round-trip
  way the debugger's own memory viewer already does, plus a machine's
  paging/bank summary line when it has one), `disk` (FDC phase + motor,
  +2A/+3 Spectrum and CPC664/6128), `tape` (cassette motor, playback
  progress `pos/total (%)`, T-states until the next EAR-line edge
  `edge_in`, a numeric list of the most recent completed pulse widths
  in T-states -- compare against the standard ROM protocol's own
  known-good values, pilot ~2168T/sync ~667/735T/bit0 ~855T/bit1
  ~1710T, all cited in `cpc::bus`/`zx_spectrum::bus`'s own doc comments
  -- plus an ASCII EAR-bit-history waveform bar, `0`/`1`, sampled at a
  fixed cycle interval so consecutive characters represent equal real
  time and a run's length is directly comparable to a real pulse's
  width, unlike a naive once-per-CPU-instruction sample rate), or `all`
  for every panel. FPS itself is always shown regardless of which panels are picked.
  A panel with nothing to report on the current machine (e.g. `disk`/
  `tape` on ZX81/C64) renders `n/a`, never omitted or an error -- only an
  unrecognized panel *name* is a clean pre-window-open `Err`. Wire
  protocol: every machine plugin may expose a generic `chip_id: "overlay"`
  `ChipSnapshot` (`memory_summary`/`fdc_phase`/`disk_motor`/`tape_motor`/
  `tape_position`/`tape_total`/`tape_edge_cycles` fields + `tape_waveform`/
  `tape_pulse_widths` blobs, the latter little-endian `u16` pairs decoded
  by `plugin_bridge::chip_blob_u16_pairs`) via the existing
  `GetChipSnapshots` debug command -- deliberately separate from CPC's own
  pre-existing `"cpc.overlay"` chip (which stays CPC-specific, still only
  feeding the `--debug` diagnostic log line). Both machine crates' own
  `tick_tape` detect real EAR-line edges (comparing `ear_bit()` before/
  after each `tape.tick()` call) to push completed pulse widths onto a
  16-entry ring buffer (`TAPE_PULSE_WIDTH_SAMPLES`), and separately
  sample the waveform bar at a fixed cycle interval
  (`TAPE_WAVEFORM_SAMPLE_INTERVAL_CYCLES`, ~1/9th of a real bit-0 pulse)
  rather than once per `tick_tape` call -- the original once-per-CPU-
  instruction sample rate made the bar visually meaningless, since
  instructions take a variable number of T-states each so consecutive
  bar characters didn't represent equal real time. `emu_thread.rs`
  fetches this (plus the memory hex window) on its own 500ms-throttled
  path in the outer tick
  loop, independent of `--debug`'s 1Hz diagnostics gate, and only when at
  least one of `mem`/`disk`/`tape` is actually selected (`OverlayPanels::
  needs_overlay_data`) -- `cpu`-only mode (the default) never pays for the
  extra plugin round trip. FPS is counted from Bevy `Update` executions
  over that real-time interval, so it measures GUI rendering for every
  machine/CPU. `gui/debug_overlay.rs` reads the emu thread's published
  snapshot, never the machine directly. Needs `bevy_ui`, `bevy_text`, and
  `bevy_ui_render` (the last paints UI nodes; `bevy_ui` alone only builds
  layout); none are implied by `"2d"`, even when some are compiled
  transitively.
  `--no-border`/`--nb` disables
  the border `render_frame` adds by default around the active picture (see
  `Zx81`/`ZxSpectrum::set_border` below) -- applies in both GUI and
  `--headless --dump-screen`.
  `--fullscreen` opens the window already `BorderlessFullscreen` (current
  monitor's own resolution) instead of windowed; F11 toggles fullscreen
  live either way -- safe key, F11/F12 are free on every machine here
  (F1/F3/F5/F7 are real C64 function keys, see `any_machine::c64_key`).
  Window creation must use `MonitorSelection::Primary` (no window exists
  yet to be "current" relative to -- confirmed live: `Current` there logs
  a runtime warning and silently no-ops); F11's live toggle uses `Current`
  correctly, since by then one does. **F12 resets the emulated machine**
  (now the rebindable `reset` hotkey, `gui::input::hotkey_input` ->
  `EmuCommand::Reset`) --
  fixed 2026-09-18, user asked directly: "i need a keyboard shortcut to
  reset machine." New `Machine::reset(&mut self)` trait method (non-
  default, same "compile error for a machine with none implemented"
  stance as `save_state`): each machine restores its CPU to power-on
  state (`z80::Cpu::new()`/`mos6502::Cpu::reset` -- `c64::C64::reset`
  reuses the exact call `C64::new` itself already made) while leaving RAM
  untouched, matching what a real hardware RESET line actually does --
  the CPU restarts from its reset vector, but whatever was in RAM a
  moment ago is still there. `emu_thread::tick` drains a `reset_rx:
  &Receiver<()>` before applying queued keyboard input each tick (bundled
  with `input_rx`, and later `mouse_rx` too once the Kempston mouse
  needed a third such channel -- see `zx_spectrum`'s own mouse bullet
  above -- into one `TickInputs<'a>` struct parameter purely to stay
  under clippy's `too_many_arguments` threshold, not a behavioral change)
  and calls `machine.as_machine_mut().reset()` -- works whether or not
  `--debugger` is on, unlike `DebuggerCommand`, since a reset hotkey is a
  basic feature. No headless equivalent (nothing to reset a batch run
  mid-flight for).
  **GUI mode** (`src/gui/{mod,audio,video,window,debug_overlay,input}.rs`
  -- split from a single `gui.rs` once the emu-thread work below gave it
  enough distinct concerns to want per-file separation; `bevy = "0.19"` w/
  `default-features = false, features = ["2d", "keyboard"]` -- real top-level feature names differ
  from guess: no `bevy_winit`/`bevy_input`, just `keyboard`, windowing
  already pulled in transitively by `"2d"`'s `default_platform`):
  CPU stepping, audio generation, and frame production all happen on a
  dedicated background thread (`src/emu_thread.rs`, `emu_thread::spawn`),
  not on Bevy's own thread at all -- a stall anywhere in Bevy's
  render/window handling (a vsync wait, a slow present, shader compilation)
  used to also stall audio generation, since both ran on the same thread
  under the old `FixedUpdate`-stepped design; moving the machine off that
  thread entirely means a render-side stall can no longer starve audio,
  which is the whole reason this thread exists. `spawn` moves the
  `AnyMachine` onto a thread named `"emu"` (visible as such in a
  debugger/Tracy), paced by a fixed-deadline sleep loop -- `next_tick:
  Instant` advances by exactly one `tick_duration` per iteration regardless
  of how long that iteration actually took, resynced to `Instant::now()`
  only if a stall (debugger pause, OS suspend) has left it more than a few
  ticks behind, so an isolated slow tick doesn't push every later tick's
  deadline back too. (A naive "sleep for `tick_duration` minus this tick's
  own elapsed time" loop undershoots the real tick rate over time, since
  `thread::sleep` only guarantees *at least* its argument -- the emu thread
  would then produce audio samples slightly slower than the sound card
  consumes them.) Each tick (`emu_thread::tick`, kept as a standalone
  function precisely so it's unit-testable without a real thread or real
  sleeping) applies queued keyboard input, calls `request_interrupt()`,
  steps the machine for its `clock_hz()/frame_hz()` cycle budget, then
  publishes that tick's audio samples/rendered frame/halted+register
  snapshot into three `Arc<Mutex<...>>` handoffs (`AudioQueue`,
  `FrameBuffer`, `DebugSnapshotHandle`) that the Bevy side reads at its own,
  independent pace. `EmuThreadHandle`'s `Drop` impl is the only way to stop
  the thread (flips a `stop: Arc<AtomicBool>`, joins, logging via
  `debug_log!` rather than propagating if the thread had panicked) --
  `run_gui` holds the handle as a plain local, not an ECS resource, since it
  only needs to outlive `app.run()`.
  `Update` system (`video::render_screen`) takes the emu thread's latest
  frame out of `FrameBuffer` rather than cloning it, both to avoid holding
  the lock across RGB->RGBA conversion and so a tick with nothing new skips
  conversion/upload entirely. The texture uses `ImageSampler::nearest()` for
  pixel-accurate scaling. CPC Gate Array mode changes can alter native frame
  dimensions at runtime; `render_screen` then resizes the texture and quad,
  updates the shader's `texture_size` uniform and `BaseResolution`, and lets
  the existing fit/window-aspect systems consume the new size. Malformed RGB
  lengths are rejected before any surface state changes. Another system
  (`input::keyboard_input`) reads
  `ButtonInput<KeyCode>` and sends events down `InputTxRes`'s channel into
  the emu thread rather than touching a machine directly -- there's no
  machine in this ECS world to touch any more.
  `fit_screen` (`Update`) recomputes the sprite's `Transform.scale` every
  frame from the window's live logical size (`min(w/base_w, h/base_h)`,
  floored, so it's always a whole-number multiplier) -- this, not a fixed
  constant, is what makes resizing the window (or toggling fullscreen)
  actually rescale the picture; `ClearColor` is black so the letterboxed
  space around a non-matching aspect ratio doesn't show bevy's default
  gray. `set_window_icon` (`Update`, retries every frame via a `Local<bool>`
  sentinel until it succeeds once) sets the real title-bar/taskbar icon via
  `bevy::winit::WinitWindows` + the raw `winit` crate's own `Icon` type
  (`bevy::winit` re-exports `WinitWindows` but not `Icon` -- needs `winit`
  as a direct dependency, same version bevy_winit itself pins, decoding
  `assets/icon-64.png` with the already-present `png` crate) -- deliberately
  `Update` not `Startup`: `bevy_winit`'s actual native window creation
  happens via its own event-loop runner, not the App's `Startup` schedule,
  so `WinitWindows` isn't guaranteed to exist yet there (confirmed live: an
  earlier `Startup`-scheduled version panicked with "Non-send data not
  found"). Separate from `winresource`'s build-time `.ico` embedding
  (`build.rs`), which only sets the icon Explorer shows for the `.exe` file
  itself, not the live window.
  **CRT/scanline/bilinear WGSL shaders** (`--shader <name-or-path>`,
  `--shader-intensity`/`--scanline-intensity`, `gui/{video,shader}.rs`,
  `assets/shaders/*.wgsl`, 2026-09-24): the emulated-screen entity always
  renders through a `Material2d` (`video::ScreenMaterial`), never a plain
  `Sprite` -- the default/no-`--shader` case is itself a WGSL shader
  (`passthrough.wgsl`, one `textureSample` call), so there's one rendering
  code path, not a `Sprite`-vs-`Mesh2d` branch. `gui::shader::resolve_source`
  turns `--shader` into WGSL source text: bundled names are `crt`,
  `scanlines`, `sharp-bilinear`, `crt-geom`, `zfast-crt`, and `crt-potato`,
  each `include_str!`'d into the binary; any other value is a filesystem
  path read directly with `std::fs`. `crt-geom`, `zfast-crt`, and
  `crt-potato` are single-pass ports/approximations using the existing
  `ScreenParams` fields: `shader_intensity` controls geometry/mask strength
  where meaningful and `scanline_intensity` controls beam darkening. Their
  upstream defaults remain baked into each shader; this intentionally avoids
  adding a second configuration system for every upstream calibration knob.
  `crt-potato` uses a procedural RGB mask because its upstream preset binds
  external PNG mask textures, which the current one-texture material does not
  load. `crt-royale` and `mame_hlsl` are deferred: their upstream presets
  require 12 and 11 render passes respectively, intermediate render targets,
  and mask textures. `Material2d::fragment_shader() -> ShaderRef` has no
  `&self`, so the runtime source is installed once through the module-level
  `static SHADER_HANDLE: OnceLock<Handle<Shader>>` in `video::setup` before
  render extraction. Existing `crt.wgsl`/`scanlines.wgsl` remain hand-ported
  from crt-pi (GPL v2+, davej); `sharp-bilinear.wgsl` remains hand-rolled.
  See `CREDITS.md` for all upstream sources and port limitations.
  **Window sizing / aspect ratio** (`gui::window::
  apply_initial_window_size`/`keep_windowed_aspect_ratio`,
  `gui::video::fit_screen`/`integer_scale` -- went through several
  same-area reversals 2026-09-20/21, each from direct user feedback; see
  `TODO.md`'s own entry for the full history, this describes only the
  final, current behavior): the window's initial size no longer scales
  with each machine's own native resolution (`native_res *
  INITIAL_SCALE`, still used only as the placeholder size at
  window-creation time before `apply_initial_window_size` runs) --
  instead, the largest whole-number multiple of the emulated picture's
  own resolution (`video::BaseResolution`) that still fits within 60% of
  the primary monitor's own resolution, via a shared `video::
  integer_scale(available, base) -> f32` ("largest integer scale that
  fits a bounding box", the same math `fit_screen` uses for the
  picture's own fit and `keep_windowed_aspect_ratio` uses to correct a
  live resize). Reads a real `bevy_window::Monitor`/`PrimaryMonitor` ECS
  component (`Monitor::physical_size()`, `scale_factor` -- confirmed to
  exist in `bevy_window-0.19.1`'s own source, not guessed) via the same
  "retry every `Update` tick via a `Local<bool>` sentinel until it
  exists" pattern `set_window_icon` already uses. Both this system and
  `keep_windowed_aspect_ratio` write the target size in *logical* pixels
  (`WindowResolution::set`, matching `fit_screen`'s own logical
  `window.width()`/`height()` reads) rather than physical -- an earlier
  version mixed units (physical monitor bounds compared against the
  picture's native resolution, written back via
  `set_physical_resolution`), correct only by coincidence at
  `scale_factor == 1.0` (this session's own Xvfb testing environment),
  wrong on real HiDPI. `apply_initial_window_size` is skipped entirely
  (`run_if`) when `--fullscreen` was given, and only ever runs once.
  `fit_screen` always uses `integer_scale` -- pixel-perfect, never
  distorts the picture, letterboxes with `ClearColor` (black) around any
  leftover space. In fullscreen that leftover space is accepted (a real
  monitor's own aspect ratio can't be forced to 4:3); in windowed mode,
  `keep_windowed_aspect_ratio` (`Update`, a no-op whenever `window.mode
  != WindowMode::Windowed`) is what keeps that space small by snapping
  the *window itself* back to the nearest exact multiple every time its
  live size isn't already one (a >0.5px epsilon guard keeps this a
  no-op, not a fight with the window manager, on every frame where
  nothing actually changed). No live aspect-ratio-*lock* API exists to
  reach for instead -- confirmed by reading `winit-0.30.13`'s real
  `Window` API directly (`set_min_inner_size`/`set_max_inner_size`/
  `set_resize_increments` exist, `set_aspect_ratio` does not) and
  `bevy_window`/`bevy_winit` add none on top; real per-platform APIs
  exist (Windows' `WM_SIZING`, macOS' `NSWindow.aspectRatio`, X11's
  `XSizeHints` `PAspect`) but aren't reachable without raw platform FFI,
  real scope beyond this fix. So this is a *snap*, not a smooth
  constrained drag -- a live resize can browse a few wrong-aspect frames
  before settling. Verified live (Xvfb): a 1280x1024 monitor's 60% initial
  size, a temporary debug system force-resizing the window to a
  deliberately wrong aspect confirming `keep_windowed_aspect_ratio`
  corrects it to the nearest multiple on the very next frame and then
  stays silent (no oscillation), and a separate fullscreen run confirming
  the same system does nothing there at all.
  **Debugger disassembly/assembly**: the debugger's disassembly view (`debugger.rs::publish`) and its step history ask
  the machine plugin (`HostedMachine::disassemble` -> `DebugCommand::GetDisassembly`), which decodes with the CPU crate
  of whichever CPU runs (`z80::disassemble`, `mos6502::disassemble`); only the single-instruction assembler is chosen in
  the host, by the machine's reported `CpuFamily` (`assemble_for`). Step-over treats `CALL`/`RST`/`JSR` as calls.
  **Settings `.toml`** (`crates/formats/app-config`, `--config <path>`,
  2026-09-20): same "own crate, no `machine`/`clap` dependency" split as
  `game-config`, but unlike `--game-config` (per-session, always
  explicit), auto-loaded as `retroclawd.toml` from the current directory
  `AppConfig { default: MachineSettings, machines: HashMap<String, MachineSettings>,
  keys, hotkeys, pads }` -- a `[default]` section plus per-machine
  `[machines.<name>]` overrides, merged with CLI settings by
  `main::resolve_app_settings`. `[machines.<name>]` keys are validated against
  real `--machine` names; `window_scale_percent` remains default-only.
  `fullscreen`/`vsync`/`max_fps` (2026-10-10) are settable under `[default]` or
  a machine section, merged CLI > machine > default like the shader settings
  (`--fullscreen`/`--no-vsync` can only force their non-default; `vsync =
  false` gets the same 60 FPS cap preset as `--no-vsync` when no `max_fps` is
  given; `max_fps = 0` is rejected). `AppSettings` carries the resolved trio
  to `gui::run_gui`.
  **Key bindings** (2026-10-10, phase 2 of the same roadmap): `[keys]` and
  `[machines.<id>.keys]` replace the old `[keymap.*]` tables (a leftover
  `[keymap]` is a clear error). Left side: a host key or chord (same parser as
  hotkeys); right side: a machine key name, a list (pressed together) or
  `"none"`. Machine key names come from the plugin: `plugin_api::
  MachineProfile::key_names` (protocol v7, `MachineKeyInfo { name, aliases }`),
  built in each machine plugin from the *same tables* its `KeyEvent` handler
  looks names up in (`spectrum_key_table`/`spectrum_combo_table`,
  `zx81_key_table`, `c64_key_table`/`c64_combo_table`, `cpc_key_table`/
  `cpc_analog_axis_table`), so the list cannot drift from what is accepted; a
  per-plugin test pins it. The host no longer hard-codes any machine's key
  names (the old `canonical_machine_key_name` table is gone): config text is
  matched against the reported names ignoring case and `_`/`-`
  (`plugin_api::normalize_key_name`), plus free aliases (`KeyA` -> `a`,
  `Digit1` -> `1`, `ArrowLeft` -> `left`) and plugin extras (`SymShift` ->
  `symbol_shift`). `gui/keys.rs` has `KeyBindings::resolve` (layering,
  validation with the valid names in the error, a binding on an ungated hotkey
  chord is an error because the hotkey would swallow it) and `KeyRouter`, the
  Bevy-free per-frame router that replaced `keyboard_input`'s ad-hoc logic: it
  remembers what each press sent (so a release always undoes its own press),
  refcounts machine keys (two host keys on one machine key), keeps the old
  combined-Shift behaviour (both Shifts and the arrow keys are one machine
  Shift), and implements chords: a held chord *consumes* its modifiers, so
  `shift+2 = "at"` releases the machine's Shift before pressing `At` and gives
  Shift back when the key goes up; releases are always emitted before presses
  in a frame. Modifier keys themselves take only plain bindings. Auto-run uses
  `KeyRouter::plain_names`. `retroclawd <machine> --list-keys` prints the
  machine's names with aliases, the host key names and the bindings in effect.
  **Gamepads** (2026-10-10, phase 3): `gui/pads.rs` + `input::pad_input`.
  Same idea as `[keys]` -- a pad button is just another source of *machine key
  names* -- so there is no new plugin command. `[pads]` / `[machines.<id>.pads]`
  (`app_config::PadsConfig`: `deadzone` plus `[pads.<slot>]` sub-tables of
  `button = "machine key"`) are resolved by `PadBindings::resolve` against the
  plugin-reported key names (shared `keys::resolve_targets`: `none`, lists,
  aliases, "only another machine has it" skipped in the shared layer, strict in
  the machine layer). Built-in defaults bind pad N to the machine's `Joy{N}*`
  names *when the plugin reports them* (ZX81: none). `PadButton` is the
  digital set (D-pad, face buttons, shoulders, select/start, thumbs, plus the
  left stick digitised past `deadzone` as `stick_*`); `PadState` is a bitset
  built per frame from Bevy's `Gamepad` component, `PadSlots` keeps pad
  numbers stable across unplugging (first seen = 1, newcomer takes the lowest
  free slot), `PadRouter::route` diffs held buttons into releases/presses and
  `KeyRouter::apply` turns those into events through the same per-machine-key
  reference counts the keyboard uses, so a stick and a D-pad on one machine key,
  or a pad and a keyboard key, release it only when both let go. Plugin side:
  the Spectrum plugin gained `Joy1Up/Down/Left/Right/Fire1` (`JOYSTICK_KEYS`,
  routed through the same Kempston/Fuller/Sinclair/Cursor tables as the
  arrow-key aliases, without pressing the cursor key), CPC already had
  `Joy1*`/`Joy2*`, and the C64 got a real joystick: `c64::JoystickButton`,
  `C64::press_joystick(port, button)`, port 1 ANDed into CIA1 Port B reads
  (shared with the keyboard rows, as on hardware) and port 2 into Port A reads,
  which now return the pin levels (`effective_port_a`) instead of the raw
  latch; joystick state is transient input, not in save states. The old
  arrow-key/Space joystick aliases inside the Spectrum and CPC plugins are
  kept (they are not replaced by host-side defaults, which would have needed
  per-machine cursor-key knowledge in the host). Verified by unit tests
  including a Bevy `App` test that spawns `Gamepad` entities and checks the
  events `pad_input` sends; **not verified with a physical pad** -- this sandbox
  has no `/dev/uinput` or evdev, so gilrs itself was not exercised live. Not
  done: rumble (pad hotkey chords since 2026-10-16, per-game pad maps in
  `--game-config` since 2026-10-16). **Analogue axes (2026-10-19)**: `[pads.N]`
  also binds `left_x`/`left_y`/`right_x`/`right_y`/`left_trigger`/`right_trigger`
  to a machine axis (`MachineProfile::axis_names`; `"-Name"` inverts, `"none"`
  unbinds; default: pad n's left stick to `Joy{n}X`/`Joy{n}Y` when reported).
  `PadRouter::route_axes` sends changed values (X right / Y down positive,
  -32767..=32767, triggers 0..=32767) as `MouseEvent::Axis` to the emu thread,
  which sends `MachineCommand::AxisEvent`. No current machine reports an axis;
  the stub plugin does (`Joy1X`/`Joy1Y`) for the tests.
  **Keyboard layouts and keymap files** (2026-10-10, phase 5): `gui::keys::Layout`
  (`positional`, the default and the old behaviour; `symbolic`), `[input]
  layout`/`keymap` and `[machines.<id>.input]` (`app_config::InputConfig`;
  typed field on `MachineSettings` so it is not swallowed into the option map;
  `[default.input]` is rejected like the other default sub-sections), `--layout`,
  repeatable `--keymap <file>`. A keymap file is `app_config::KeymapFile` (`layout`,
  `[keys]`, `[machines.<id>.keys]`, deny-unknown-fields) -> `gui::keys::LoadedKeymap`;
  `KeyBindings::resolve` takes them as extra layers (lowest: files from `[input]
  keymap`, then the config's `[keys]`/machine keys, highest: `--keymap`; error
  sections read "[keys] in <file>"). Layout precedence is in `main::effective_layout`.
  **Symbolic mechanism:** protocol v8 adds `MachineKeyInfo::chars` (the ASCII
  characters a key types on the real machine, filled by `plugin_api::set_key_chars`
  from a per-plugin table: `SPECTRUM_CHARS`, `C64_CHARS`, `CPC_CHARS`, ZX81 space
  only; each plugin has a test that every table name is a listed key and no
  character is claimed twice). `SymbolMap::from_names` turns the key list into
  char -> (machine key, `CharKind`): letters from `KeyA`..`KeyZ`, digits from
  `Digit0`..`Digit9`, symbols from `chars`. `input::record_logical_keys` reads
  Bevy's `KeyboardInput` messages (`logical_key` = `Key::Character`) into
  `LogicalKeys` (KeyCode -> typed char, only single plain characters), and
  `KeyRouter::resolve_press` consults it after exact chords and an explicit plain
  binding: a typed letter keeps Shift, a typed digit consumes Shift, a typed symbol
  consumes Shift and AltGr (the same `Held::consumed` mechanism as chords, so the
  machine's Shift is released before the key and restored after); a character the
  machine lacks falls back to the physical key's name. The C64 plugin gained the
  shifted-legend combo keys (`Exclamation`..`RightBracket`: Shift+1..9 = `! " #
  $ % & ' ( )`, Shift+`, . /` = `< > ?`, Shift+`:` `;` = `[ ]`), **checked against
  the committed KERNAL's own shift decode table** (index = column*8 + row of this
  crate's matrix) rather than recalled. The CPC's punctuation row and shifted
  symbols are mapped since 2026-10-16 (read from the firmware's own key tables), so symbolic typing covers them too. Auto-run typing and
  hotkeys/pads stay physical. Verified live under Xvfb by remapping the X keymap
  (`change_keyboard_mapping` *after* the app started -- winit only picked it up then; `setxkbmap de`
  did not load in that Xvfb) so the QWERTY-Y position types `z`: positional sent `KeyY`, symbolic `KeyZ`. Not done: dead keys/non-ASCII, per-game
  layout in `--game-config`, VICE-style per-machine positional keymap files with
  matrix positions.
  **QMK host keycodes and the US/C64 keymap** (2026-10-10): host keys in
  `retroclawd.toml`, keymap files and `--list-*` output are written as QMK
  basic keycodes (<https://docs.qmk.fm/keycodes_basic>: `KC_F12`, `KC_RALT`,
  `KC_GRV`, ...). `gui/hotkeys.rs` holds the table: `qmk_name` (one primary
  keycode per bindable `KeyCode`, what `key_display_name`/`Chord::to_text`
  print), `QMK_ALIASES` (long forms like `KC_ENTER`), `QMK_SHIFTED` (QMK's
  shifted keycodes -- `KC_PLUS` = Shift + `=`, `KC_AT`, `KC_COLN`, `KC_DQUO`,
  `KC_UNDS`, ... -- accepted only by `parse_chord`, where they become a chord
  with Shift set, never as a bare key such as `hotkeys.modifier`) and
  `qmk_modifier` (`KC_LCTL`/`KC_LSFT`/`KC_LALT`/`KC_LGUI` and aliases as chord
  modifiers). The old spellings (`f12`, `alt_right`, `ctrl+shift+q`) still
  parse, so existing configs keep working. Honest limit: `Chord` stores
  `ctrl/shift/alt/meta` as either-side flags, so `KC_LCTL` in a chord also
  matches right Ctrl; only `hotkeys.modifier` names one physical side.
  `keyboard/us_c64.toml` is a keymap file (`[machines.c64.keys]`) for a US
  keyboard, loaded through `retroclawd.toml`'s `[machines.c64.input] keymap`:
  the positional default cannot type the C64's `+ : @ * ^ _`, quotes,
  brackets or `( )` (the C64's shifted number row differs from US: Shift+2 is
  `"`, Shift+8/9 are `(`/`)`), so each is bound as a shifted QMK keycode chord
  (Shift consumed) to the C64 key or Shift combo that types it. The C64
  plugin test `the_shipped_us_keymap_only_names_listed_keys` reads the
  committed file and fails if it names a key the machine does not list.
  Verified live under Xvfb (software rendering, XTest key events, screen RAM
  read over the debug-ui HTTP API): `+ : @ * ( ) - = ' " ^ _` typed as the
  screen codes `$2B $3A $00 $2A $28 $29 $2D $3D $27 $22 $1E $1F`.
  **Shift+key chord race (root-caused, fixed)**: the very first Shift+8 after a
  cold boot once typed `$40` (shifted asterisk). Cause: the host sends "release
  Shift, press the key" for a consumed-modifier chord, and `emu_thread::tick`
  applied both at one tick boundary -- which can fall inside the KERNAL's IRQ
  keyboard scan (it reads the matrix column by column), so a scan that had
  already read Shift as down then read the key as down and decoded it shifted.
  `crates/machines/c64/tests/key_switch_race.rs` reproduces it deterministically
  on the real KERNAL (switch applied at 870 offsets across an IRQ period: wrong at
  39 of them atomically, at 0 with a tick between). Fix: `emu_thread::
  take_key_batch` applies a key *press* that follows a *release* in the same
  batch one tick later (order kept, `TickInputs::held_over` carries the rest), so
  no scan can see both. Applies to every machine; costs one 20 ms tick only when
  a release and a press arrive together.
  **File dialogs** (2026-10-10, phase 4): `apps/retroclawd/src/dialogs.rs`,
  `gui/prompt.rs`, hotkey actions `load_tape`/`load_disk`/`eject_tape`/
  `eject_disk`/`load_snapshot`/`save_snapshot`/`load_state`/`save_state`/
  `save_tape`/`save_disk` (defaults only on `ctrl+shift+f9`-`f12` for the first
  four of load/save; the rest unbound). `rfd` 0.17 (default features: XDG
  portal on Linux, no gtk) opens native dialogs. A file action runs on its own
  `file-dialog` thread (`FileDialogs::start`, one at a time via an
  `AtomicBool`, which is also what `FileDialogs::is_busy` reports): rfd's
  dialogs block, and on macOS rfd dispatches to the main thread itself, so
  neither the Bevy loop nor the emu thread waits. `dialogs::run` is the whole
  sequence, written against two traits (`Emu`: the machine side; `Ui`: dialogs
  and the in-app question) so it is unit-tested with fakes, including every
  branch of the unsaved-disk question; `ChannelEmu`/`RfdUi` are the real
  ones. The machine is only ever touched on the emu thread: new `EmuCommand`s
  (`Dialog(bool)` holds it still and silent like a pause, without undoing a
  real pause; `Media(MediaOp)` attaches/ejects/loads and reports with a toast;
  `DiskStatus(Sender)` and `Export(Export, Sender)` are request/reply for the
  save dialogs). `EmuCommand` lost `Copy`/`Eq` for the reply senders. Media
  removal reuses the existing `MachineCommand::DetachMedia` (so no protocol
  bump): implemented in the CPC, ZX Spectrum and C64 plugins on top of new
  `eject_tape`/`eject_disk` methods on `Cpc`/`ZxSpectrum`/`C64`
  (the ZX81 has neither). While a dialog or prompt is open `hotkey_input` calls
  `HotkeyState::swallow_all` and returns, so nothing typed there reaches the
  machine or fires a hotkey (and the machine's own held keys are released).
  The XDG portal has no message boxes (rfd falls back to `zenity`), so the
  unsaved-disk question is drawn in-app: `Ui::choose` sends a `PromptRequest`
  to `gui::prompt::prompt_input`, which shows it as a sticky toast line and
  answers with the option key (`S`/`O`/`D`, Esc = cancel). A swapped-in disk
  gets `DiskExitPolicy` per `--disk-writeback`/`--disk-save` (new
  `emu_thread::MediaSettings`), a modified disk is never discarded silently,
  `save_disk` writes a copy and leaves the disk's own unsaved state alone, a
  "Save As" during a swap marks it saved. Last folders per kind are
  `dialogs::FolderMemory` in `retroclawd.state.toml` next to the config
  (seeded from `--tape`/`--disk`/`--snapshot`/`--load-state`). **Verified live**
  under Xvfb on a CPC664 with a hand-assembled Z80 program that formats a track
  through the FDC (making the disk modified): Ctrl+Shift+F10 showed the prompt
  (screenshot), Esc cancelled cleanly, `D` answered and the pick dialog -- no
  portal exists in that sandbox -- returned immediately without a hang, as did
  `load_tape`. **Not seen:** an actual native dialog window (needs a desktop
  portal), nor macOS/Windows. Not done: `--headless` dialogs, drag and drop,
  recent files, a second drive.
  **Zip and URL inputs (2026-10-11/12)**: every path the host reads -- `--rom*`/`File` options, `--program`,
  `--disk`, `--tape`, `--snapshot`, `--load-state`, `--cartridge`, `--game-config`, `media` in a game config, an
  `.m3u` playlist and its entries, and the load dialogs -- goes through `apps/retroclawd/src/inputs.rs`, a thin
  layer over `file_loader::FileLoader`: a source starting with `http://`/`https://` is downloaded (blocking
  `reqwest`, rustls, streamed in 64K chunks), a `.zip` (local or downloaded) is unpacked, and the **effective
  name** (zip entry, else URL file name) decides the format by extension, so `Game.zip` holding `Game.d81` is a
  `.d81`. A zip with one file just works; **`archive.zip#entry`** (also on URLs, `%20` decoded, matched exactly,
  then ignoring case, then by file name alone) picks one file of a multi-file zip, and a multi-file zip without
  `#entry` is an error listing the entries. A `#` only selects an entry when what precedes it ends in `.zip`.
  **Temp folder**: the run has one `FileLoader`, so every unpacked file goes to one
  `<system temp>/retroclawd-unzip-XXXX/<n>/<entry>` folder (numbered per extraction), removed by
  `inputs::cleanup()` -- `main` holds an `inputs::CleanupGuard`, so it runs on normal exit and when unwinding from
  a panic -- and `file_loader::remove_stale_temp_dirs(24 h)` sweeps what a killed run left behind at the next
  start (a kill/Ctrl-C/power loss skips the guard; there is no signal handler). The in-memory cache makes the
  extension probe and the read share one download/extraction, and survives `cleanup()`.
  **Progress** (`--progress`, `file_loader::Progress`): stderr gets `Downloading name`, a throttled
  (250 ms) in-place `Downloading name: 1.0 MB of 4.0 MB (25%)` line on a terminal (only the first and last line
  when redirected), `Downloaded name (2.0 MB) in 1.2 s`, and `Unpacking entry`. Startup downloads happen before
  the window exists, so they cannot toast; once the GUI is up (`gui::run_gui` calls `inputs::set_toast_target`)
  a download -- an `.m3u` disk swap to a URL -- posts a toast for its start and its end (no running percentage:
  the toast queue is one line for a couple of seconds). No OS notifications (`notify-rust` was considered and
  skipped: nothing else here notifies). `.m3u` entries may be paths, URLs or `zip#entry`; relative ones resolve
  against the playlist's folder, or its URL directory when the playlist is a URL (`/abs` against the site root).
  `--disk-writeback` refuses a URL or zip source (`inputs::is_writable_file`; the swapped-in-disk path in
  `emu_thread` falls back to `--disk-save`), because the raw image must never replace an archive; use
  `--disk-save <file>`. Verified live: a `.zip` of a `.d81`, the same zip and a multi-file zip (`#entry`) served by
  `python3 -m http.server`, an `.m3u` URL whose entry is a `zip#entry`, `--progress` output, no
  `retroclawd-unzip-*` folder left behind, and a C128 kernal passed as `--rom-kernal http://.../k.zip`. Tests:
  `file-loader` (6: selector parsing, temp folder + cleanup + drop, entry selection, no name collisions, stale
  sweep), `inputs::tests` (plain file, zip, `#entry`, cleanup, a local TCP HTTP server, progress text),
  `playlist::tests` (URL playlists). Not done: HTTP auth/headers, an on-disk download cache, a signal handler for
  Ctrl-C cleanup, a running-percentage toast.
  **Per-run settings, per-game file names, symbols on input, crash report (2026-10-16)**: `apps/retroclawd/src/overrides.rs` folds
  `--hotkey ACTION=CHORD` (repeatable; the first flag for an action replaces the config's bindings, later ones add; `none`;
  `modifier=KEY`) and a game config's `[keys]`/`layout` into the loaded `AppConfig` as the top layer of the machine's own
  sections (`[machines.<id>.hotkeys|keys|input]`), so `Hotkeys::resolve`/`KeyBindings::resolve`/`effective_layout` validate them with their
  usual messages (CLI `--layout`/`--keymap` still rank above a game config); a game config's `symbols` (string or list) are loaded
  before `--symbols`. `GameConfig` gained `keys`, `layout`, `symbols` (all optional, `deny_unknown_fields` still on). Screenshot and
  quick-save names carry the game: `host_files::game_stem` (file stem of the loaded media; `a.zip#dir/Game.d64` -> `Game`, URLs
  decoded, anything unsafe `_`) and `game_tag` -> `states/<machine>-<game>-slotN.state`, `MediaSettings::game` filled in `main` from the
  game config's/CLI media, then `--program`, then `--cartridge`; with nothing loaded the old `<machine>-slotN` names stay. The debugger's
  `assemble` (also ZRCP's) accepts symbol names in operands: `machine::SymbolTable::substitute` (shared repository) replaces a known
  name by `$XXXX` (6502) or `0xXXXX` (Z80) -- not the mnemonic, registers/conditions or numbers --, so `JSR CHROUT` and
  `CALL LD_BYTES` assemble (`debugger::assemble_with_symbols`, unit-tested against the built-in tables). A crashed plugin (a panic
  caught by `plugin_api::guard`, after which every call returns an error naming it) is now noticed by `HostedMachine::crash_message`
  once per emu-thread tick: one toast and a console line, and the machine is held like a pause with its last picture, instead of the
  emu thread panicking on the next clock query. Unit-tested: the argument parsing/layering, the names, the substitution and the
  crash-error predicate; and live (2026-10-16) with the stub plugin, which panics on a `crash-now` key event (`test_machine_plugin_impl::
  CRASH_KEY`): the crash flag is process-wide, so `emu_thread::tests::a_crashed_plugin_is_reported_once_and_the_machine_is_held`
  re-runs the test binary on itself (env `RC_CRASH_TEST_CHILD`) and the crash happens only in that child; it checks `tick` reports once
  and holds the machine (mutation-checked: without the probe the clock query panics). A game config also takes `[hotkeys]` (incl. `modifier`) and `[pads]`/`[pads.N]` (incl. `deadzone`), folded into the machine's own sections per action / per button below `--hotkey`.
  **Pad hotkeys, accents and dead keys, headless audio, a second drive (2026-10-16, later)**: *Pad chords*: `[hotkeys]` bindings written
  `pad:select+start` / `pad2:l1+r1+south` (`gui::hotkeys::PadChord`, `parse_pad_chord`; a keyboard chord never starts with `pad<N>:`) resolve
  into `Hotkeys::pad_bindings` (a duplicate is the same error as for keys). `pad_input` calls `Hotkeys::poll_pads` on the slot/state list
  before routing: a chord fires `Trigger::Pressed` when its last button goes down (`Released` for a hold action when it stops being held),
  its buttons go into `PadHotkeyState::swallowed` and `mask` clears them from the states the `PadRouter` sees until they are released. The
  triggers wait in `PadTriggers` (which also owns that state) for `hotkey_input`, which runs first next frame and handles them exactly like
  keyboard triggers (dialogs open: discarded). Bevy `App` test: Select reaches the machine, Start completes the chord, Select is released.
  *Accents and dead keys*: `gui/fold.rs` (`ascii_fold`: Latin-1/Latin Extended-A letters to ASCII, `ß` -> `ss`, `Æ` -> `AE`). In the symbolic
  layout `SymbolMap::lookup` tries the typed character, then its single-letter fold; `LogicalKeys::pressed_dead` records Bevy's `Key::Dead`
  and `resolve_press` sends nothing for it unless the physical key has an explicit plain binding. `paste::plan` types a character with no key
  as its folded letters if the machine has keys for all of them. Not verified with a real dead-key keyboard (unit tests feed `LogicalKeys`).
  *`--dump-audio file.wav`* (`wav_bytes`): headless `--frames` runs drain the machine's audio each frame; `rc_suite::Audio` reads it
  (DC-free `rms`, single-bin `tone`). *Second drive*: `HostedMachine::apply_disk_in(drive, ..)`, `disk_exit: [Option<DiskExitPolicy>; 2]`,
  `--disk-b`/`--disk-b-save`, `MediaOp::{AttachDiskB, EjectDiskB, DiskBSaved}`, `Export::DiskB`, file actions `load_disk_b`/`eject_disk_b`/
  `save_disk_b`, `--at` actions of the same names. Drive B has no "ask first" prompt: `HostControl::drive_b_can_be_replaced` writes unsaved
  changes to the `--disk-b` file (`--disk-writeback`) or `--disk-b-save` before a replace/eject, else **refuses** the change with a message
  -- never a silent loss. Only the CPC664/6128 and the Spectrum +3 plugins have a drive B (the other Spectrums and the C64 refuse `drive 1`); `--disk-writeback` no
  longer requires `--disk` (either disk will do). The stub machine has two drives and test keys `dirty-disk-a`/`dirty-disk-b` for the guest
  writing. Verified through the real 6128 firmware: AMSDOS reads `B:SAME.BIN` from drive B while `A:SAME.BIN` is a different file.
  **`--capture-at ADDR:REG=FILE` (2026-10-18)**: headless, needs `--frames`; appends the low byte of Z80 register `REG` (A/F/B/C/D/E/H/L) to FILE each time PC reaches ADDR (`register_byte`, `parse_capture_at`). Used by rc-suite to read the CPC acid programs' text (TXT OUTPUT `0xBB5A`, register A); an interrupt landing exactly on the entry can double a character, so compare squeezed. Acid-test gap status (what was fixed, what remains) is in `TODO.md` "Acid suites".
  **Headless actions and symbol arithmetic (2026-10-16)**: `--at FRAME:ACTION[=FILE]` (`apps/retroclawd/src/scripted.rs`, needs
  `--frames`) runs the hotkey/dialog actions of a batch run: `scripted::Runner` owns a real `HostControl` and feeds it the same
  `EmuCommand`s (`Reset`, `QuickSave`, `Media(MediaOp)`, `Export` ...) the window sends, with files read through `inputs::load` (zips
  and URLs work) and written where the command line says; actions are sorted by frame (stable), run before that frame executes
  (`frames` itself = after the last frame), their toasts are printed as `frame N: ...`, and a failure message (or an unreadable /
  empty export) makes the exit code 1. `key_down=NAME`/`key_up=NAME` press or release a machine key by its `--list-keys` name (`HostedMachine::try_key_event`, so a name the plugin
  refuses fails the run instead of being logged and ignored). Window-only actions are refused by name. Tested with the stub machine (order, file round trip,
  failure) and live on a Spectrum 48K. `SymbolTable::substitute` (shared) also folds a constant offset after a known name
  (`CHROUT+1`, `BORDER - $10`, decimal/`$`/`0x`/`h`, chained, wrapping at 64K) and `#<NAME`/`#>NAME` (6502 low/high byte, applied after
  the offset); a register's displacement (`(IX+5)`) and an unknown name are untouched. No general expressions (`*`, parentheses).
  **Version info (2026-10-17)**: `build-info` (shared repository, `crates/build-info`) stamps a crate at build time -- its `build.rs` calls
  `build_info::emit()`, which asks git (in the crate's own repository) for `rev-list --count HEAD` (the **build number**), the abbreviated
  `HEAD` hash and its commit date, and whether tracked files differ from `HEAD` (submodules and untracked files ignored); without git it
  reads `RC_BUILD_NUMBER`/`RC_GIT_COMMIT`/`RC_GIT_DATE`/`RC_GIT_DIRTY`. `build_info::build_info!()` is the host's `BuildInfo`
  (`version::HOST`), `build_info::version_tag!()` a string `0.1.0+39.6bdc7515ff.2026-10-17[.dirty]` (semver build metadata) that each
  machine plugin puts in its descriptor's existing `version` field -- **no ABI change**, an older plugin just shows its plain version
  (`Info::parse` reads both). `apps/retroclawd/src/version.rs` formats it: `--version`/`-V` and the top of `--help` print the host line and
  one line per plugin found, `--list-plugins` the same per plugin, and the overlay has a `version` panel (`--show-overlay ...:version`, in
  `all`) that the `version` hotkey (`shift+f9`, `Action::Version`) flips, showing the overlay with it when hidden. The host reports the
  retroclawd repository, each plugin its machine family's repository. The build script reruns when HEAD, the branch ref or the crate's own
  files change; an edit in another crate of the same repository does not rerun it, so the `dirty` flag can lag until the next commit or
  edit there (the commit and build number are exact). Tests: `build-info` (tag round trip, dirty, plain version), `version::tests`
  (stamped host, plugin lines, overlay text), `debug_overlay` (panel parse, hotkey toggle).
  **Debugger symbols (2026-10-12)**: `machine::SymbolTable` (shared crates) is a CPU-agnostic address->name table that
  annotates disassembly *text* (`  ; NAME` after the first known `$1234`/`$12` (6502) or `0x1234` (Z80) operand; text
  that already has a `;` or an immediate `#$12` is left alone) and parses symbol files (`NAME = $1234`, `NAME EQU
  0x1234`, `NAME 1234`, VICE `al C:1234 .NAME`; a bad line is an error naming the line). `apps/retroclawd/src/
  debug_symbols.rs` holds the built-in tables per machine id and the `--symbols` files; `debugger::disassembly_window`
  applies them (and `present_text` rewrites a Z80 `JR`/`DJNZ` to its absolute target with `z80::disassemble::
  disassemble_at`), so annotation lives in the host and travels in no protocol field. `assemble_for` uses
  `assemble_one_at` for the Z80 for the same reason.
  **Hotkeys** (2026-10-10, phase 1 of `TODO.md`'s "Input, hotkeys and file
  dialogs roadmap"): host-level actions bound to key chords in `retroclawd.toml`
  -- `[hotkeys]` plus per-machine `[machines.<id>.hotkeys]`
  (`app_config::HotkeysConfig`: an optional hold-to-enable `modifier` plus
  `action = "chord"` or a list of chords, flattened so action names are
  validated by the app, not serde). `gui/hotkeys.rs` owns everything that does not
  need Bevy systems: the `Action` list (`ACTIONS`, with defaults and help
  text; `--list-hotkeys [machine]` prints the effective table), chord
  parsing (`ctrl`/`shift`/`alt`/`super` + a key, modifiers matched *exactly*
  and ignoring both the chord's own key and the gate modifier -- a real bug the
  unit test `the_hold_modifier_gates_every_hotkey` caught: with `modifier =
  "alt_right"` the Alt key itself was counted as an extra chord modifier),
  layering (defaults -> `[hotkeys]` -> machine, per binding, `"none"`
  unbinds, a chord bound to two actions is an error naming both) and
  `Hotkeys::poll`, a pure function of `ButtonInput<KeyCode>` that returns
  `Trigger`s and records *swallowed* keys: `input::keyboard_input` runs
  after `input::hotkey_input` and skips both the press and the release of a
  swallowed key, so a hotkey key never half-reaches the emulated machine.
  Defaults stay on F9-F12, Pause, Print Screen, Scroll Lock and chords on
  them (a test pins that none of F1-F8/Tab/Escape is taken: the C64's
  F1/F3/F5/F7 and every letter/digit stay the machine's). Actions:
  `reset` (F12) / `fullscreen` (F11, `window::toggle_fullscreen(&mut Window)`)
  / `quit` (Ctrl+F12) run on the GUI side; `pause`, `screenshot`,
  `quick_save`/`quick_load`/`state_slot_next|prev` and `fast_forward` are
  `emu_thread::EmuCommand`s on the (replaced) `reset_tx` channel, applied by
  `HostControl::apply` before the machine steps. Pause freezes stepping,
  audio and rendering like a debugger pause (without its step requests);
  fast-forward shortens the pacing period by `FAST_FORWARD_FACTOR` (8) and
  drops its audio, every tick still steps the full cycle budget (so tape
  loading runs ~8x faster in wall time too, a different mechanism from
  `--tape-speed`). Files: `screenshots/<machine>-<YYYYMMDD>-<HHMMSS>.png`
  (UTC, civil-from-days, `-2`/`-3` on collision) and
  `states/<machine>-slotN.state` (10 slots), both under the working
  directory (`host_files.rs`, pure and unit-tested). `mute`/`volume_up|down`
  drive `audio::AudioControl` (relaxed atomics read on the audio callback
  thread; the decoder ramps its gain over ~512 samples so muting never
  clicks; raising the volume un-mutes). `overlay` toggles
  `debug_overlay::OverlayStateRes` (always registered now: `--show-overlay`
  only picks the starting state, with `cpu` shown when the flag was never
  given) and flips the `Arc<AtomicBool>` the emu thread watches before it
  fetches the slower memory/disk/tape panel data. `shader_next|prev` rewrite
  the screen shader asset's WGSL in place (`video::switch_shader`, relying on
  Bevy's own shader hot-reload path) over the bundled list
  (`shader::BUNDLED`). `gui/toast.rs` shows one-line messages ("Paused",
  "State saved: slot 2", "Volume 70%") from both the GUI and the emu thread
  (`emu_thread::Messages`). Not done yet: `.state` slots are per machine only
  (no per-game naming), no hotkeys in `--headless`.
  **Paste (2026-10-12)**: the `paste` hotkey (default `shift+insert`; Insert is no machine's key)
  types the system clipboard's text into the machine. `gui/paste.rs`: `plan(text, SymbolMap, enter,
  shift, PasteCase)` turns text into machine key events with the same `SymbolMap` the symbolic
  keyboard layout uses (letters/digits from `KeyA..`/`Digit0..`, symbols from the plugin's
  `MachineKeyInfo::chars`), so every machine works without a plugin command; CR/LF/CRLF = one Enter,
  tab and no-break space = space, typographic quotes/dashes fold to ASCII, a character with no key is
  skipped and counted in the toast. `PasteState` + `paste_typing` drip one event per `paste_delay_ms`
  (default 60: three 50 Hz frames, one press or release each) into the same `InputTxRes` channel the
  keyboard uses, **waiting while any host Shift/Ctrl/Alt/Super is down** (found live: the chord's own
  Shift reached the machine and typed `!` for `1`). A second press cancels and releases every key the
  paste still holds. `[input] paste_case` / `[machines.<id>.input]`: `"fold"` (default; every letter is
  the plain key -- an uppercase BASIC listing on the C64, where Shift+letter is a graphics glyph) or
  `"exact"` (a capital holds the machine's Shift, found through `KeyRouter::plain_names(ShiftLeft)`,
  right for Spectrum/CPC text); `paste_delay_ms` likewise. The clipboard is read with `arboard`
  (text only; X11 and Wayland data-control on Linux). Tests: 8 planner unit tests, 4 Bevy-`App`
  system tests (ordering and pacing, cancel, modifier wait, errors), a config-resolution test; live
  under Xvfb with a python-xlib clipboard owner and XTest, the pasted `10 PRINT "Hello, c64!"` /
  `20 GOTO 10` / `RUN` ran on the C64.
  **Audio**: `bevy` needs the `bevy_audio` cargo feature (not implied by
  `"2d"` or any other feature already listed -- added explicitly). Bevy's
  audio API is built around `Handle<Source>` + `AudioPlayer` for decoding
  files; streaming synthesized PCM instead uses its documented escape
  hatch, `Decodable`/`AddAudioSource::add_audio_source`. `EmulatorAudio`
  (an `Asset` wrapping the emu thread's own `AudioQueue` -- created in
  `emu_thread::spawn`, not here; one instance for the whole run, spawned
  once in `gui::audio::setup_audio`) is that `Decodable` type;
  `EmulatorAudioDecoder` is its `rodio::Source` (re-exported as
  `bevy::audio::Source` -- no direct `rodio` dependency needed), an
  infinite stream that pops the queue and returns silence (`0.0`, never
  `None`) on underrun, since returning `None` would tell the backend
  playback finished and it would stop polling for good. `emu_thread::tick`
  itself (not a Bevy system -- see above) is what pushes samples into the
  queue every emu-thread tick, capped at `MAX_QUEUED_AUDIO_SAMPLES` (~2s)
  by dropping the oldest samples if the audio backend ever falls behind.
  Works for any machine with no special-casing: `zx81::Zx81` (no audio
  modeled) just never pushes anything, and the sink plays silence -- exactly
  the `Machine::drain_audio` default-empty-`Vec` behavior paying off.
  `setup_audio` also clears the queue right before starting playback
  (`Startup`, so this runs after window creation/GPU init/shader
  compilation -- often several hundred ms to a couple of seconds, during
  which the emu thread has already been ticking since before `App::new()`
  with nobody draining it yet): whatever backlog built up in that window is
  stale, pre-playback audio, not real dropped audio, and clearing it here is
  what stops it from becoming permanent latency for the rest of the session
  (production/consumption rates match from then on, so nothing would
  otherwise shrink the queue back down later).
  **Resampling (real bug, root-caused live, not guessed)**: Bevy's
  `Decodable`/`add_audio_source` escape hatch does NOT resample a custom
  source to the real output device's configured rate the way normal
  file-decoded playback does -- confirmed by reading rodio's own
  `mixer.rs`, whose `Mixer::add` wraps every appended source in a
  `UniformSourceIterator` that resamples to the mixer's fixed rate, and
  confirmed live that `sink.append(decoder...)` (Bevy's own code, for a
  custom `Decodable`) doesn't go through that same conversion. Diagnosed
  with `--debug-file` evidence (see `emu_thread`'s `UnderrunCount`/
  `RequestedCount` diagnostics below) on a real 48000Hz-configured Windows
  output device: `EmulatorAudioDecoder` was being read at ~48000/sec (the
  real device rate) while only ever producing `machine::AUDIO_SAMPLE_RATE`
  (44100)/sec -- a permanent ~8% shortfall every second, heard as
  continuous skipping, not an occasional glitch. Fix: `EmulatorAudioDecoder`
  does its own linear-interpolation resampling from `machine::
  AUDIO_SAMPLE_RATE` to `output_rate` (the real device rate, queried once
  via `cpal` directly in `setup_audio` -- `cpal::default_host().
  default_output_device()...default_output_config()`, falling back to
  `machine::AUDIO_SAMPLE_RATE` -- i.e. no resampling -- if the query fails
  for any reason) and reports `output_rate` from `Source::sample_rate`
  instead of the fixed generation-rate constant. The input-consumption
  rate this self-corrects to is exactly `machine::AUDIO_SAMPLE_RATE`
  regardless of how often the backend actually calls `next` -- see
  `EmulatorAudioDecoder`'s own doc comment for the fractional-position
  math (unit-tested directly, `apps/retroclawd/src/gui/audio.rs`'s own
  `tests` module, since it's pure logic with no Bevy/hardware dependency).
  **Diagnostics** (`emu_thread`'s `UnderrunCount`/`RequestedCount`, both
  `Arc<AtomicU64>`, incremented from the audio backend's own real-time
  callback thread in `EmulatorAudioDecoder`): logged once/second under
  `--debug`/`--debug-file` as `queue_len`/`requested/s`/`underruns/s`/
  `total_underruns` -- this is what actually found the resampling bug
  above (`requested/s` tracked the real device's rate, not `machine::
  AUDIO_SAMPLE_RATE`, which a queue-depth-only view couldn't have shown).
  `emu_thread::tick` also logs a `slow tick: total=.. step=.. audio=..
  render=..` line (only when a tick exceeds `SLOW_TICK_THRESHOLD`, 10ms)
  -- ruled out a stepping/rendering stall as the cause here (zero such
  lines over ~80s of a real reproduction), which is what pointed the
  investigation at a rate mismatch instead of a scheduling one. The 1Hz
  log also reports `ticks/s` (real ticks completed per wall-clock second)
  -- ruled out Windows' default ~15.6ms timer resolution making
  `thread::sleep` oversleep and silently starve the real production rate
  (measured a steady ~50.0/sec throughout a real reproduction, ruling that
  hypothesis out too).
  **HALT-truncated audio budget (the actual root cause, real bug,
  root-caused live)**: the resampling fix above was real and correctly
  fixed, but didn't resolve the reported skipping -- `underruns/s` stayed
  within the same range across every diagnostic round regardless of which
  consumption-side fix or rule-out was applied, which is what pointed the
  investigation upstream of the queue entirely. `tick`'s step loop used to
  break the instant `m.halted()` went true, even with real cycle budget
  left (`spent < budget`) -- documented at the time as deliberate ("step
  the machine for one `frame_hz()` tick's cycle budget (or until
  halted)"). But `ZxSpectrum::step`'s audio sample generation is driven
  purely by the cycle count each individual `step()` call returns (see
  `audio_cycle_accumulator`'s doc comment in `zx-spectrum`) -- so a step
  loop that stops calling `step()` early stops producing audio for the
  rest of that tick's real ~20ms too, exactly like real hardware would
  still be burning T-states (and needing new sample output) the entire
  time it idles in `HALT`. Almost any game hits this on nearly every
  single tick: the standard `EI`/`HALT` idle-loop-between-interrupts
  pattern means the CPU re-halts within a handful of instructions after
  each frame interrupt, so only a small, fairly constant fraction of each
  tick's budget ever actually ran -- explaining both the magnitude and the
  tick-over-tick consistency of the underrun rate seen in every round.
  Confirmed via a failing-then-passing unit test
  (`emu_thread::tests::tick_keeps_producing_audio_for_the_full_budget_even_after_the_cpu_halts`)
  before the fix landed: a tick that halts ~26 T-states in (a `LD`/`IN`/
  `HALT` sequence) produced exactly 0 audio samples under the old code,
  against an expected ~882 (a full tick's worth at `machine::
  AUDIO_SAMPLE_RATE`/`frame_hz()`). Fix: the step loop no longer breaks on
  `m.halted()` at all, only on `spent >= budget` -- `m.step()` while
  halted is a cheap, constant-cost 4-cycle no-op (`z80::halted_nop`), so
  always spending the full budget (worst case ~budget/4 extra no-op steps)
  costs nothing worth avoiding, and keeps the audio accumulator advancing
  in real time exactly like real hardware's own T-state clock does
  regardless of what the CPU is doing with them.
  **Profiling**: the opt-in `profiling` Cargo feature (`bevy/trace_tracy` +
  `tracing`, off by default -- `cargo run --features profiling`) wires a
  Tracy span (`emu_thread_step_budget`) tightly around just `tick`'s own
  step loop, not the audio/frame/debug publishing after it, so a Tracy
  client attached to the running process shows per-tick CPU-step cost in
  isolation from that other work. Costs nothing when the feature is off --
  the span is behind `#[cfg(feature = "profiling")]`, not a runtime check.
  **`--plugin debug-log` (fixed 2026-10-01)**: the debugger (module
  `debuggers/log.rs`, selector `debug-log`) is the only thing that installs a
  `tracing` subscriber -- Bevy's `LogPlugin` is disabled and `main.rs` has no
  host-side sink. `run_gui` used to filter `debug-log` out of the launch list
  (a leftover from an earlier host-side logger), so `--plugin
  debug-log:.\output.log:debug` silently produced no file and no stdout;
  it now launches like every other debugger (test
  `debuggers::tests::a_debug_log_selection_starts_and_writes_the_requested_file`,
  verified live under Xvfb with the reporter's exact command shape plus the
  stdout sink). Shorthand is `debug-log:<output>:<level>` (`split_plugin_tokens`
  keeps a Windows drive prefix `C:\...` in one token); third-party engine
  crates (bevy/wgpu/naga/winit/audio, `ENGINE_CRATE_PREFIXES`) are held to the
  new `engine_level` option (default `warn`) because `level=debug` otherwise
  wrote ~45k shader-compiler lines in 20 s against ~500 of the emulator's
  own. Under `--headless` debugger plugins aren't started (no GUI emulation
  thread to attach to) and `main` now says so on stderr.

  **RomM and `--firmware auto` (2026-10-15)**: `apps/retroclawd/src/romm.rs` is the RomM client (`romm platforms|games|
  firmware|match`, `romm:<id>` / `romm:fw/<id>` sources through `inputs.rs`, a download cache with `cache list|clean|
  remove`; aligned text tables; `ROMM_URL`/`ROMM_TOKEN` or `[romm]` in `retroclawd.toml`, the token never printed).
  `apps/retroclawd/src/firmware.rs` is `--firmware auto` (`--firmware auto|off`, `[romm] firmware`): after the normal
  option resolution (`resolve_machine_options(..., defer_missing_files = true)` leaves required file options
  unfilled instead of erroring), every `File` option still without a value is matched against the `firmware.toml`
  entries (`[[rom]]` with `machines`, `option`, `label`, `sha1 = [...]` preferred first or `parts = [...]` to join,
  optional `when = { key = "value" }` on resolved option values, e.g. `true_drive`/`drive_type` for the drive DOS) and
  the server's `GET /api/firmware` list (its `sha1_hash`), downloaded through the cache, **verified against the hash it
  was chosen by**, and added to the machine's ROM blobs. The `firmware.toml` files live in the family repositories
  (hashes only) and are found as `firmware.toml` / `*.firmware.toml` in the machine plugin folders
  (`plugin_bridge::plugin_search_dirs`; `tools/build_all.sh` and `package_release.sh --firmware` stage them). A named
  file always wins; options no entry applies to (a drive ROM without `--true-drive`) are left alone; a required option
  with no match on the server is one error naming what was wanted. Tests: parsing/validation, preferred-hash order,
  `when`, `parts`, skipping, deferral and mode resolution; checked live against a throwaway fake RomM (Spectrum and ZX81
  booted from firmware fetched by hash; a tampered cached file refused).

  **Debuggers are modules, not plugins (2026-10-14)**: `debug-ui`, `debug-zrcp` and `debug-log` were
  separate crates behind a C-ABI of their own (`DebuggerPluginVTableV1`, `HostDebugApiV1`, bincode
  messages), a `crates/plugin-dlls` cdylib each and an env-var discovery mechanism, although nothing
  other than this application ever loaded them. They are now `apps/retroclawd/src/debuggers/{ui,zrcp,log}.rs`
  behind the cargo features `debug-ui`, `debug-zrcp`, `debug-log` (all default; `--no-default-features`
  builds an app with none, which then has no `--debugger`/ZRCP/`--plugin debug-log`). A debugger gets a
  `HostBridge` (an `Arc<dyn HostApi>`, in-process request/response with the `DebuggerHostRequest`/
  `Response` types now in `debuggers/api.rs`) answered by `debuggers/host.rs` from the emu thread's
  `DebuggerState` and command channel; `start(options, instance_id, host) -> Running` returns the web
  URL / ZRCP address and a guard whose drop stops the server. `--plugin debug-ui=<path>` (a debugger
  library path) and the `RETROCLAWD_DEBUGGER_PLUGIN(S|_DIRS)` variables are gone (a clear error for
  the former); `--plugin <selector>:OPT:OPT` and the old aliases (`debugger`, `debug-tcp`) work as
  before, and `--list-plugins` prints the built-in debuggers. In `plugin-api` (shared repository) the
  whole debugger half was deleted (vtable, host API, `DebuggerHandle`, `PluginKind::Debugger`, the
  `Debugger*` protocol types); the machine ABI and wire format are unchanged, so `PROTOCOL_VERSION`
  stays 10 (only the schema fingerprint was re-recorded) and machine libraries built earlier still
  load. Verified live under Xvfb: `--plugin debug-ui:port=N` (state, pause, step over HTTP),
  `debug-zrcp:port=N` (`get-version`, `about`, `get-registers` over TCP) and `debug-log:<file>:info`
  together on a ZX Spectrum 48K. The unit tests of the former plugin crates (the ~200 ZRCP tests
  with their fake host, the log filter tests) moved with the code; the fake host is now a `HostApi`
  implementation.

  **Plugin-host runtime routing** (current): `apps/retroclawd/src/plugin_bridge.rs`
  owns `HostedMachine`, machine-profile discovery, bundled/static registry,
  dynamic library loading, instance lifecycle, media/state/render/audio calls,
  and the generic `Machine` adapter. `AnyMachine` and `src/any_machine.rs`
  were removed. Machine-specific keyboard, tape, snapshot, disk, joystick, and
  bank behavior now lives behind each machine plugin's `MachineCommand` handler.
  The browser, log and ZRCP debuggers are not plugins (2026-10-14, see below).
  `plugin-api` protocol version 2 carried machine bitmap palette and
  Bitmap Explorer defaults for the debugger state snapshots; version 3 adds
  `ExportDisk`/`MarkDiskSaved`/`DiskImage` for disk persistence; version 4
  adds `ExportTape`/`TapeImage` for `--tape-save` cassette recording; version 5
  adds `MachineCommand::WriteIo` and `DebuggerHostCommand::WritePort` for ZRCP
  `write-port`; version 6 adds the port-watch commands (`SetPortWatches`/
  `TakePortHit`, `SetPortBreakpoint`/`ClearPortBreakpoints`) and the
  `port_breakpoints`/`last_port_hit` fields of the debugger state snapshot.

  **Machines are shared libraries (2026-10-12)** -- supersedes the feature-gated bundling paragraph
  below. The Windows `*_machine_plugin.dll` files were ~10 KB because they were empty: the plugin
  crates exported `retroclawd_plugin_entry_v1` only for `dynamic` and not `bundled`, `retroclawd`
  depended on them as `bundled`, and cargo unifies features across a workspace build, so the
  libraries a normal `cargo build --release` produced had no exported entry and the linker dropped
  all their code -- the emulation was inside `retroclawd.exe`. The fix is structural: every plugin
  is an ordinary library crate (`crates/plugins/<name>-plugin`, lib `<name>_plugin_impl`, only
  `entry_v1()`) plus a `crates/plugin-dlls/<id>` `cdylib` crate in the family repository (`<name>-plugin-dll`, lib
  `<name>_plugin`) containing just `plugin_api::export_plugin!(<impl>::entry_v1)`; the `dynamic`/
  `bundled` features no longer exist, so no feature combination can produce an empty library.
  `retroclawd` depends on no machine plugin (its unit tests link the implementation libraries as
  dev-dependencies, which never reach the shipped binary) and discovers libraries in
  `RETROCLAWD_MACHINE_PLUGIN_DIRS`, else `<exe dir>/plugins`, `<exe dir>` and `./plugins`
  (`plugin_bridge::default_plugin_dirs`; none under `cfg(test)`); `RETROCLAWD_MACHINE_PLUGIN(S)`
  name files explicitly. `cargo build --workspace` (or `-p <name>-plugin-dll`) builds them next to
  the executable; `cargo build -p retroclawd` alone builds only the app. A named machine with no
  library is "unknown machine ...; discovered machines: ..." (or "machine plugin discovery failed:
  no machine plugins found ... searched ..." when there are none) instead of clap rejecting its
  options. Verified: `nm` on the debug binary finds none of the machine/chip crates' symbols; the
  four machines render their screenshots from the libraries (`render_plugin_screens.sh`); a folder
  holding only the ZX81 library offers only `zx81`. Not done: the machine
  source still lives in this repo, so removing a machine from the *project* (not just a build)
  still means moving its crates out -- the planned next step is one repository per machine.
  **ABI readiness for 16-bit machines (2026-10-18)**: protocol 11, ABI minor 2, profile contract 2 (ADR 0003, design in
  `docs/superpowers/specs/2026-10-18-abi-16bit-readiness-design.md`). `machine::Addr` (`u64`) is the address type of the
  `Machine` trait and the plugin protocol; an address beyond a machine's space wraps to it. The fast path gained `pc_wide`
  (the host uses it; the old `pc` is the low 16 bits). `machine::Bus` is unchanged (16-bit): a 68000 Core defines its own bus.
  `DebugCommand::OnCpu { cpu, command }` addresses one CPU of several (`MachineProfile::cpus`, primary first; plugins today accept
  only `"main"`), `MachineCommand::AxisEvent` carries analogue input (`axis_names`), `MachineResponse::AudioPcm` replaces
  `AudioPcmMono`, and `MachineMetadata` reports the exact `frame_rate`, `pixel_aspect` and audio format (zero = the old defaults).
  The host pacing and cycle budget use the exact frame rate; the debugger state and the browser page follow the machine's
  address width (`addr_bits`); ZRCP stays 16-bit at its edge. Stereo/sample-rate audio, pixel aspect and pad axis events are consumed since 2026-10-19 (see the
  spec's Status section).
  **Plugin ABI hardening and compatibility policy (2026-10-12)**: the policy is `docs/plugin-abi.md`
  (read it before touching `plugin-api`). Code: `plugin_api::compat` (the host's accept/refuse rules,
  shared by both loaders; reads `abi_major`/`abi_minor` first, refuses another major before
  interpreting the descriptor), `codec::decode_message` (protocol version checked before the payload;
  256 MiB decode limit; `CodecError::into_call_result` gives plugins a readable error), `plugin_api::
  guard` (`guarded`/`guarded_call`, applied to every exported function of all eight plugins by wrapping
  each in `*_inner`: panics are contained, the first is remembered, a crashed plugin's `step` returns 4
  and `halted` 1), `export_plugin!`. Tests: `abi.rs` layout pins, `tests/protocol_schema.rs`
  (fingerprint of `protocol.rs` per `PROTOCOL_VERSION` -- changing the wire schema fails it until the
  version is bumped and the new fingerprint recorded), `compat::tests`, `codec::tests`, `guard::tests`
  and the stub-plugin skew tests in `plugin_bridge::tests`. `--list-plugins` shows the host numbers
  and each plugin's ABI. Releases: `.github/scripts/package_release.sh` (Linux-verified) and the manual
  `release-artifacts.yml`. A real-library skew fixture and a host-side crash toast are not done.
  **(Superseded, see above) Feature-gated bundled machine plugins (2026-09-28)**: prompted by the
  same architecture reassessment that motivated the debug-contract cleanup
  above -- bundled mode (the default, and what actually ships; dynamic
  loading is opt-in, dev/deployment-only) previously linked all four
  machine plugins (`zx81`/`zx-spectrum`/`cpc`/`c64`) into one binary
  unconditionally, regardless of `--machine` selection, so there was no
  way to build a smaller single-machine binary even though the mechanism
  (a bundled/dynamic plugin split) nominally existed to make that
  possible. `apps/retroclawd/Cargo.toml` now declares each `*-machine-
  plugin` dependency `optional = true` behind its own cargo feature
  (`machine-zx81`/`machine-zx-spectrum`/`machine-cpc`/`machine-c64`,
  `dep:`-gated), all four still on by `default` so a plain `cargo build`
  produces the exact same binary contents as before -- this is additive,
  not a behavior change for the common case. `plugin_bridge.rs`'s
  `bundled_machine_plugin_entries` (previously a fixed `[(&str,
  PluginEntryV1); 4]` unconditionally referencing all four crates) is now
  a `Vec` built from four independently `#[cfg(feature = "machine-*")]`-
  gated pushes, so a build with a feature disabled genuinely doesn't
  compile that machine's plugin code into the binary at all -- verified
  live: `cargo build -p retroclawd --no-default-features --features
  machine-c64` links successfully, and the resulting binary's own
  `--list-machine`/`--help` correctly show only `c64` (with its real
  `--rom-kernal`/etc. options merged in), not the other three; `retroclawd
  zx81 --rom ...` on that same binary is a clean clap "unexpected
  argument" error, since the zx81 profile was genuinely never registered.
  Explicitly honest about what this does and doesn't fix: this changes
  binary size/link time/"can I build just one machine" (a real, previously
  entirely-missing capability), not necessarily runtime memory (RSS) for a
  normal single-machine run -- unreached code mostly never gets paged into
  the resident set either way, and Bevy's own footprint dwarfs any of this
  regardless, per the same reassessment. Debugger plugins (`debug-ui`/
  `debug-zrcp`/`debugger`) are unaffected -- always bundled, same as
  before; only the four machine plugins gained this feature-gating, since
  they were the ones actually flagged as "always linked regardless of
  selection."

  **Fast-path `step`/`halted`/`pc` ABI bypass (2026-09-28)**: a third fix
  from the same architecture reassessment, this one addressing speed
  rather than memory or debuggability -- prompted directly by "I don't
  care about a huge binary in the end, I care that it's accurate, fast,
  easy to debug and expandable." Investigation found every single
  emulated CPU instruction was paying a full `bincode`-encode + FFI call +
  `bincode`-decode round trip (`HostedMachine::step()` ->
  `MachineCommand::Step` -> `instance_call`), even in bundled mode where
  no real process/address-space boundary exists to justify one --
  confirmed via `emu_thread.rs`'s own tight `advance_one_frame` loop
  (`spent += m.step() as u64` once per instruction, up to
  `clock_hz()/frame_hz()` cycles' worth every tick, 50-60 times/sec). This
  is a real, measured architectural cost the plugin split introduced, not
  a hypothetical one -- distinct from the memory/debuggability findings
  above, which is why the fix is different in kind: not a genericity
  cleanup or a feature gate, but a second, narrower calling convention
  alongside the existing one. `plugin-api`'s `MachinePluginVTableV1`
  gained 3 new fields (`step`/`halted`/`pc`, raw `extern "C" fn(instance)
  -> {u32,u8,u16}` -- plain scalar in, plain scalar out, no request/
  response enum, no serialization at all) appended after the original 5
  fields; `ABI_MAJOR_V1`/`ABI_MINOR_V1` (0 -> 1) gate this safely -- see
  `abi.rs`'s own doc comment on `ABI_MINOR_V1` for exactly why appending
  fields to a `#[repr(C)]` struct is safe here specifically (the host's
  major/minor compatibility check runs strictly before any vtable memory
  access, so an old minor-0 plugin is cleanly rejected, never read
  through the new, larger struct shape). Every machine plugin
  (`zx81`/`zx-spectrum`/`cpc`/`c64`) implements the 3 new raw functions by
  reusing its own existing `state()`/instance-registry lookup, calling
  `Machine::step`/`halted`/`pc` directly with zero serialization;
  `plugin_bridge.rs` threads 3 new `step_fast`/`halted_fast`/`pc_fast`
  methods straight down `HostedMachine` -> `HostedBackend` ->
  `PluginV1MachineBackend` -> `PluginHost` -> the raw vtable call, and
  `impl Machine for HostedMachine`'s `step`/`halted`/`pc` now call these
  instead of `call_machine(MachineCommand::...)`. Deliberately scoped to
  only these 3 methods: `clock_hz`/`frame_hz` are called once per tick,
  not once per instruction, and `render_frame`/`drain_audio` are already
  heap-allocating `Vec<u8>`-returning calls at 50-60Hz, nowhere near hot
  -- no optimization without a measured bottleneck, same principle
  `vic-ii`'s own SIMD work already established. The slower
  `MachineCommand::Step`/`IsHalted`/`GetPc` protocol variants are left in
  place (still exercised by `DebugCommand`-driven paths inside each
  plugin, e.g. debugger step-over) -- this is an additional fast lane for
  the proven-hot calls, not a replacement of the general-purpose one.
  **`--debugger`/`--debugger-port`** (GUI-only, `debug_server`/`debugger`
  below) opens a browser-based interactive debugger: `debug_server.rs`
  (`tiny_http`, binds `127.0.0.1` only) serves one self-contained embedded
  page (`assets/debugger.html`, no CDN/external requests) plus a small
  JSON API it polls (`GET /api/state`, ~200ms) and posts commands to;
  `debugger.rs` is the shared vocabulary between it and the emu thread --
  a one-way `DebuggerCommand` `mpsc` channel (mirrors `input_tx`'s
  keyboard-event channel) and an outbound `Arc<Mutex<DebuggerState>>` the
  emu thread republishes every tick. Tabbed UI (CPU/Memory/Bitmap
  Explorer), redesigned 2026-09-17/18 to match a reference structure the
  user pointed at (the Atari ST Emulator's own web debugger, see
  `CREDITS.md`): CPU tab = disassembly (PC arrow, breakpoint dots,
  click-to-select, single-instruction assemble-and-poke) + registers/
  flags; Memory tab = full-page hex viewer with page/goto navigation;
  Bitmap Explorer tab = raw-memory-as-pixels viewer (start address/bytes-
  width/bitplane-count/zoom, wheel-to-scroll, Export PNG), same purpose
  as ASE's own Bitmap Explorer -- for hunting sprite/tile data anywhere
  in memory, not a live screenshot of the emulated display (the first
  redesign draft got this backwards -- built a `GET /api/screen.png`
  showing the current rendered frame via `emu_thread`'s `FrameBuffer`;
  user corrected it same day: "screens is not supposed to be a
  screenshot of current screen but a visual representation of the memory
  visualized as bitmaps... what ase can do and what is described in
  chapter 12" -- `screen.png`/`FrameBuffer` plumbing was removed
  entirely, replaced with this). `GET /api/memory-bytes?addr=&len=`
  (one-shot `DebuggerCommand::ReadMemory` reply, same shape as
  `SaveState`) hands the browser raw bytes; every bit of bitplane
  decode/palette/zoom/export happens client-side on a `<canvas>`, not on
  the server -- lets `Export PNG...` be a plain `canvas.toDataURL()`, no
  server-side PNG encoding needed at all. Interleaved-bitplane decode
  (classic Atari ST/Amiga row shape: `bitplanes` groups of `width` bytes
  per scanline, MSB-first bit order) verified pixel-exact against a
  hand-computed expected image during this redesign (Playwright +
  `getImageData`, both 1-bitplane and 4-bitplane cases), not just
  `DebuggerStateSnapshot` now carries machine-provided Bitmap Explorer palette
  colors and defaults through `plugin-api`; generic fallback colors remain for
  incomplete external plugins. Register editing, watches, conditional
  breakpoints, step-over, `.POK` import, reverse stepping, and ZRCP transport
  are implemented through the debugger plugin command/state contract.
  Layout (fixed 2026-09-18 same day, user feedback: "memory display need
  to be taller... disassembly, memory and bitmap explorer should be more
  dynamic to the height of the browser window. poke should go below
  registers instead"): every tab's main scrollable panel (disassembly/
  hex/bitmap canvas) is a flexbox chain from `body` down (`.main`/`.page`/
  `.grid2`/`.panel.fill` all `flex:1; min-height:0`) instead of a fixed
  `max-height: NNvh` guess, so it genuinely stretches and shrinks with the
  real window and scrolls its own overflow. Disassembly line count itself
  is now dynamic too, not just its container: `DebuggerCommand::
  SetDisassemblyLineCount` (`POST /api/disassembly-line-count`, clamped to
  `MAX_DISASSEMBLY_LINES` = 200 server-side) replaces the old fixed
  `DISASSEMBLY_LINES = 24` constant; the browser measures actual rendered
  row height against the panel's current `clientHeight` after every render
  (`measureAndSyncDisassemblyLineCount`, only while the CPU tab is
  visible -- a hidden `display:none` page reports `0` height) and only
  POSTs when the count actually changed, so a taller window shows more
  code instead of leaving blank space below a fixed-size list. The
  "Assemble & Poke" row moved out of the Disassembly panel into its own
  small panel directly under Registers (was previously below the
  disassembly table itself). Memory tab's default page size also bumped
  256->1024 bytes (16->64 rows) now that the panel scrolls its own
  overflow instead of being a short, non-scrolling block.
  Load/save reuses `Machine::save_state`/`load_state` and plugin host commands;
  snapshot/media dispatch also stays behind `plugin-api` machine instances.
  Building on fresh Linux dev machine needs `libwayland-dev` +
  `libudev-dev` system packages (`apt install`) for `bevy_winit`'s Wayland
  backend + `bevy_gilrs`' gamepad support respectively -- both pulled in
  transitively by `"2d"` feature. Running without real GPU (e.g.
  CI/sandboxed env) additionally needs `Xvfb` X server + software Vulkan
  driver (`mesa-vulkan-drivers`, giving `llvmpipe` software-rendering
  adapter) or panics w/ "Unable to find a GPU!".
  `apps/retroclawd/build.rs` embeds `assets/icon.ico` into Windows exe via
  `winresource` (`CARGO_CFG_TARGET_OS` check, not `#[cfg(windows)]`, since
  build scripts always compile for host — keeps cross-compiling to Windows
  from another host working correctly); no-op on other targets.
  `assets/icon.icns` exists for future macOS app bundle but not wired up
  anywhere yet -- bare CLI binary has no Finder icon of its own on macOS,
  only `.app` bundles do, no effect until bundled (likely GUI) app exists.

## Architecture guidance

Z80 emulation cores generally split into these concerns — keep decoupled so
CPU core stays testable in isolation from host/machine integration:

- **CPU state** — registers (main + shadow AF'/BC'/DE'/HL' set), PC, SP,
  IX/IY (half-register access IXH/IXL/IYH/IYL), interrupt mode (IM 0/1/2),
  IFF1/IFF2 flip-flops, halt state.
- **Instruction decode/dispatch** — Z80 opcodes not uniform-width; decoding
  must handle CB/ED/DD/FD prefix bytes (incl DD CB/FD CB four-byte
  displacement forms) correctly. Common approach: dispatch table per prefix
  rather than one giant match, since DD/FD mostly reuse unprefixed table w/
  IX/IY substituted for HL.
- **Flag behavior** — F register's undocumented bits 3/5 ("XF"/"YF" flags) +
  behavior on BIT/block instructions frequent source of inaccuracy; get
  correctness from trusted reference (e.g. Z80 undocumented-behavior docs),
  not intuition.
- **Memory/IO bus** — CPU core should be generic over bus (trait w/
  read/write for memory + IO ports) so can be driven by unit tests, bare
  memory array, or full machine (e.g. ZX Spectrum/CP/M host) w/o core
  knowing about host.
- **Cycle timing** — instruction timing (T-states) matters for any host
  needing cycle-accurate sync (video/audio); track T-states per
  instruction, incl extra cycles from prefixed/indexed addressing.
- **Test ROMs** — accuracy best validated vs known-good Z80 test suites
  (e.g. ZEXALL/ZEXDOC) rather than hand-written unit tests alone; consider
  wiring one up early as accuracy baseline.

## Agent skills

### Issue tracker

Issues live in GitHub Issues for `elpapimango/retroclawd` (via `gh`). See `docs/agents/issue-tracker.md`.

### Domain docs

Single-context: one `GLOSSARY.md` + `docs/adr/` at repo root. See `docs/agents/domain.md`.
