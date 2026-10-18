# retroclawd

A multi-machine retro-computer emulator. A Host application loads emulated Machines from Plugin libraries and provides the window, sound, input, media handling and debugging around them.

## Language

### Machines and loading

**Machine**:
One emulated model the user selects, such as `cpc6128`, declared by a machine profile.
_Avoid_: System, computer, emulator, platform, model

**Plugin**:
A shared library that provides one or more Machines to the Host.
_Avoid_: Driver, module, DLL, core

**Family**:
A repository holding a related set of Plugins and Machines (Sinclair, Amstrad, Commodore).
_Avoid_: Vendor, brand, platform

**Machine instance**:
A running Machine inside the Host.
_Avoid_: Session, emulation

**Region**:
The video standard and timing variant of a Machine (PAL, NTSC, Japan).
_Avoid_: Variant, standard, locale

**Host**:
The `retroclawd` application, which loads Plugins and owns the window, audio, input, media, configuration and Debuggers.
_Avoid_: App, frontend, emulator

**Debugger**:
A Host module (browser UI, ZRCP, log) that inspects and controls a Machine instance. Not a Plugin.
_Avoid_: Debug plugin

**Symbol table**:
An address-to-name map the Debugger uses to annotate disassembly and accept names when assembling.
_Avoid_: Labels

### Media and state

**Media**:
Anything attached from outside to a Machine: a tape, disk, cartridge or program.
_Avoid_: Image (bare), file, input

**Drive**:
A Media slot on a Machine that holds a disk (A, B). A separate disk unit such as the C64's 1541 is also a Drive, not a Machine.
_Avoid_: Disk unit, floppy, device

**Program**:
A raw binary loaded straight into Machine memory. A kind of Media.
_Avoid_: Binary, executable

**Snapshot**:
A third-party format capturing a whole Machine state (`.z80`, `.sna`, `.szx`, `.p`). Part of State, not Media.
_Avoid_: Dump, image

**Save state**:
This project's own `.state` format capturing a whole Machine state. Part of State, not Media.
_Avoid_: Savegame, snapshot

**State**:
The whole condition of a Machine, covering both Snapshots and Save states. It replaces the Machine instead of attaching to it.

**Poke**:
A Machine memory write applied from a `.pok` trainer file.
_Avoid_: Cheat

### Boot content

**ROM**:
The content a Machine boots from, supplied through a file Option.
_Avoid_: BIOS, kernal (as a general term), firmware

**Firmware**:
The Host's automatic sourcing of a ROM, matched by hash, when no file is named.
_Avoid_: ROM (as a synonym)

### Configuration

**Option**:
A value a Plugin declares for its Machine, such as `rom`, `ram_banks` or `turbosound`.
_Avoid_: Config value, parameter, flag

**Setting**:
A value the Host owns, such as `shader`, `fullscreen`, `vsync` or `max_fps`.
_Avoid_: Option, preference

**App config**:
The persistent `retroclawd.toml`, auto-loaded from the working directory. Holds Settings, Options per Machine, Key bindings and Hotkeys.
_Avoid_: Settings file, profile

**Game config**:
A per-game `.toml` passed explicitly, naming a Machine, its Media and any Option, Key binding or Hotkey overrides for that game.
_Avoid_: Game profile, launcher file

**Keymap file**:
A standalone file of Key bindings, loaded on top of the App config.
_Avoid_: Layout file

**Firmware list**:
A `firmware.toml` shipped by a Family, matching ROMs by hash for Firmware sourcing.
_Avoid_: ROM database

### Input

**Host key**:
A key on the user's keyboard, physical or typed, written as a QMK keycode.
_Avoid_: Real key, PC key

**Machine key**:
A key name a Plugin reports for its Machine, such as `SymShift` or `Joy1Fire1`.
_Avoid_: Emulated key, target key

**Key binding**:
A mapping from a Host key, chord or pad button to a Machine key, delivered to the Machine.
_Avoid_: Keymap entry, remap

**Hotkey**:
A Host key chord bound to a Host action such as `reset` or `pause`. The key is swallowed and never reaches the Machine.
_Avoid_: Shortcut, key binding

### Host UI

**Overlay**:
The live diagnostic panels (CPU, memory, disk, tape, version) drawn over the picture.
_Avoid_: HUD, status bar

**Toast**:
A transient one-line Host message, such as "Paused" or "State saved: slot 2".
_Avoid_: Notification

**Prompt**:
A Toast that waits for a key answer.
_Avoid_: Dialog (that is a native file dialog)

### Time

**Frame**:
One `1/frame_hz` slice of Machine time. The unit headless runs count and scripted actions are scheduled in.
_Avoid_: Tick, step

### Source structure

**Core**:
An emulated CPU (`z80`, `mos6502`).
_Avoid_: CPU crate, processor

**Chip**:
Any emulated integrated circuit other than the CPU, shared or per Family (`sid`, `nec765`).
_Avoid_: Device, peripheral

**Format**:
A parser for a Media or State file type (`dsk`, `zx-tape`, `.sna`).
_Avoid_: Loader, codec

### Accuracy testing

**Acid test**:
A program run on a Machine whose output proves hardware-accurate behaviour.
_Avoid_: Conformance test, test ROM

**Acid suite**:
A group of Acid tests.

**Core test**:
A suite that checks only a Core (ZEXALL/ZEXDOC, Klaus Dormann's, Tom Seddon's).
_Avoid_: Acid test
