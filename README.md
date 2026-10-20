<p align="center">
  <img src="assets/icon-256.png" width="128" alt="retroclawd icon">
</p>

# retroclawd

> **Work in progress. Use at your own risk.** This is an active,
> unfinished personal project -- behavior, flags, and file formats can
> change without notice, and things can break. Not accepting outside
> code contributions or issue reports at this time; please don't open
> pull requests or issues on this repo.

A multi-machine retro-computer emulator written in Rust. Each machine is a shared library
(`*_machine_plugin`) that the `retroclawd` application loads at run time; the application draws the
picture in a Bevy window, plays the sound, handles keyboards, gamepads, hotkeys, media and the debuggers,
and links no machine code itself.

Machines (each family has its own repository, see Building):

| Family | Machines | CPU |
|---|---|---|
| Sinclair | ZX81; ZX Spectrum 48K, 128K, +2, +2A, +3 | Z80 |
| Amstrad | CPC464, CPC664, CPC6128, CPC464+/6128+, GX4000 | Z80 |
| Commodore | Experimental VIC-20; C64, C128 (8502 and Z80, 40/80 columns), 1541/1570/1571/1581 drives | 6502, 6510/8502, Z80 |

The CPU cores (Z80, validated with ZEXALL/ZEXDOC; NMOS 6502 with the illegal opcodes, validated with Klaus
Dormann's and Tom Seddon's suites) and the chips several families share (AY-3-8910, uPD765A) live in
`retroclawd-crates`. In short, the machines have: a real keyboard matrix, video and audio (the Spectrum's
beeper and AY, the CPC's PSG, the C64/C128's SID), real-time tape playback (`.tap`/`.tzx`/`.cdt`/`.pzx`/
`.csw`/`.voc`/`.wav`, C64 `.tap`/`.t64`) with a turbo multiplier and fast-load traps, floppy images (`.dsk`/`.edsk`,
`.d64`/`.d71`/`.d81` and more, writable with `--disk-save`/`--disk-writeback`), cartridges (C64 `.crt`, CPC+
`.cpr`), snapshots (`.z80`, `.sna`, `.szx`, `.p`/`.p81`/`.81`) and this project's own `.state` save states.
The application adds zip and URL inputs, an optional input-latency "run-ahead" mode, CRT shaders, gamepads,
remappable keys and hotkeys, file dialogs, clipboard paste, and a browser debugger with disassembly and symbol
names for the Z80 and 6502 machines. Per-machine detail is in each family repository's `README.md` and
`CLAUDE.md`.

## Building

**One command** builds the app and all machines:

```
git clone https://github.com/elpapimango/retroclawd
cd retroclawd
tools/build_all.sh            # release; add --debug for a debug build
```

On Windows use PowerShell: `tools\build_all.ps1` (same arguments; also `smoke_test.ps1` and `bump_pin.ps1`). The `.ps1`
files only start the bash scripts through Git for Windows' bash, so you need Git for Windows and Rust installed, nothing
else. (The `.sh` files keep LF line endings through `.gitattributes`; a CRLF copy fails in bash.) The Windows route was run (build and
smoke test) and passes.

The script checks out the submodules (`shared/` and the three machine repositories under `machines/`),
builds the app, builds each family, and copies every machine library into
`target/release/plugins/` -- the folder the app searches next to its executable. Afterwards
`target/release/retroclawd zx81 --rom <your ZX81 ROM>` just works.
`--families commodore` builds only one family; `--update` first moves the submodules to the newest
commit of each family's `main` (otherwise you get the commits this repository records).

How the repositories relate (all public, same account):

| Repository | Holds |
|---|---|
| [`retroclawd`](https://github.com/elpapimango/retroclawd) | the application (with its debuggers), the build/smoke/bump scripts |
| [`retroclawd-crates`](https://github.com/elpapimango/retroclawd-crates) (`shared/`) | the shared crates: `machine`, `plugin-api` (ABI policy), the Z80 and 6502 cores, AY-3-8910, uPD765A, `.dsk`, tape formats |
| [`retroclawd-sinclair`](https://github.com/elpapimango/retroclawd-sinclair) (`machines/sinclair`) | ZX81, ZX Spectrum 48K/128K/+2/+2A/+3 |
| [`retroclawd-amstrad`](https://github.com/elpapimango/retroclawd-amstrad) (`machines/amstrad`) | Amstrad CPC464/664/6128, CPC+ and GX4000 |
| [`retroclawd-commodore`](https://github.com/elpapimango/retroclawd-commodore) (`machines/commodore`) | Experimental VIC-20, C64, C128 and the 1541/1570/1571/1581 drives |

- The application uses the shared crates **by path** through the `shared/` submodule and contains no machine
  code; it loads the machine libraries (`*_machine_plugin`) at run time. Removing a machine is deleting its library.
- Each machine repository is a separate Cargo workspace that depends on `retroclawd-crates` **by git
  revision** (`rev = "..."` in its manifests) and builds its own libraries. It builds on its own too
  (`cd machines/amstrad && cargo build --release --workspace`; or clone just that repository).
- The ROMs and test software live in the machine repositories (`roms/`, `software/`).
- Each submodule records one commit, so a checkout is reproducible.
- Changing a shared crate: commit and push it in `shared/`, then run `tools/bump_pin.sh` (it checks the commit is
  on GitHub, prints the plugin ABI/protocol diff, rewrites the families' pins, runs their tests and records the
  new submodule commits; `--push` publishes the family commits). The app and the libraries must agree on
  `plugin-api`'s ABI (`shared/docs/plugin-abi.md`).
- `tools/smoke_test.sh` builds everything, boots every machine headless and checks the boot screens against
  recorded hashes -- run it after changing a shared crate or a pin.
- Cargo fetches `retroclawd-crates` by git (the scripts set `CARGO_NET_GIT_FETCH_WITH_CLI=true`).

On Linux, building the GUI needs two system packages (`apt install
libwayland-dev libudev-dev` on Debian/Ubuntu) for Bevy's windowing and
gamepad support -- Windows and macOS builds don't need anything extra.

## Tools

[`retroclawd-tools`](https://github.com/elpapimango/retroclawd-tools) (builds on its own; not part of this
repository's build) holds command-line tools on the same crates: `rcasm`, a two-pass 6502/Z80 assembler whose symbol
files `--symbols` reads, and `rcdis`, a disassembler that writes source `rcasm` assembles back to the same bytes. Disk
and tape tools (`rcdisk`, `rctape`) and a linker (`rcld`) follow.

## Running

```
target/release/retroclawd <machine> [--rom <path-to-rom>] [options]      # after tools/build_all.sh
```

`<machine>` is a positional argument and must come first (e.g. `retroclawd
cpc6128 --rom machines/amstrad/roms/cpc/cpc6128.rom`), the same convention `git`/`cargo`/`docker` use
for their own leading subcommand-like argument. Machine ids are whatever libraries were found
(`retroclawd -lm` prints the live list): `zx81`, `zx-spectrum`, `zx-spectrum-128`, `zx-spectrum-2a3`, `cpc464`,
`cpc664`, `cpc6128`, `cpc464plus`, `cpc6128plus`, `gx4000`, `c64`, `c128`. The machine (and every other setting)
can also come entirely from `--game-config`/`retroclawd.toml` -- see
**Machine-specific options** below -- so `retroclawd cpc6128 --disk
game.dsk` works with no `--rom` flag at all once that machine's ROM path is
configured in `retroclawd.toml` (copy `retroclawd.example.toml`, which points at `machines/<family>/roms/` for your own dumps; the
+2A/+3 ROM has no default path, so `zx-spectrum-2a3` needs `--rom`).

This opens a live window showing the machine's video output, booting
straight from the ROM's reset vector (same as powering on real hardware).
The host keyboard types on the emulated one by physical position, or by the character typed
(`layout = "symbolic"`), and can be remapped; gamepads drive the machine's joystick -- see **Key bindings**,
**Keyboard layouts** and **Gamepads** below. On the Spectrum and CPC the arrow keys and Space also
act as joystick 1 (Kempston on the Spectrum by default), alongside their normal meaning.
Running with no arguments at all (e.g. double-clicking the exe with no console attached)
prints usage and exits cleanly instead of erroring.

ROM images are copyrighted and are not bundled with the application -- supply your own dump. Each family's
`docs/firmware.md` says where each file goes and the exact size the machine expects; `--firmware auto` can fetch
them from a RomM server by hash.

### Machine-specific options

Every machine plugin declares its own options -- ROM paths, and anything
meaningful only to that machine family (e.g. CPC's tape motor policy/RAM
expansion/upper-ROM slots, ZX Spectrum's speaker/joystick/Turbosound
choices) -- rather than the host hardcoding every machine's flags. The host
only owns options that apply the same way across every machine (shaders,
debugger plugin selection, dump/state flags, `--headless`, etc.).

Two ways to see a given machine's own options:

```
retroclawd cpc6128 --help     # common flags + cpc6128's own options
retroclawd -lp                # every discovered plugin's options, machine and debugger
```

A flag only exists in a given machine's parsed command line if that
machine actually declares it -- e.g. `--ram-banks` only parses under
`cpc6128`/`cpc6128plus` (the only real hardware with that expansion slot);
`retroclawd cpc464 --ram-banks 2` is a clean clap "unexpected argument"
error, not a silent no-op.

Every plugin-declared option can also be set in `retroclawd.toml` directly in
the machine's `[machines.<id>]` section (flat, beside shader/fullscreen/vsync/
max_fps -- there is no nested options table), or in a `--game-config` file's own `[options]`
table (which overrides `retroclawd.toml`'s value for the same key) --
precedence is CLI flag > `--game-config`'s `[options]` >
`retroclawd.toml`'s `[machines.<id>]` > the plugin's own default.
An unrecognized key in either toml table is a clear runtime error naming
the real available options for that machine, not a silent no-op. See the
committed `retroclawd.toml` for commented-out examples for every machine.

A repeatable numbered option (CPC's upper-ROM expansion slots, `--rom-slot
N=path` on the CLI) becomes a nested slot-number table in toml instead of
a single value:

```toml
[machines.cpc6128]
rom = "machines/amstrad/roms/cpc/cpc6128.rom"
ram_banks = 3

[machines.cpc6128.rom_slot]
7 = "amsdos.rom"
```

### RomM (firmware and games from your own server)

`retroclawd` can read firmware and games from a [RomM](https://romm.app) server you run. Set `ROMM_URL` and
`ROMM_TOKEN` (a read-only client API token) in the environment, or `[romm] url` / `token` in `retroclawd.toml`
(keep the token out of anything you commit; the environment variables win).

```
retroclawd romm platforms [--all]        # platforms with ROMs or firmware (--all: every platform)
retroclawd romm games zx-spectrum        # a platform by id, slug or name; --search <text> narrows it
retroclawd romm firmware [<platform>]    # id, name, size and SHA-1, as an aligned table
retroclawd romm match [<machine>]        # which firmware on the server satisfies which machine option
retroclawd cache list | clean (--days N | --all) | remove romm:<id>...
```

Copy an id into the next command: `romm:<id>` is a game and `romm:fw/<id>` a firmware file, accepted wherever a file
is (`--disk romm:42`, `--rom romm:fw/7`, `media = "romm:42"` in a game config). Downloads are cached (move the cache
with `RETROCLAWD_CACHE_DIR`; `cache clean --days 90` removes what has not been used for 90 days).

**`--firmware auto`** (or `[romm] firmware = "auto"`) fills every ROM option you did not name -- from a flag, the
game config or `retroclawd.toml` -- from the server's firmware, matched by SHA-1: `retroclawd c64 --firmware auto`
needs no `--rom-*` flag at all. Which dump satisfies which option is data: each machine family has a `firmware.toml`
(hashes only), staged next to its libraries as `<family>.firmware.toml` by `tools/build_all.sh`. A downloaded file is
checked against the hash it was chosen by before a machine sees it, and `retroclawd romm match c128` shows what
would be used (the C128's two BASIC halves are joined into the one image it needs).

### Batch mode (`--headless`)

For scripting, dumps, or debugging without a window, add `--headless`:

```
retroclawd zx81 --rom machines/sinclair/roms/zx81/zx81.rom --headless --max-cycles 2000000 --dump-screen screen.png
```

This runs to completion in the terminal instead of opening a window --
same behavior this tool always had before the GUI existed. Without
`--headless`, a real ROM never executes `HALT` (no keyboard input in
batch mode means it spins forever) -- `--max-cycles` caps it, or Ctrl+C.

### Flags

**Core (common to every machine)**
- `<machine>` -- positional, must come first (required, either here or as
  `machine` in `--game-config`)
- `--region <pal|ntsc|japan>` -- affects clock speed and frame rate
  (default `pal`, or `region` in `--game-config`)
- `--game-config <path>` -- per-game `.toml` file supplying
  `machine`/`region`/`media`/`tape_speed`, plus an `[options]` table for
  plugin-declared options (see **Machine-specific options** above); any of
  those also given directly on the command line overrides the toml's value
  for that setting, rather than conflicting with it. It also takes, for this game only, a `[keys]` table (host key ->
  machine key name(s), layered over `retroclawd.toml`'s `[keys]` and the machine's own), `layout = "symbolic"`
  (`--layout` still wins), `symbols = "game.sym"` (or a list; loaded before `--symbols`), a `[hotkeys]` table
  (`action = "chord"`, a list or `"none"`, plus `modifier`; `--hotkey` still wins) and `[pads]`/`[pads.1]`
  (`button = "machine key"`, plus `deadzone`). Each replaces only the actions / buttons it names
- `--version` (`-V`) -- print the version, build number and git commit of retroclawd and of every machine plugin found, and exit; the
  same lines head `--help`, and `--list-plugins` shows them per plugin. A line reads `retroclawd 0.1.0 build 39 (6bdc7515ff, 2026-10-17)`:
  the **build number** is the number of commits in the repository up to the one built from, the hash and date are that **commit**'s (the
  one that was pushed, once it has been), and `dirty` is added when tracked files differ from it. The host reports the retroclawd
  repository, each plugin the machine repository it was built in (sinclair, amstrad, commodore), so a stale plugin next to a new
  executable shows at a glance. Built without git, set `RC_BUILD_NUMBER`, `RC_GIT_COMMIT`, `RC_GIT_DATE` (and `RC_GIT_DIRTY=1`).
- `--list-machine` (alias `-lm`) -- print discovered machine ids and exit
- `--list-plugins` (alias `-lp`) -- print discovered machine/debugger
  plugin libraries and exit (debugger entries include selector names and
  available options; machine entries include each machine's own declared
  options)
- `--plugin <selector>` -- repeatable plugin selector; supported forms:
  `debug-ui[:OPT[:OPT...]]`, `debug-log[:OPT[:OPT...]]`,
  `debug-zrcp[:OPT[:OPT...]]` (legacy alias `debug-tcp[:OPT[:OPT...]]`),
  `debugger[:OPT[:OPT...]]` (legacy alias for `debug-ui`), `machine=<path>`, or plain `<path>` (the debuggers take options, never a path: they are built in)
- Every machine is a plugin library (`*_machine_plugin.dll` / `.so` / `.dylib`) that `retroclawd`
  loads at startup from `plugins/` next to the executable, the executable's own folder or `./plugins`
  (override with `RETROCLAWD_MACHINE_PLUGIN_DIRS`). `cargo build --workspace` builds the app and
  all the libraries; copy `retroclawd` plus the libraries you want, and delete a library to remove
  that machine. The debuggers (`debug-ui`, `debug-zrcp`, `debug-log`) are not plugins: they are
  modules of the app, each behind a cargo feature of the same name (on by default).
- `--program <path>` -- optional standalone program to load and run
  instead of booting the ROM, with `--org <0xNNNN>` for its load address
  (default `0x4000`) -- works in both GUI and `--headless` mode; mutually
  exclusive with `--snapshot`/`--tape`/`--load-state`

**Hotkeys**

Emulator actions are bound to key chords in `retroclawd.toml` -- `[hotkeys]`
for every machine, `[machines.<id>.hotkeys]` to override per machine
(`retroclawd --list-hotkeys [machine]` prints the effective table; `--hotkey ACTION=CHORD`, repeatable, binds one
for a single run: `--hotkey pause=ctrl+KC_P --hotkey reset=none --hotkey modifier=alt_right`, the first flag for an
action replacing what the config bound and later ones adding chords). Layers:
built-in defaults, then `[hotkeys]`, then the machine's table, per binding;
`"none"` unbinds; a list binds several chords (`reset = ["KC_F12", "ctrl+KC_R"]`).
Host keys are written as [QMK keycodes](https://docs.qmk.fm/keycodes_basic)
(`KC_F12`, `KC_PAUS`, `KC_PSCR`, `KC_RALT`, `KC_GRV`, ...; case does not matter,
long forms such as `KC_ENTER` work, and the older spellings `f12`/`alt_right`
still parse). A chord is `key` or `modifier+key` with `ctrl`, `shift`, `alt`,
`super` (or `KC_LCTL`, `KC_LSFT`, `KC_LALT`, `KC_LGUI`; either physical side
matches, and modifiers are matched exactly): `KC_F12`, `shift+KC_F11`,
`ctrl+shift+KC_Q`. QMK's shifted keycodes are chords too: `KC_PLUS` is Shift and
the `=` key, `KC_AT` Shift and `2`, `KC_COLN`, `KC_DQUO`, `KC_UNDS`, ... A key
used by a hotkey is not also sent to the emulated machine.

| Action | Default | What it does |
|---|---|---|
| `reset` | `KC_F12` | reset the machine (CPU only, RAM kept) |
| `fullscreen` | `KC_F11` | toggle borderless fullscreen |
| `mute` / `volume_down` / `volume_up` | `KC_F9` / `ctrl+KC_F9` / `ctrl+KC_F10` | audio mute, volume in 10% steps |
| `overlay` | `KC_F10` | show / hide the debug overlay (cpu panel, or the `--show-overlay` panels) |
| `version` | `KC_LSFT` + `KC_F9` | show / hide the version panel in the overlay (shows the overlay too if it was hidden) |
| `pause` | `KC_PAUS` | pause / resume emulation |
| `screenshot` | `KC_PSCR` | save `screenshots/<machine>-<date>-<time>.png` (UTC) |
| `quick_save` / `quick_load` | `shift+KC_F11` / `shift+KC_F12` | save / load `states/<machine>-slotN.state`, or `states/<machine>-<game>-slotN.state` when a game was loaded (the file stem of the media: `--tape`/`--disk`/`--snapshot`, a game config's `media`, `--program`, `--cartridge`), so games do not share slots; screenshots are named the same way |
| `state_slot_next` / `state_slot_prev` | `shift+KC_F10` / unbound | pick the slot, 0-9 |
| `fast_forward` | `KC_SCRL` (hold) | run 8x faster while held (also tape loading), audio off |
| `shader_next` / `shader_prev` | `ctrl+KC_F11` / unbound | cycle the bundled shaders |
| `paste` | `shift+KC_INS` | type the clipboard's text into the machine (again to cancel; see **Paste**) |
| `quit` | `ctrl+KC_F12` | quit |
| `load_tape` / `load_disk` | `ctrl+shift+KC_F9` / `ctrl+shift+KC_F10` | file dialog: put a tape / disk in (see "File dialogs") |
| `load_snapshot` / `save_snapshot` | `ctrl+shift+KC_F11` / `ctrl+shift+KC_F12` | file dialog: load / save a snapshot |
| `eject_tape` / `eject_disk` | unbound | take the tape / disk out |
| `load_disk_b` / `eject_disk_b` / `save_disk_b` | unbound | the same for the second drive, drive B (CPC664/6128 and Spectrum +3): choose a disk image for it, take it out, save a copy; replacing or ejecting a drive B disk with unsaved changes is refused unless `--disk-b-save` / `--disk-writeback` can write them first |
| `load_state` / `save_state` | unbound | file dialog: any `.state` file (quick-save slots stay on `quick_save`/`quick_load`) |
| `save_tape` / `save_disk` | unbound | save the cassette recording (`.cdt/.tzx/.voc`, CPC) / a copy of the disk image |
| `freeze` | `ctrl+KC_PAUS` | press the freeze button of a C64 cartridge that has one |
| `next_disk` / `prev_disk` | unbound | swap the disk of an `.m3u` playlist |

The defaults stay on F9-F12, Pause, Print Screen and Scroll Lock because no
emulated machine uses them (the C64's F1/F3/F5/F7 stay the machine's). For a
machine that needs those too, or to avoid clashes while typing, set a hold
modifier: `[hotkeys]` `modifier = "KC_RALT"` makes every hotkey fire only
while Right Alt is held (on Linux/X11 avoid combining it with `ctrl+F1`..`F12`
chords: the X server treats Ctrl+Alt+Fn as a VT switch and never delivers the
key). Quick-save files are this project's internal
`.state` format and need the same `--rom`/`--tape`/`--disk` on load.

```toml
[hotkeys]
modifier = "KC_RALT"         # optional: hold this with a hotkey
mute = ["KC_F9", "KC_M"]
quit = "none"

[machines.c64.hotkeys]
reset = "ctrl+KC_F12"
```

**File dialogs**

The file hotkeys above open a native file dialog ([`rfd`](https://docs.rs/rfd);
on Linux the XDG desktop portal, so a portal backend such as
`xdg-desktop-portal-gtk` or `-kde` must be running) so you can swap a tape or
disk, load or save snapshots and states, and save a recorded cassette or a copy
of the disk image while the machine runs.

- The machine is held still and silent while a dialog is open, and keys typed
  meanwhile reach neither the machine nor the hotkeys. Cancelling changes
  nothing.
- A dialog opens in the folder last used for that kind of file (tape, disk,
  snapshot, state), remembered in `retroclawd.state.toml` next to
  `retroclawd.toml` (the config is never rewritten); the first time, in the
  folder of the file the session started with, else the working directory. The
  filter shows the extensions that machine takes.
- Putting in a disk or ejecting one the program has written to asks first, in
  the window (the portal has no message boxes): `[S]` save as, `[O]` overwrite
  the original file (when it is known), `[D]` discard the changes, `[Esc]`
  cancel. A disk swapped in later is covered by the same exit policy as the
  `--disk` one: with `--disk-writeback` its own file, with `--disk-save` that
  path, otherwise a warning on exit if it was modified -- never a silent loss.
  `save_disk` saves a *copy* and leaves that exit policy alone.
- `eject_*` and `load_*` are plugin commands (`DetachMedia`/`AttachMedia`);
  results are shown as one-line messages ("Disk loaded: game.dsk", "Tape load
  failed: ..."). Tape and disk slots exist where the machine has them (ZX81:
  snapshots only; C64 has no snapshots; disks on the +3, CPC664/6128 and C64;
  only drive 0 exists).
- In a `--headless --frames` run the same actions are `--at FRAME:ACTION[=FILE]` (see below): no dialog, the file is named
  on the command line.
- Not done: drag and drop, a recent-files list,
  more than one drive. macOS and Windows use rfd's native dialogs, which were not
  exercised here (this was built and tested on Linux without a desktop portal:
  the hotkeys, the machine pause, the prompt and every command behind them are
  tested, a real dialog window was not seen).

**Paste**

`shift+KC_INS` (the `paste` hotkey) types the system clipboard's text into the machine, one key at a time
(`paste_delay_ms`, default 60, per key press and release), using the same character-to-key table as the
symbolic layout, so it works on every machine. Line breaks become Enter, tabs and no-break spaces become
spaces, typographic quotes and dashes become ASCII, and a character the machine has no key for is skipped
and counted in the message. Pressing it again cancels. It waits while a host Shift/Ctrl/Alt/Super is held.
`[input] paste_case` (or per machine) is `"fold"` (default; every letter is the plain key -- an uppercase
BASIC listing on the C64) or `"exact"` (a capital holds the machine's Shift -- right for Spectrum/CPC text).

**Zip and URL inputs**

Every file the application reads -- ROMs, `--program`, `--disk`, `--tape`, `--snapshot`, `--load-state`,
`--cartridge`, `--game-config`, the `media` of a game config, an `.m3u` playlist and its entries, `--symbols`
and the load dialogs -- can be a path, an `http://`/`https://` URL, a `.zip`, or a `.zip` at a URL. The name
inside the zip (else the URL's file name) decides the format, so `Game.zip` holding `Game.d81` is a `.d81`.
A zip with one file just works; `archive.zip#entry` (also on URLs) picks one file of a zip with several,
and without `#entry` such a zip is an error listing its entries. Unpacked files live in a temporary folder
that is removed when the program exits (a folder left by a killed run is swept at the next start).
`--progress` prints download and unpack progress on stderr. `--disk-writeback` refuses a URL or zip source
(use `--disk-save <file>`). `.m3u` entries may be paths, URLs or `zip#entry`.

**Debugger symbols**

The debugger's disassembly shows a name after an address operand (`JSR $FFD2  ; CHROUT`,
`CALL 0x0556  ; LD_BYTES`). The C64/C128 (KERNAL jump table, vectors, VIC/SID/CIA registers) and the ZX Spectrum
(48K ROM entry points, system variables) have built-in names; `--symbols <file>` (repeatable, later files win)
adds your own from an assembler listing (`NAME = $1234`, `NAME EQU 0x1234`), a map file (`NAME 1234`) or VICE
labels (`al C:1234 .NAME`). Z80 `JR`/`DJNZ` and 6502 branches show their absolute target.

**Key bindings**

`[keys]` in `retroclawd.toml` changes which emulated key a host key presses,
for every machine; `[machines.<id>.keys]` overrides it per machine (per
binding). `retroclawd <machine> --list-keys` prints the machine's key names
(with their aliases), the host key names, and the bindings in effect.

```toml
[keys]
KC_SCLN = "colon"                   # host key = machine key
KC_GRV = "none"                     # the key does nothing
KC_F = ["caps_shift", "1"]          # a list presses several machine keys together
KC_AT = "at"                        # QMK's KC_AT is Shift+2: the machine gets its `at`

[machines.zx-spectrum.keys]
KC_CAPS = "symbol_shift"
"shift+KC_7" = "apostrophe"

[machines.c64.keys]
KC_LBRC = "at"
```

- **Left side:** a host key or a chord, written like a hotkey (`KC_SCLN`,
  `KC_AT`, `ctrl+KC_F1`; `retroclawd ... --list-keys` lists the names).
  A chord's modifiers are matched exactly, and while a chord is held its
  modifiers are *not* sent to the machine, so `shift+2 = "at"` presses `At`
  and not `At` plus a stray Shift. A plain binding (`f = "b"`) still lets a
  held Shift through. Modifier keys themselves only take plain bindings.
- **Right side:** a name the machine reports, checked at startup. Case and
  `_` don't matter (`symbol_shift` = `SymbolShift`), and `a`, `1`, `left` work
  for `KeyA`, `Digit1`, `ArrowLeft`. A typo is an error listing the valid names.
- **Layers:** `[keys]` then `[machines.<id>.keys]`, per chord; a host key with
  no binding sends its own name, which every machine understands natively.
  A binding on a hotkey's chord (F12 by default) is an error: unbind or move
  the hotkey first.
- This replaces the old `[keymap.default]` / `[keymap.machines.<id>]`
  tables, which now produce an error pointing here.

**Keyboard layouts and keymap files**

By default a host key is the machine key at the same *physical position*
(`positional`): on a German or French keyboard the key labelled `Z`/`A` is in the
QWERTY `Y`/`Q` place and so types `Y`/`Q` on the emulated machine. Set
`layout = "symbolic"` and what the key *types* decides instead:

```toml
[input]
layout = "symbolic"        # or --layout symbolic; "positional" is the default

[machines.zx-spectrum.input]
layout = "positional"      # per machine
keymap = "keymaps/spectrum.toml"
```

- **What it does:** letters and digits are matched by the character typed
  (`Shift`+`&` on AZERTY types `1`, so the machine gets `1` without a stray Shift;
  a capital letter keeps Shift). Punctuation is matched through the characters
  each machine plugin declares its keys type (`!`, `"`, `;`, `[`, ... -- ZX
  Spectrum and C64 cover the whole punctuation row, the CPC only `, . /`, the ZX81
  letters, digits and space), and the host's Shift/AltGr used to type it are not
  also sent. A character the machine has no key for falls back to the physical
  key, exactly as in the positional layout -- except an accented letter, which types its plain letter (`é` -> `e`, `Ü` -> `U`, `ß` -> `ss`
  when pasted), and a dead key (an accent waiting for its letter), which types nothing by itself so the composed letter that follows is
  the only thing the machine sees. Paste folds accents the same way.
- **Explicit bindings still win:** a plain `[keys]` entry for a physical key beats the
  layout; chords, hotkeys and pad bindings are always physical.
- **Not covered:** dead keys and non-ASCII characters (`é`, `ß`) use the physical
  key; the typed character is read as each key goes down, so switching the host
  layout while the emulator runs just works.
- `retroclawd <machine> --list-keys` shows the layout and keymap files in effect.

**US keyboard on the C64:** `keyboard/us_c64.toml` (loaded by `retroclawd.toml`'s
`[machines.c64.input] keymap`) makes a US keyboard type what its keycaps say: `+`
(Shift and `=`), `:`, `@`, `*`, `^`, `_`, quotes, brackets and the shifted number
row. Without it the positional default sends Shift+`=` as the C64's Shift and `=`,
which types nothing useful, because the C64 has its own `+`, `:`, `@`, `*` keys and
a different Shift row (Shift+2 is `"` there). Copy it as the starting point for
another keyboard.

A **keymap file** is a standalone TOML file with the same `[keys]` /
`[machines.<id>.keys]` tables as `retroclawd.toml` plus an optional `layout`, so a
mapping can be kept per keyboard or shared:

```toml
# keymaps/german.toml
layout = "symbolic"
[keys]
KC_GRV = "none"
[machines.c64.keys]
"shift+KC_7" = "slash"  # chords here are host chords, like in retroclawd.toml
```

Files are listed in `[input] keymap = "file.toml"` (or a list; paths relative to the
config file) or given with `--keymap file.toml` (repeatable; relative to the working
directory). Layers, lowest first: files from `[input] keymap` (global, then the
machine's), `[keys]`, `[machines.<id>.keys]`, then `--keymap` files. The layout is
`--layout`, else `[machines.<id>.input] layout`, else `[input] layout`, else the
last keymap file that names one, else positional. Errors name the file.

**Gamepads and joysticks**

Connected gamepads (any pad the OS exposes, via Bevy's gilrs backend) drive
the machine's joystick with no configuration: pad 1 is `Joy1*`, pad 2 is
`Joy2*`. D-pad and left stick are the directions, South/East (A/B) are
`Fire1`, West/North (X/Y) are `Fire2`/`Fire3` where the machine has them.

| Machine | Joystick keys the pads reach |
|---|---|
| ZX Spectrum (all models) | `Joy1Up/Down/Left/Right/Fire1` -> whichever joystick `joystick_type` selects (Kempston, Fuller, Sinclair 1/2, Cursor) |
| Amstrad CPC / CPC+ / GX4000 | `Joy1*` and `Joy2*` (`Fire1`-`Fire3`) on the keyboard matrix |
| Commodore 64 | `Joy1*` (control port 1, shares CIA1 Port B with the keyboard) and `Joy2*` (port 2, what most games read), `Fire1` only |
| ZX81 | no joystick |

A pad can also trigger the host actions of `[hotkeys]`: write the buttons held together as a chord, `pad1:select+start` (one pad) or
`pad:select+start` (any pad), beside the keyboard chords -- `reset = ["f12", "pad1:select+start"]`. The action fires when the last
button goes down, a hold action (`fast_forward`) lasts while the chord is held, and the chord's buttons are hidden from the machine
until they are let go (the first one of two reaches the machine a moment before the second arrives, as a key would). `pad:` chords
need no `modifier` key. `--list-hotkeys` shows them.

`[pads]` rebinds buttons; `[machines.<id>.pads]` overrides per machine. Pad
slots are `[pads.1]`..`[pads.4]` (first pad seen = 1; unplugging one does not
renumber the others). A button can press any machine key, not only joystick
ones -- a list presses several, `"none"` unbinds. `retroclawd <machine>
--list-keys` also prints the button names and the pad bindings in effect.

```toml
[pads]
deadzone = 0.5               # how far a stick must move to count (0..1)

[pads.1]
south = "Joy1Fire1"
start = "Enter"              # any machine key works
l1 = ["caps_shift", "1"]     # a list presses several together
east = "none"

[machines.c64.pads.1]
west = "Joy2Fire1"           # e.g. play port 2 games on pad 1
north = "Joy2Up"
```

Buttons: `up down left right` (D-pad), `south east west north` (also `a b x y`),
`l1 r1 l2 r2`, `select start`, `l3 r3`, and `stick_up stick_down stick_left
stick_right` (left stick past the deadzone). Not done: pad buttons cannot
trigger hotkeys yet, and the right stick is unused. The arrow keys and Space
still double as the Spectrum/CPC joystick as before.

**Batch mode (`--headless` only)**
- `--headless` -- batch mode (see above) instead of opening a window
- `--max-cycles <N>` (alias `-mc`) -- stop after N instructions even if
  never halted
- `--frames <N>` -- run N emulated frames the way the window does (a frame
  interrupt each, no stop at `HALT`), then dump; for ROMs that `HALT` while
  they wait for the next frame (the ZX81, `EI`/`HALT` game loops).
  Conflicts with `--max-cycles`
- `--type <TEXT>` (needs `--frames`) -- type TEXT on the emulated keyboard, the way the paste hotkey does (the machine's own
  key names, shifts and pacing); `\n` is Enter. `--type-after <FRAMES>` (default 150) is how long to wait for the prompt: a
  C128 with a real drive (`--true-drive`) probes it for a boot sector first and needs about 500. `--type-delay <MS>` sets the time
  between key events (default: the paste delay, 60); the ZX81 ROM scans keys once a frame and loses them that fast: use 160. For example
  `retroclawd c128 ... --headless --frames 400 --type 'PRINT 12*34\n' --dump-memory 0x0400:1000` leaves `408` on the screen
- `--at <FRAME:ACTION[=FILE]>` (repeatable, needs `--frames`) -- run a hotkey or file-dialog action at the start of a frame
  (`FRAMES` itself means after the last frame, before the dumps): `reset`, `freeze`, `screenshot[=FILE]`, `quick_save`,
  `quick_load`, `state_slot_next`, `state_slot_prev`, `load_tape|load_disk|load_snapshot|load_state=FILE`, `eject_tape`,
  `eject_disk`, `save_snapshot|save_state|save_tape|save_disk=FILE`, and `key_down=NAME` / `key_up=NAME` (a machine key by the
  name `--list-keys` prints, e.g. `Joy1Up`, held for as many frames as you like; an unknown name fails the run). It is the window's own code (a quick save goes to
  `states/` as usual) with the file named here instead of asked for; each action prints `frame N: <message>`, and a failed one
  (missing file, nothing to save) makes the run exit 1. Actions with no meaning without a window (`pause`, `mute`, `quit`...)
  are refused. For example `--frames 300 --at 100:save_state=a.state --at 200:reset --at 250:load_state=a.state`
- `--dump-audio <file.wav>` (needs `--frames`) -- write everything the machine played as a mono 16-bit 44.1 kHz `.wav`, to check a
  tone's frequency or silence without a sound card
- `--dump-memory <START:LEN>` (alias `-dm`) -- hex-dump memory after the
  run stops, e.g. `--dump-memory 0x4000:16`
- `--dump-screen <path>` -- write the current video frame as a PNG after
  the run stops
- `--dump-snapshot <path>` -- save the machine's registers/memory to a
  snapshot file after the run stops, format by extension (`.z80` or
  `.sna`) -- the save-side counterpart to `--snapshot`
- `--dump-state <path>` -- save the machine's full internal state
  (registers, memory, every peripheral's own live timing state) to this
  project's own `.state` format after the run stops -- the save-side
  counterpart to `--load-state`; never interoperates with other
  emulators, purely for resuming later

**Snapshots and save states**
- `--snapshot <path>` -- load a snapshot file, replacing the machine's
  registers/memory outright instead of booting the ROM. Format by
  extension: `.z80`/`.sna` for `zx-spectrum*`, `.p`/`.p81`/`.81` for
  `zx81`. Conflicts with a standalone program and `--load-state`
- `--load-state <path>` -- load a previously-saved `--dump-state` file,
  restoring the machine's full internal state outright. Requires the same
  ROM (and `--tape`/`--disk`, if the saved state had one attached) to
  already be given. Conflicts with `--snapshot` and a standalone program

**Tape**
- `--tape <path>` -- load a `.tap`/`.tzx`/`.cdt`/`.pzx`/`.csw`/`.voc`/`.wav` tape
  file, playing back real pulses. ZX Spectrum starts when ROM reads EAR;
  CPC loads start from its cassette motor protocol. Conflicts with
  `--snapshot`
- `--tape-speed <N>` -- turbo speed multiplier (default `1`, real time,
  or `tape_speed` in `--game-config`) -- plays the exact same real pulse
  protocol, just faster
- CPC puts `--tape-leader <ms>` (default `2000`, `0` = play the image exactly
  as stored) of silence in front of an attached tape: the firmware waits ~2.4 s
  after switching the motor on before it samples, so images that start straight
  on their first pilot tone (Arkanoid, Killapede, Dan Dare, Joe Blade 2, ...)
  otherwise lose that first block. CPCEC does the same with a 2000 ms hold.
- CPC's cassette motor policy (`--tape-mode`) and ZX Spectrum's fast-load
  trap (`--fast-tape-load`) are machine-declared options now -- see
  **Machine-specific options** above; run `retroclawd cpc6128 --help` or
  `retroclawd zx-spectrum --help` for their exact descriptions/defaults.

**Disk (`zx-spectrum-2a3`, `cpc664`, `cpc6128`, or `cpc6128plus`)**
- `--disk-b <path>` -- a disk for the second drive, drive B (the CPC664/6128's or +3's external drive; a machine without one refuses it).
  Same formats as `--disk`; AMSDOS finds it by the drive letter (`B:FILE.BIN`, `|B`). `--disk-b-save <path>` writes drive B's modified
  image when the session ends; `--disk-writeback` rewrites each given disk's own file. Put a disk in or out later with the
  `load_disk_b`/`eject_disk_b`/`save_disk_b` hotkeys or `--at N:load_disk_b=FILE`.
- `--disk <path>` -- load a `.dsk`/`.edsk` floppy image into the emulated
  +3/CPC drive. Standard and Extended DSK are auto-detected by content.
  Independent of `--snapshot`/`--tape`.

**Cartridge (`c64`; `.cpr` for the CPC Plus family)**
- `--cartridge <file.crt>` -- plug a cartridge into the C64's expansion port and
  reset, so it starts like a real one switched on. Normal 8K/16K and Ultimax
  images, Simons' BASIC, Ocean, Fun Play, Super Games, Epyx Fastload, System 3,
  Dinamic, Zaxxon, Magic Desk and EasyFlash (flash programming works; `--cartridge-save` keeps it), Action Replay, KCS Power,
  Final Cartridge III, Retro Replay and Expert (freeze button: `freeze` hotkey, ctrl+pause) are
  supported, as is crystalct's Unified Diagnostic Cartridge (our own `.crt` type 0xC640: the standard diagnostic as the 8K chip at `$8000`, the dead test as the one at `$E000`, switched from Ultimax to standard mode by a write to `$DE00`); `--georam 512|1024|2048|4096` fits a GeoRAM; `--serial swiftlink|turbo232` (+ `--serial-net connect:host:port` or `listen:port`) fits a TCP-backed serial cartridge, `--rrnet loopback` an RR-Net ethernet card with no real network, `--printer --printer-out out.txt` a serial-bus text printer; `--true-drive --rom-1541 dos1541.rom` runs a real 1541 (its own 6502 and your DOS ROM) on the IEC bus instead of the instant LOAD traps; any other `.crt` hardware type is a clear error naming the types that
  work. `--program game.p00` loads PC64 `.p00` programs like `.prg`.
- BASIC `SAVE"name",8` to an attached `.d64` works (`@0:name` replaces); persist it with
  `--disk-save <path>` or `--disk-writeback`. `.d71`/`.d81` are writable too (shared CBM DOS allocator). C64 `--disk` also takes `.g64` (decoded to sectors; not written back), `.x64`, a single-file `.zip`, and an `.m3u` playlist (first disk attached; bind the `next_disk`/`prev_disk` hotkeys to swap). `.nib` is refused with a clear error.
- `.tap` cassettes load instantly through the KERNAL (`--run` types `LOAD` and `RUN`); BASIC `SAVE"x",1` records to
  `--tape-save out.tap`. `--fast-tape-load false` (config) plays every pulse in real time instead.
- ZX81: `--tape game.p` (also `.81`, `.p81`) plays the file as the cassette signal for `LOAD ""` (real speed, about 307 bit/s: a
  small program takes seconds, a 16K one minutes); BASIC `SAVE "NAME"` is recorded and written by `--tape-save out.p`
  (or `.81`, `.p81` with the name). The ZX81 `.p` can also be saved straight from memory with `--dump-snapshot out.p`.
  `.tzx`, `.pzx`, `.csw`, `.voc` and `.wav` tapes play too (real time; rescaled to the ZX81's 3.25 MHz clock).
  `--fast-tape-load` makes `LOAD` store the file at once after the ROM has matched its name from the real signal (a 16K
  program is otherwise minutes of tape). A machine with a 16K or larger pack (`--ram-kb 16`) takes about 2.4 s to reach its prompt.
- `mouse = "1351-port1"` (or `1351-port2`, `paddle-port1`, `paddle-port2`) in `[machines.c64]` turns the host mouse into a 1351
  or a paddle pair; keys `Paddle1AInc` etc. drive paddles from the keyboard or a pad.
- `--reu off|128|256|512` fits a Commodore REU (1700/1764/1750) on the expansion
  port (not together with a cartridge); programs reach it through `$DF00-$DF0A`
  and `$FF00`.
- `--region ntsc` selects the NTSC machine (1.022727 MHz, 65x263 raster, 60 Hz);
  pair it with the `-03` KERNAL (`kernal-901227-03.bin`), which detects the
  standard itself -- the older `-01` KERNAL does not.

**Display**
- `--no-border` (alias `-nb`) -- render just the 256x192 active picture,
  without the border real hardware shows around it. Border is on by
  default (white on ZX81; the ZX Spectrum ULA's actual current border
  color otherwise); applies to both the GUI and `--headless --dump-screen`
- `--fullscreen` -- open the GUI window already fullscreen (borderless,
  current monitor's resolution); F11 toggles fullscreen live either way
  (GUI only)

- `--shader <name-or-path>` -- GUI post-processing shader. Bundled names:
  `crt`, `scanlines`, `sharp-bilinear`, `crt-geom`, `zfast-crt`, and
  `crt-potato`; any other value is loaded as a custom WGSL file. `--shader`
  is GUI-only. `crt-royale` and `mame_hlsl` are deferred because their
  upstream libretro presets require multi-pass render targets.
- `--shader-intensity <0.0-1.0>` -- overall curvature/mask strength for
  `crt`, `crt-geom`, `zfast-crt`, and `crt-potato` (default `0.6`). Ignored by
  `scanlines`, `sharp-bilinear`, and passthrough.
- `--scanline-intensity <0.0-1.0>` -- scanline strength for `crt`,
  `scanlines`, `crt-geom`, `zfast-crt`, and `crt-potato` (default `0.3`).
  Ignored by `sharp-bilinear` and passthrough. Both parameters also work as
  `shader_intensity` and `scanline_intensity` in `retroclawd.toml`.
  `fullscreen`, `vsync` (`false` = `--no-vsync`) and `max_fps` can be set there
  too, under `[default]` or a machine's own section.
- New shader ports keep upstream calibration values internally; no additional
  TOML/CLI parameters exist yet. `crt-geom` uses fixed progressive geometry
  and classic aperture mask, `zfast-crt` uses fixed zfast scanline/mask values,
  and `crt-potato` uses a procedural mask in place of its upstream PNG mask.

**Performance / input latency**
- `--run-ahead <N>` -- speculatively steps a cloned copy of the machine N
  extra frames ahead each tick and displays *that* clone's frame instead
  of the real machine's own, reducing perceived input latency (the real
  machine is never mutated by this). `0` (default) disables it entirely
  at zero extra cost (GUI only)

**Debugger plugins (GUI-only)**
- `--plugin debug-ui[:port=<N>]` -- browser debugger HTTP interface
  (`:port=9090` style inline option)
- `--plugin debug-log[:stdout][:file=<path>][:level=<trace|debug|info|warn|error>][:engine_level=<level>]`
  -- text diagnostics sink. `stdout` is a shorthand token (`output=stdout`);
  `file=...` overrides output to file when both are supplied. The first two
  bare tokens are positional shorthand for `output` then `level`, so
  `--plugin debug-log:.\output.log:debug` writes `output.log` at `debug`
  (a Windows drive prefix such as `C:\logs\out.log` stays one token). The
  third-party engine stack (bevy/wgpu/naga/winit/audio) is held to
  `engine_level` (default `warn`) so `level=debug` shows the emulator's own
  diagnostics instead of tens of thousands of shader-compiler lines; set
  `engine_level=debug` to see them. Like every debugger it attaches to
  the GUI's emulation thread -- with `--headless` it is ignored (with a
  warning).
- `--plugin debug-zrcp[:port=<N>]` (legacy alias `debug-tcp`) -- line-oriented TCP debugger interface
- You can load several at once, e.g.:
  `--plugin debug-log:stdout:file=.\\output.log:level=debug --plugin debug-ui:port=9090`

**Diagnostics**
- `--plugin debug-log:...` controls runtime diagnostics (keyboard events,
  HALT transitions, audio/tape diagnostics, and Bevy INFO logs) without
  enabling console output in normal runs
- `--show-overlay` (alias `-so`) -- show a semi-transparent GUI debug
  overlay. Optional colon-delimited panel list, e.g. `--show-overlay
  cpu:mem:tape`:
  - `cpu` -- frame-rate/register/HALT (6502 view includes A, P with
    `NV-BDIZC` flags, PC, S, X, and Y)
  - `mem`/`memory` -- hex dump of a 16-byte window at PC, plus the
    machine's paging/bank summary (when it has one)
  - `disk` -- floppy controller phase + motor state (ZX Spectrum +2A/+3,
    CPC664/6128)
  - `tape` -- cassette motor state, playback progress (`pos/total`,
    percent), T-states until the next EAR-line edge, a numeric list of
    recent pulse widths in T-states (compare against the standard ROM
    protocol's known-good values: pilot ~2168T, sync ~667/735T, bit0
    ~855T, bit1 ~1710T -- this is the real signal for "is the tape
    decoding actual bits or garbage"), and a time-uniform ASCII EAR-bit
    waveform bar (ZX Spectrum, CPC)
  - `version` -- the retroclawd version line and the line of the plugin running this machine (version, build number, commit, date);
    the `version` hotkey (`shift+f9`) flips this panel at any time
  - `all` -- every panel

  A bare `--show-overlay` (no value) shows `cpu` only. A panel with
  nothing to report on the current machine (e.g. `disk`/`tape` on
  ZX81/C64) renders as `n/a` rather than being omitted or erroring.

Example: load and run a standalone test program instead of the ROM's own
boot code, then dump the first 16 bytes of RAM afterward:

```
retroclawd zx81 --rom machines/sinclair/roms/zx81/zx81.rom --program program.bin --headless --dump-memory 0x4000:16
```

Example: boot a ROM for a while, then save what's on screen:

```
retroclawd zx81 --rom machines/sinclair/roms/zx81/zx81.rom --headless --max-cycles 2000000 --dump-screen screen.png
retroclawd zx-spectrum --rom machines/sinclair/roms/zx-spectrum/spec48.rom --headless --max-cycles 2000000 --dump-screen screen.png
```

Example: boot with both debugger UI + debug-log plugins:

```
retroclawd zx-spectrum --rom machines/sinclair/roms/zx-spectrum/spec48.rom --plugin debug-ui:port=9090 --plugin debug-log:stdout
# then open http://127.0.0.1:9090/
```

Example: CPC with its own machine-declared options, RAM expansion, and an
expansion ROM slot:

```
retroclawd cpc6128 --rom machines/amstrad/roms/cpc/cpc6128.rom --ram-banks 3 --rom-slot 7=amsdos.rom --tape-mode caprice
```


**Commodore VIC-20 (`vic20`, experimental)**

The Commodore Family builds `vic20_machine_plugin` alongside its existing
libraries; no machine code is linked into the Host. It provides an NMOS 6502,
VIC-I video/sound, two VIAs, keyboard/joystick, PAL/NTSC, expansion RAM, raw/PRG
programs, debugger integration and save states. Required firmware flags are
`--rom-basic`, `--rom-kernal` and `--rom-char`. `--ram-expansion` selects
`none|3k|8k|16k|24k|32k|35k`.

`--execution-mode fast|accurate` clocks Chips after instructions or on each
CPU bus event, respectively. Both modes support PAL and NTSC. Sub-clock VIC-I
effects, interlace, paddles, light pen and cassette/disk/cartridge attachment
remain unimplemented. **Real PAL/NTSC boots, BASIC typing, tone pitch, state
continuation and frame budgets pass in both modes.** Twenty scenarios pass,
with boot/input compared against hash-pinned VICE 3.9 captures; five
media/reference/diagnostic gaps remain. Audio gain differs from VICE;
see `machines/commodore/docs/vic20-gaps.md`. Do not apply the broader media
support claims for other machines to this experimental Plugin.

The Family also supplies a shared C16/Plus4 TAP-v2 parser/writer, consumed by
the `rctape` feature branch for inspection, structural validation and exact
half-wave normalization. This is a media prerequisite, not a C16/Plus4 Machine
or cassette playback implementation. See the Family README and tools manual
for supported headers, size limits and explicit-output behavior.

**Commodore 128 (`c128`)**
- `--mode c128|c64` (boot into C128 or C64 mode), `--display 40|80` (VIC-IIe or VDC screen), `--vic-accuracy
  line|cycle`, `--run` (auto-type `LOAD`/`RUN` for the attached media), `--drive-type 1541|1570|1571|1581`,
  `--true-drive` (a real drive CPU on the IEC bus), and the ROM options `--rom-kernal`, `--rom-basic`,
  `--rom-char`, `--rom-c64-kernal`, `--rom-c64-basic`. `retroclawd c128 --help` has the full list.

**Commodore execution modes (`c64`, `c128`)**

`--execution-mode fast|accurate` selects the existing instruction-batched or
per-bus-cycle VIC-II/VIC-IIe engine. Fast is the default. Accurate is PAL-only
and improves raster effects and BA stalls; it does not change Host pacing.
The C128 Z80 and 2 MHz CPU paths remain instruction-batched. Media traps and
true-drive emulation remain independent Options.

Set `execution_mode = "accurate"` under `[machines.c64]` or `[machines.c128]`
in application configuration, or under `[options]` in a game configuration.
Normal CLI > game > application > Plugin precedence applies per Option key.
The older `vic_accuracy = "line"|"cycle"` key remains supported; contradictory
explicit old/new selections are errors, even across configuration layers.
Save states retain their existing engine check and reject cross-mode loading.
See the Commodore Family README for fidelity limits and real-time media settings.

### Current debugger and plugin status

Debugger UI now supports reverse stepping, watches, conditional breakpoints, step-over, typed
register editing, `.POK` trainer import, machine-provided Bitmap Explorer palettes/defaults, and
ZRCP transport. Disassembly and the single-instruction assembler follow the CPU that runs (Z80 or
6502, the C128 switches with its running CPU), with symbol names (see **Debugger symbols**). Machines run through `plugin-api`; machines load from shared
libraries. Remaining roadmap focus: remove superseded app-side diagnostic paths, then resume machine
parity work. See `TODO.md` for order.

## Development

```
cargo test --workspace     # unit tests
cargo clippy -- -D warnings
cargo fmt
```

This repository's tests use `crates/plugins/test-machine-plugin`, a do-nothing machine; the real
machines are tested in their family repositories. Set `RETROCLAWD_TEST_MACHINE_LIBRARY` to a built
machine library to also run the "a real library loads" check. The screenshot-hash smoke test that
used to live here (`render_plugin_screens.sh`) needs all machines at once and has not been rebuilt
for the split yet (see `TODO.md`).

CPU accuracy is checked in the `retroclawd-crates` repository (`shared/`): Z80 against the ZEXALL/ZEXDOC
suites (`cargo run --release -p z80 --example zexall -- zexall`, also `zexdoc`) and the 6502 against Klaus
Dormann's and Tom Seddon's suites (part of `cargo test -p mos6502`).

See `CLAUDE.md` for the workspace's crate layout and architecture notes.

## License

retroclawd is free software under the **GNU General Public License v3.0 or later** (see `LICENSE`). Third-party
material keeps its own terms and is credited in `CREDITS.md`: the CRT shaders are ports of GPL-licensed libretro
shaders (see `CREDITS.md`). No ROM image or piece of commercial software is part of this repository or its
history; you need your own dumps (or `retroclawd --firmware auto` with a RomM server).

## Contributing / issues

Not accepted right now -- this is a personal work-in-progress project.
Pull requests and issues will be closed unreviewed.
