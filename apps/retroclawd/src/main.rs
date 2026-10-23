mod debug_symbols;
#[cfg_attr(
    not(any(feature = "debug-ui", feature = "debug-zrcp")),
    allow(dead_code)
)]
mod debugger;
mod debuggers;
mod dialogs;
mod emu_thread;
mod firmware;
mod gui;
mod host_files;
mod inputs;
mod overrides;
mod playlist;
mod plugin_bridge;
mod romm;
mod scripted;
mod version;
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use app_config::AppConfig;
use clap::{Command, CommandFactory, FromArgMatches, Parser, ValueEnum};
use game_config::GameConfig;
use machine::Region;
use plugin_api::{
    BinaryBlob, DebugValue, MachineCreateRequest, MachineOptionInfo, MachineOptionKind,
    MachineProfile, MediaKind, NamedValue,
};

/// `--firmware`: where ROM options nothing else names come from.
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum FirmwareMode {
    /// Take them from the RomM server's firmware, matched by SHA-1.
    #[value(name = "auto")]
    Auto,
    /// Only the files named on the command line / in the config.
    #[value(name = "off")]
    Off,
}

#[derive(Copy, Clone, ValueEnum)]
enum RegionArg {
    #[value(name = "pal")]
    Pal,
    #[value(name = "ntsc")]
    Ntsc,
    #[value(name = "japan")]
    Japan,
}

impl From<RegionArg> for Region {
    fn from(region: RegionArg) -> Region {
        match region {
            RegionArg::Pal => Region::Pal,
            RegionArg::Ntsc => Region::Ntsc,
            RegionArg::Japan => Region::Japan,
        }
    }
}

fn parse_hex_u16(s: &str) -> Result<u16, String> {
    u16::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| e.to_string())
}

fn parse_dump_range(s: &str) -> Result<(u16, u16), String> {
    let (start, len) = s.split_once(':').ok_or("--dump-memory expects START:LEN")?;
    let start = parse_hex_u16(start)?;
    let len: u16 = len
        .parse()
        .map_err(|e: std::num::ParseIntError| e.to_string())?;
    Ok((start, len))
}

/// Resolves `--program`'s effective load address and bytes: a real `.prg`
/// file (case-insensitive extension) begins with its own real 2-byte
/// little-endian load address (the same real header a genuine Commodore
/// KERNAL LOAD reads), so that address is used instead of `org` and only
/// the bytes after it are loaded; every other extension is returned
/// unchanged, still loaded verbatim at `org`, exactly as `--program`
/// already worked before `.prg` support existed. Fully machine-agnostic --
/// this is the *only* change `.prg` support needed: the caller still
/// drives the exact same generic `Machine::load`/`set_pc` calls every
/// other `--program` file already goes through, no new plugin command or
/// per-machine handling required (same "own format, own authoritative
/// bytes, no override" precedent every other extension-sniffed media path
/// here already follows, e.g. `.d64`'s own track/sector geometry).
const P00_HEADER_LEN: usize = 26;

/// `p00`..`p99` -- PC64's program container (`s00`/`u00`/`r00` are SEQ/USR/REL
/// files, not programs).
fn is_p00_extension(extension: &str) -> bool {
    extension.len() == 3
        && extension.starts_with('p')
        && extension[1..].bytes().all(|b| b.is_ascii_digit())
}

fn resolve_program_load(path: &str, bytes: Vec<u8>, org: u16) -> Result<(u16, Vec<u8>), String> {
    let extension = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // PC64 container (`.p00`..`.p99`): a 26-byte header -- "C64File\0", a
    // 16-byte PETSCII name, a zero byte and a REL record size -- in front of
    // the plain `.prg` bytes (load address first).
    let bytes = if is_p00_extension(&extension) {
        if bytes.len() < P00_HEADER_LEN + 2 || &bytes[..8] != b"C64File\0" {
            return Err(format!(
                "{path} is not a PC64 .p00 file (needs the \"C64File\" header and a load address)"
            ));
        }
        bytes[P00_HEADER_LEN..].to_vec()
    } else {
        bytes
    };
    let is_prg = extension == "prg" || is_p00_extension(&extension);
    if !is_prg {
        return Ok((org, bytes));
    }
    if bytes.len() < 2 {
        return Err(format!(
            "{path} is too short to be a real .prg file (needs at least a 2-byte load address)"
        ));
    }
    let real_org = u16::from_le_bytes([bytes[0], bytes[1]]);
    Ok((real_org, bytes[2..].to_vec()))
}

/// Whether `--program name` goes to the machine plugin as `MediaKind::Program`: the machine declares it and the file is a
/// GEMDOS program by its extension.
fn program_goes_to_plugin(name: &str, media_kinds: &[MediaKind]) -> bool {
    let extension = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    media_kinds.contains(&MediaKind::Program) && matches!(extension.as_str(), "prg" | "tos" | "ttp")
}

fn normalize_cli_args() -> Vec<OsString> {
    env::args_os()
        .map(|arg| {
            let as_text = arg.to_string_lossy();
            match as_text.as_ref() {
                "-dm" => OsString::from("--dump-memory"),
                "-mc" => OsString::from("--max-cycles"),
                "-nb" => OsString::from("--no-border"),
                "-so" => OsString::from("--show-overlay"),
                "-lm" => OsString::from("--list-machine"),
                "-lp" => OsString::from("--list-plugins"),
                _ => arg,
            }
        })
        .collect()
}

fn render_help_with_single_dash_aliases(cmd: &mut Command) -> String {
    let mut help = Vec::new();
    let _ = cmd.write_long_help(&mut help);
    let mut text = String::from_utf8_lossy(&help).to_string();
    const ALIASES: [(&str, &str); 6] = [
        (
            "      --dump-memory <DUMP_MEMORY>",
            "  -dm, --dump-memory <DUMP_MEMORY>",
        ),
        (
            "      --max-cycles <MAX_CYCLES>",
            "  -mc, --max-cycles <MAX_CYCLES>",
        ),
        ("      --no-border", "  -nb, --no-border"),
        ("      --show-overlay", "  -so, --show-overlay"),
        ("      --list-machine", "  -lm, --list-machine"),
        ("      --list-plugins", "  -lp, --list-plugins"),
    ];
    for (needle, replacement) in ALIASES {
        text = text.replace(needle, replacement);
    }
    text
}

fn help_requested(args: &[OsString]) -> bool {
    args.iter().skip(1).any(|arg| {
        let text = arg.to_string_lossy();
        text == "-h" || text == "--help"
    })
}

/// Extracts the candidate machine id from raw argv: the first token after
/// the binary name that doesn't look like a flag. Per this CLI's own
/// `retroclawd <machine> [options]` convention, a directly-given machine
/// (as opposed to one supplied only via `--game-config`) must be that
/// exact first token -- the same convention `git`/`cargo`/`docker` all use
/// for their own leading subcommand-like positional. Needed before the
/// real parse: which machine was selected decides which plugin-declared
/// options get merged into the `Command` that then does the real parse.
fn prescan_machine_arg(args: &[OsString]) -> Option<String> {
    let first = args.get(1)?;
    let text = first.to_string_lossy();
    if text.starts_with('-') {
        // No positional machine: a `--game-config` may name it, and its
        // plugin-declared options (and flags) then belong in the command too.
        prescan_game_config_machine(args)
    } else {
        Some(text.into_owned())
    }
}

/// The `machine` named by a local `--game-config <file>` / `--game-config=<file>`
/// on the raw command line, if any. Unreadable or invalid files are left for
/// the real load below to report.
fn prescan_game_config_machine(args: &[OsString]) -> Option<String> {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        let text = arg.to_string_lossy();
        let path = if let Some(value) = text.strip_prefix("--game-config=") {
            value.to_string()
        } else if text == "--game-config" {
            iter.next()?.to_string_lossy().into_owned()
        } else {
            continue;
        };
        let body = fs::read_to_string(&path).ok()?;
        return game_config::parse(&body).ok()?.machine;
    }
    None
}

/// Extracts every `--plugin VALUE` / `--plugin=VALUE` occurrence from raw
/// argv without a full clap parse -- needed for the same reason as
/// `prescan_machine_arg`: `--plugin machine=<path>` may itself select a
/// dynamically-loaded machine plugin whose declared option schema the
/// real `Command` needs to already have merged in before parsing.
fn prescan_plugin_values(args: &[OsString]) -> Vec<String> {
    let mut values = Vec::new();
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        let text = arg.to_string_lossy();
        if let Some(value) = text.strip_prefix("--plugin=") {
            values.push(value.to_string());
        } else if text == "--plugin"
            && let Some(next) = iter.next()
        {
            values.push(next.to_string_lossy().into_owned());
        }
    }
    values
}

#[derive(Parser)]
#[command(
    name = "retroclawd",
    override_usage = "retroclawd [MACHINE] [OPTIONS]",
    next_help_heading = "Global options",
    disable_help_flag = true
)]
struct CommonArgs {
    /// Machine id to run (discovered from loaded plugins at startup). Must
    /// be the first argument when given directly, e.g. `retroclawd
    /// cpc6128 --rom cpc6128.rom`. Required for emulation, either here or
    /// as `machine` in --game-config.
    #[arg(index = 1, value_name = "MACHINE", display_order = 0)]
    machine: Option<String>,

    /// Print all discovered machine ids and exit.
    #[arg(long = "list-machine")]
    list_machine: bool,

    /// Print the version, build number and git commit of retroclawd and of every machine plugin found, and exit.
    #[arg(short = 'V', long = "version")]
    show_version: bool,

    /// Print discovered machine plugin libraries and the debuggers built into this
    /// executable, and exit. For debuggers, includes selector names and options;
    /// for machine plugins, includes each machine's own declared options.
    #[arg(long = "list-plugins")]
    list_plugins: bool,

    /// Print the emulator hotkeys (action, key, what it does) and exit. With
    /// a machine id, shows that machine's table after `retroclawd.toml`'s
    /// `[hotkeys]` and `[machines.<id>.hotkeys]` overrides; without one, the
    /// global table.
    #[arg(long = "list-hotkeys")]
    list_hotkeys: bool,

    /// Print the machine's key names (what `[keys]` values may be, with their
    /// aliases), the host key names bindings may use, and the effective
    /// bindings from `retroclawd.toml`, then exit. Needs a machine id.
    #[arg(long = "list-keys")]
    list_keys: bool,

    /// How host keys become machine keys: `positional` (the key at the same
    /// physical position, the default) or `symbolic` (the character the key
    /// types, so a German or French keyboard types the right letters and
    /// punctuation). Beats `[input] layout` in `retroclawd.toml`.
    #[arg(long = "layout", value_name = "positional|symbolic")]
    layout: Option<String>,

    /// A keymap file (`[keys]` / `[machines.<id>.keys]` and an optional
    /// `layout`, same syntax as `retroclawd.toml`) layered above the config's
    /// own `[keys]`. Repeatable; later files win.
    #[arg(long = "keymap", value_name = "FILE")]
    keymap: Vec<PathBuf>,

    /// Bind a hotkey for this run: `ACTION=CHORD` (`pause=ctrl+p`, `reset=none`, `modifier=alt_right`; `--list-hotkeys` shows the
    /// actions). The first flag for an action replaces what `retroclawd.toml` bound, further ones add chords. Repeatable.
    #[arg(long = "hotkey", value_name = "ACTION=CHORD")]
    hotkey: Vec<String>,

    /// A symbol file naming addresses in the debugger's disassembly (`; NAME` beside an operand).
    /// Repeatable, later files win. Accepts assembler listings (`NAME = $1234`, `NAME EQU 0x1234`),
    /// map files (`NAME 1234`) and VICE labels (`al C:1234 .NAME`); a path, URL or `archive.zip#entry`.
    /// Added to the machine's built-in names (C64/C128 KERNAL and chips, Spectrum ROM).
    #[arg(long = "symbols", value_name = "FILE")]
    symbols: Vec<String>,

    /// Extra plugin selectors. Repeatable. Accepted forms:
    /// `debug-ui[:OPT[:OPT...]]`, `debug-log[:OPT[:OPT...]]`,
    /// `debug-zrcp[:OPT[:OPT...]]` (or legacy `debug-tcp[:OPT[:OPT...]]`),
    /// `debugger[:OPT[:OPT...]]` (legacy alias for `debug-ui`),
    /// `machine=<path>`, or plain `<path>` (machine plugin path).
    ///
    /// Options use `:` as delimiter and are `KEY=VALUE` pairs.
    /// Examples:
    /// `--plugin debug-log:./output.log:level=trace`
    /// `--plugin debug-ui:port=9090`
    #[arg(long = "plugin")]
    plugin: Vec<String>,

    /// Optional standalone program to load instead of booting the ROM. A
    /// `.prg` path (case-insensitive) is auto-detected: its own real
    /// leading 2-byte little-endian load address is used instead of
    /// --org, and only the bytes after that header are loaded -- every
    /// other extension is loaded verbatim at --org, unchanged.
    #[arg(long, conflicts_with_all = ["snapshot", "tape", "load_state"])]
    program: Option<String>,

    /// Region variant (affects clock speed and frame rate). Defaults to
    /// `pal` if given neither here nor as `region` in --game-config.
    #[arg(long, value_enum)]
    region: Option<RegionArg>,

    /// `auto`: ROM options not named by a flag or config are filled from the
    /// RomM server's firmware, matched by SHA-1 (see `retroclawd romm match
    /// <machine>`); `off` (the default) uses only the files you name. A single
    /// ROM by id is `--rom romm:fw/<id>`. Also `[romm] firmware` in
    /// retroclawd.toml.
    #[arg(long, value_enum)]
    firmware: Option<FirmwareMode>,

    /// Per-game .toml config file supplying machine/region/media/
    /// tape-speed, plus a `[options]` table for plugin-declared options
    /// (ROM paths, tape mode, etc.). Any of those also given directly on
    /// the command line overrides the same setting from this file, rather
    /// than conflicting with it.
    #[arg(long)]
    game_config: Option<PathBuf>,

    /// Persistent settings .toml file (shader/window/fullscreen/vsync/max-fps
    /// choices, plus plugin-declared options flat in `[machines.<id>]`),
    /// unlike --game-config's per-game scope. Auto-loaded as
    /// `retroclawd.toml` from the current directory if present -- this flag
    /// only overrides *which* file to load, so it's optional even when a
    /// settings file exists. See `app_config::AppConfig`'s own doc comment
    /// for its `[default]`/`[machines.<name>]` section shape.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Program load address, hex (e.g. 0x4000). Ignored for a `.prg`
    /// --program file, which supplies its own real load address instead.
    #[arg(long, default_value = "0x4000", value_parser = parse_hex_u16)]
    org: u16,

    /// Hex-dump memory after halt: START:LEN (e.g. 0x4000:16)
    #[arg(long, value_parser = parse_dump_range, requires = "headless")]
    dump_memory: Option<(u16, u16)>,

    /// Stop after this many instructions even if never halted (safety valve
    /// for booting a real ROM with no video/keyboard, which spins forever)
    #[arg(long, requires = "headless")]
    max_cycles: Option<u64>,
    /// Run this many emulated frames the way the window does -- a frame
    /// interrupt every frame, the full cycle budget each, no stop at HALT --
    /// then dump. A ROM that HALTs while it waits for the next frame (the
    /// ZX81's display routine, an EI/HALT game loop) only gets past that
    /// this way; `--max-cycles` ends the run at the first HALT.
    #[arg(long, requires = "headless", conflicts_with = "max_cycles")]
    frames: Option<u32>,
    /// With `--frames`: type this text on the emulated keyboard, the way the
    /// paste hotkey does (the same key names, shifts and pacing), starting
    /// `--type-after` frames in. `\n` is Enter. Lets a batch run drive BASIC
    /// (`LOAD"*",8`, `RUN`) or check the keyboard.
    #[arg(long = "type", value_name = "TEXT", requires = "frames")]
    type_text: Option<String>,
    /// Frames to run before `--type` starts typing, long enough for the
    /// machine to reach its prompt.
    #[arg(
        long,
        value_name = "FRAMES",
        default_value_t = 150,
        requires = "type_text"
    )]
    type_after: u32,
    /// Milliseconds between two `--type` key events (a press, then a release,
    /// each one event); default: the paste delay of the configuration (60). A
    /// machine whose ROM scans the keyboard once a frame and debounces it,
    /// like the ZX81, loses keys typed that fast and wants about 160.
    #[arg(long, value_name = "MS", requires = "type_text")]
    type_delay: Option<u64>,
    /// With `--frames`: run a hotkey / file-dialog action at the start of a frame, `FRAME:ACTION[=FILE]` (repeatable): `reset`,
    /// `freeze`, `screenshot[=FILE]`, `quick_save`, `quick_load`, `state_slot_next|prev`, `load_tape|load_disk|load_snapshot|
    /// load_state=FILE`, `eject_tape|eject_disk`, `save_snapshot|save_state|save_tape|save_disk=FILE`. The same code as the
    /// window's hotkeys and dialogs, with the file named here instead of asked for; an action that fails makes the run exit 1.
    #[arg(long = "at", value_name = "FRAME:ACTION", requires = "frames", value_parser = scripted::parse)]
    at: Vec<scripted::Scripted>,
    /// With `--frames`: write everything the machine played as a mono 16-bit `.wav` (44.1 kHz) to this path, so a run can
    /// be checked for a tone's frequency or for silence.
    #[arg(long, value_name = "FILE", requires = "frames")]
    dump_audio: Option<String>,
    /// With `--frames`: each time the program counter reaches `ADDR` (e.g. a text-output routine such as the CPC's
    /// `0xBB5A`), append the low byte of register `REG` (`A`, ...) to `FILE`: a transcript of what a test program printed,
    /// readable even when the text scrolls off the screen. `--capture-at 0xBB5A:A=out.txt`.
    #[arg(long, value_name = "ADDR:REG=FILE", requires = "frames", value_parser = parse_capture_at)]
    capture_at: Option<CaptureAt>,
    /// Write the current video frame as a PNG to this path after the run
    /// stops (halt or --max-cycles cutoff)
    #[arg(long, requires = "headless")]
    dump_screen: Option<String>,

    /// Save the machine's state to this path after the run stops (halt or
    /// --max-cycles cutoff), in the snapshot format its extension names
    /// (.z80, .sna, or .szx) -- the save-side counterpart to --snapshot.
    #[arg(long, requires = "headless")]
    dump_snapshot: Option<String>,

    /// Save the machine's full live state (registers, memory, peripheral
    /// timing state) to this path after the run stops (halt or
    /// --max-cycles cutoff), in this project's own internal `.state`
    /// format -- the save-side counterpart to --load-state. Unlike
    /// --dump-snapshot, this never interoperates with other emulators; it
    /// exists purely to resume later via --load-state.
    #[arg(long, requires = "headless")]
    dump_state: Option<String>,

    /// Run to completion in the terminal (today's batch behavior) instead
    /// of opening a live window. Required by --max-cycles/--dump-memory/
    /// --dump-screen, which only make sense for a finite batch run.
    #[arg(long)]
    headless: bool,

    /// Load a snapshot file, replacing this machine's registers and memory
    /// outright instead of booting the ROM. Format is picked by file
    /// extension and must match the selected machine: `.z80`/`.sna`/`.szx`
    /// for zx-spectrum, `.p`/`.p81`/`.81` for zx81. Mutually exclusive with
    /// a standalone program (a snapshot fully replaces machine state; a
    /// program is poked into a running boot).
    #[arg(long, conflicts_with = "load_state")]
    snapshot: Option<String>,

    /// Load a previously-saved --dump-state file, restoring this machine's
    /// full live state outright instead of booting the ROM. Requires the
    /// same ROM (and --tape/--disk, if the saved state had one attached)
    /// to already be given -- a save state, like --snapshot, never embeds
    /// ROM/tape/disk contents themselves. Mutually exclusive with
    /// --snapshot and a standalone program, same reasoning as --snapshot's
    /// own conflicts.
    #[arg(long)]
    load_state: Option<String>,

    /// Load a tape file, playing it back in real time. Format is picked by
    /// file extension: `.tap`, `.tzx`, `.cdt`, `.pzx`, `.csw`, `.voc`, `.wav`, or
    /// `.t64`. ZX Spectrum models support all six non-C64 extensions; CPC
    /// models support `.cdt`/`.tzx`/`.voc` cassette images; C64 supports its
    /// own, differently-shaped `.tap` (a real bit-level pulse capture, an
    /// unrelated format despite the shared extension) plus `.t64` -- a
    /// directory container invented by an emulator, never a real bitstream,
    /// so it's poked directly into memory like a `.prg` instead of played
    /// back over real time. Mutually exclusive with --snapshot (a snapshot
    /// replaces machine state instantly; a tape plays back over real time
    /// -- different enough semantics to not overload one flag).
    #[arg(long, conflicts_with = "snapshot")]
    tape: Option<String>,

    /// Tape turbo speed multiplier (defaults to 1, real time, if given
    /// neither here nor as `tape_speed` in --game-config). Plays back the
    /// exact same real pulse protocol, just faster -- not a different
    /// loading mechanism. Only meaningful when a tape is loaded, via
    /// --tape or --game-config's `media`.
    #[arg(long)]
    tape_speed: Option<u32>,

    /// Load a `.dsk`/`.edsk` floppy image into a Spectrum +2A/+3, CPC664,
    /// CPC6128, or CPC6128Plus drive (standard and Extended DSK are
    /// detected by content, not extension), or a `.d64`/`.d71`/`.d81` disk
    /// image for `--machine c64` (1541/1571/1581 respectively) --
    /// serviced entirely by `c64::C64`'s own IEC-level fast loader (a real
    /// KERNAL LOAD request answered directly from the image's own logical
    /// contents, no simulated 1541/1571/1581/IEC-bus bit-banging at all --
    /// see `TODO.md`'s "drives and baseline media" section). Independent
    /// of snapshot/tape state.
    #[arg(long)]
    disk: Option<String>,

    /// A disk for the second drive, drive B (the external drive of a CPC664/6128 or Spectrum +3; the machine refuses it when it has no second
    /// drive). Same formats as `--disk`. Guest writes to it are handled like `--disk`'s: discarded with a warning unless
    /// `--disk-b-save` or `--disk-writeback` (which applies to both drives, each to its own file) is given.
    #[arg(long, value_name = "PATH")]
    disk_b: Option<String>,

    /// Where to write drive B's disk image, with the guest's writes, when the session ends.
    #[arg(
        long,
        value_name = "PATH",
        requires = "disk_b",
        conflicts_with = "disk_writeback"
    )]
    disk_b_save: Option<String>,

    /// Plug a cartridge into the expansion port: a `.crt` image for
    /// `--machine c64` (normal 8K/16K and Ultimax, Simons' BASIC, Ocean,
    /// Fun Play, Super Games, Epyx Fastload, System 3, Dinamic, Zaxxon,
    /// Magic Desk and EasyFlash -- anything else is a clear error), or a
    /// `.cpr` for the CPC Plus family. The machine resets so the cartridge
    /// starts, as when switching a real one on.
    #[arg(long)]
    cartridge: Option<String>,

    /// Write the cartridge, with everything the emulated machine programmed
    /// into its flash, to this `.crt` path when the session ends (EasyFlash
    /// only). The original `--cartridge` file is left untouched; without this
    /// flag programmed flash is discarded, with a warning.
    #[arg(long, value_name = "PATH", requires = "cartridge")]
    cartridge_save: Option<String>,

    /// Write what the C64 prints to its serial-bus printer (device 4/5, needs
    /// `--printer`) to this text file when the session ends, PETSCII converted
    /// to ASCII.
    #[arg(long, value_name = "PATH")]
    printer_out: Option<String>,

    /// Write the disk image, with everything the emulated machine wrote to
    /// it, to this path when the session ends. The original `--disk` file
    /// is left untouched. Only written if the guest actually modified the
    /// disk.
    #[arg(
        long,
        value_name = "PATH",
        requires = "disk",
        conflicts_with = "disk_writeback"
    )]
    disk_save: Option<String>,

    /// Write the hard disk behind ACSI target N (the `--acsiN` image), with everything the guest wrote to it, to FILE when the
    /// session ends: `--acsi-save 0=hd-copy.img`, repeatable. Only written if the guest modified the disk.
    #[arg(long, value_name = "N=FILE")]
    acsi_save: Vec<String>,

    /// Write the modified disk image back over the `--disk` file itself
    /// when the session ends (atomically: temp file + rename). Without
    /// this or `--disk-save`, guest disk writes are discarded, with a
    /// warning if any happened.
    #[arg(long)]
    disk_writeback: bool,

    /// Record everything the program writes to the cassette output (a real
    /// `SAVE`) and write it to this file when the session ends. The format
    /// follows the extension: `.cdt`/`.tzx` (pulse blocks) or `.voc` (8-bit
    /// PCM at 44.1 kHz) for the CPC, `.tap` for the C64, `.p`/`.81`/`.p81`
    /// (the file a ZX81 `SAVE` made, read back from the pulses) for the ZX81.
    /// A run that never wrote to the cassette reports so and writes nothing.
    #[arg(long, value_name = "PATH")]
    tape_save: Option<String>,

    /// Print progress on stderr while a URL is downloaded or a zip is
    /// unpacked (any input: ROMs, disks, tapes, snapshots, configs). These
    /// happen before the window opens, so a toast could not show them; later
    /// downloads (an `.m3u` disk swap) also show a toast.
    #[arg(long)]
    progress: bool,

    /// Show a semi-transparent GUI debug overlay. Optional colon-delimited
    /// panel list: `cpu` (registers/HALT -- 6502 view includes A, P flags,
    /// PC, S, X, Y), `mem`/`memory` (hex dump + paging/bank summary),
    /// `disk` (FDC phase + motor), `tape` (motor + EAR-bit waveform bar),
    /// or `all` for every panel. A bare `--show-overlay` (no value) shows
    /// `cpu` only. Unrecognized/unsupported panels on a given machine
    /// (e.g. `disk`/`tape` on ZX81/C64) render as `n/a`, not an error. A
    /// CLI flag rather than a runtime toggle because every window key
    /// drives the emulated keyboard.
    #[arg(
        long = "show-overlay",
        alias = "debug-info",
        conflicts_with = "headless",
        num_args = 0..=1,
        default_missing_value = "cpu",
        value_name = "PANELS"
    )]
    show_overlay: Option<String>,

    /// Render just the active picture (256x192), without the border real
    /// hardware shows around it. Border is on by default, in the machine's
    /// current border color (fixed white on ZX81; ULA-controlled via
    /// `OUT (0xFE)` on ZX Spectrum) -- applies to both GUI and
    /// `--headless --dump-screen`, since both go through `render_frame`.
    #[arg(long)]
    no_border: bool,

    /// Open the window already fullscreen (borderless, at the current
    /// monitor's own resolution). F11 toggles fullscreen live either way.
    #[arg(long, conflicts_with = "headless")]
    fullscreen: bool,

    /// Disable display VSync for the GUI window. If `--max-fps` is not
    /// given, this automatically applies a 60 FPS limiter preset.
    #[arg(long, conflicts_with = "headless")]
    no_vsync: bool,

    /// Cap GUI render/update rate to this FPS value (e.g. 60 or 120).
    /// When omitted, `--no-vsync` defaults this to 60 FPS.
    #[arg(long, conflicts_with = "headless", value_parser = clap::value_parser!(u32).range(1..))]
    max_fps: Option<u32>,

    /// CRT-style post-processing shader applied to the emulated picture:
    /// `crt` (curvature+scanlines+phosphor mask), `crt-geom` (geometry-
    /// accurate curvature, beam scanlines, and aperture mask), `zfast-crt`
    /// (fast luminance-aware scanlines and mask), `crt-potato` (lightweight
    /// phosphor mask), `scanlines` (scanlines only, no curvature),
    /// `sharp-bilinear` (smooth upscale, no CRT artifacts), or a filesystem
    /// path to a custom `.wgsl` file. Defaults to plain passthrough (today's
    /// exact nearest-neighbor look) if given neither here nor as `shader` in
    /// --config. `crt-royale` and `mame_hlsl` are not included: their
    /// upstream presets require multi-pass render targets.
    #[arg(long, conflicts_with = "headless")]
    shader: Option<String>,

    /// Overall strength of active --shader's effect (curvature/mask/bloom
    /// where supported). 0.0-1.0, defaults to 0.6 if given neither here nor
    /// as `shader_intensity` in --config.
    #[arg(long, conflicts_with = "headless")]
    shader_intensity: Option<f32>,

    /// Scanline darkness, shared by CRT shaders that support scanlines.
    /// 0.0-1.0, defaults to 0.3 if given neither here nor as
    /// `scanline_intensity` in --config.
    #[arg(long, conflicts_with = "headless")]
    scanline_intensity: Option<f32>,

    /// Speculatively steps a cloned copy of the machine this many extra
    /// frames ahead each tick, using whatever input is currently held, and
    /// displays *that* clone's frame instead of the real machine's own --
    /// the standard "run-ahead" input-latency trick (see RetroArch/
    /// Spectral). The real machine is never mutated by this -- only the
    /// disposable clone is stepped ahead; its audio is always discarded
    /// (never reaches the sound card), matching RetroArch's own run-ahead
    /// audio handling. `0` (default) disables this entirely, at zero extra
    /// cost -- no clone is ever made. GUI-only: there's no "displayed
    /// frame" to speed up in `--headless`, so the two flags conflict
    /// rather than one silently no-op'ing the other.
    #[arg(long, default_value_t = 0, conflicts_with = "headless")]
    run_ahead: u32,
}

#[derive(Debug, Clone)]
struct PluginRuntime {
    debugger_plugins: Vec<debuggers::LaunchSelection>,
}

fn parse_plugin_value(raw: &str) -> plugin_api::DebugValue {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        return plugin_api::DebugValue::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return plugin_api::DebugValue::Bool(false);
    }
    if let Ok(value) = trimmed.parse::<i64>() {
        return if value < 0 {
            plugin_api::DebugValue::I64(value)
        } else {
            plugin_api::DebugValue::U64(value as u64)
        };
    }
    plugin_api::DebugValue::Text(trimmed.to_string())
}

fn debugger_selection_index(
    selections: &mut Vec<debuggers::LaunchSelection>,
    selector: &str,
) -> usize {
    if let Some(index) = selections.iter().position(|item| item.selector == selector) {
        index
    } else {
        selections.push(debuggers::LaunchSelection {
            selector: selector.to_string(),
            options: Vec::new(),
        });
        selections.len() - 1
    }
}

fn push_debugger_option(
    selection: &mut debuggers::LaunchSelection,
    selector: &str,
    bare_position: &mut usize,
    token: &str,
) -> Result<(), String> {
    let token = token.trim();
    if token.is_empty() {
        return Ok(());
    }

    let (key, value) = if let Some((key, value)) = token.split_once('=') {
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("empty option key in --plugin token {token:?}"));
        }
        (key.to_string(), parse_plugin_value(value.trim()))
    } else {
        // Bare (KEY=VALUE-less) shorthand tokens are positional, counted
        // only among themselves (an explicit KEY=VALUE token elsewhere in
        // the same selector doesn't consume a slot): `debug-log` has two
        // (output, then level -- e.g. `debug-log:./out.log:debug`),
        // everything else has exactly one. Deriving the implied key from
        // `selector` alone (ignoring position) was a real bug: every bare
        // token mapped to the SAME key ("output" for every debug-log
        // token), so `debug-log:./out.log:debug`'s second token silently
        // overwrote the first, and logs ended up in a file literally named
        // `debug` instead of `./out.log`.
        let implied_key = match (selector, *bare_position) {
            ("debug-log", 0) => "output",
            ("debug-log", 1) => "level",
            ("debug-ui" | "debug-zrcp", 0) => "port",
            _ => {
                return Err(format!(
                    "option {token:?} for --plugin {selector} must be KEY=VALUE (only the first {} positional token{} support shorthand)",
                    if selector == "debug-log" { 2 } else { 1 },
                    if selector == "debug-log" { "s" } else { "" }
                ));
            }
        };
        *bare_position += 1;
        (implied_key.to_string(), parse_plugin_value(token))
    };

    selection
        .options
        .push(plugin_api::NamedValue { key, value });
    Ok(())
}

/// Splits a `--plugin` selector's option tail on `:`, except that a Windows
/// drive prefix stays attached to its path: `C:\logs\out.log:debug` is two
/// tokens (`C:\logs\out.log`, `debug`), not three (a single ASCII letter
/// followed by a token starting with `\` or `/` is a drive letter).
fn split_plugin_tokens(tail: &str) -> Vec<String> {
    let mut parts = tail.split(':').peekable();
    let mut tokens = Vec::new();
    while let Some(part) = parts.next() {
        let is_drive_letter = part.len() == 1
            && part.as_bytes()[0].is_ascii_alphabetic()
            && parts
                .peek()
                .is_some_and(|next| next.starts_with('\\') || next.starts_with('/'));
        if is_drive_letter {
            let rest = parts.next().unwrap_or_default();
            tokens.push(format!("{part}:{rest}"));
        } else {
            tokens.push(part.to_string());
        }
    }
    tokens
}

fn apply_plugin_runtime(plugin_values: &[String]) -> Result<PluginRuntime, String> {
    let mut runtime = PluginRuntime {
        debugger_plugins: Vec::new(),
    };
    let mut machine_paths = Vec::<PathBuf>::new();

    for raw_selector in plugin_values {
        let selector = raw_selector.trim();
        if selector.is_empty() {
            continue;
        }

        if let Some((head, tail)) = selector.split_once(':')
            && let Some(debug_selector) = debuggers::canonical_selector(head.trim())
        {
            let index = debugger_selection_index(&mut runtime.debugger_plugins, debug_selector);
            let mut bare_position = 0usize;
            for token in split_plugin_tokens(tail) {
                push_debugger_option(
                    &mut runtime.debugger_plugins[index],
                    debug_selector,
                    &mut bare_position,
                    &token,
                )?;
            }
            continue;
        }

        if let Some((kind, value)) = selector.split_once('=') {
            let kind = kind.trim();
            let value = value.trim();
            if value.is_empty() {
                return Err(format!("--plugin has empty value in {selector:?}"));
            }
            if kind.eq_ignore_ascii_case("machine") {
                machine_paths.push(PathBuf::from(value));
                continue;
            }
            if debuggers::canonical_selector(kind).is_some() {
                return Err(format!(
                    "--plugin {selector:?}: the debuggers are built into retroclawd and are not loaded from files; use {kind}:KEY=VALUE for options"
                ));
            }
            return Err(format!(
                "unsupported --plugin selector {:?}; expected machine=..., debug-ui[:...], debug-log[:...], debug-zrcp[:...], debug-tcp[:...] (legacy), or debugger[:...]",
                kind
            ));
        }

        if let Some(debug_selector) = debuggers::canonical_selector(selector) {
            debugger_selection_index(&mut runtime.debugger_plugins, debug_selector);
            continue;
        }

        machine_paths.push(PathBuf::from(selector));
    }

    if !machine_paths.is_empty() {
        let joined = env::join_paths(machine_paths.iter())
            .map_err(|e| format!("invalid --plugin machine path list: {e}"))?;
        // SAFETY: called during process startup before worker threads are spawned.
        unsafe { env::set_var("RETROCLAWD_MACHINE_PLUGINS", joined) };
    }

    Ok(runtime)
}

fn region_name(region: Region) -> &'static str {
    match region {
        Region::Pal => "pal",
        Region::Ntsc => "ntsc",
        Region::Japan => "japan",
    }
}

fn build_machine_from_rom_blobs(
    machine_id: &str,
    region: Region,
    roms: Vec<BinaryBlob>,
    options: Vec<plugin_api::NamedValue>,
) -> Result<plugin_bridge::HostedMachine, String> {
    plugin_bridge::create_hosted_machine(MachineCreateRequest {
        machine_id: machine_id.to_string(),
        region: Some(region_name(region).to_string()),
        roms,
        options,
    })
}

pub(crate) fn write_png(path: &str, width: u32, height: u32, rgb: &[u8]) -> Result<(), String> {
    let file = fs::File::create(path).map_err(|e| e.to_string())?;
    let writer = BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgb).map_err(|e| e.to_string())
}

/// Everything needed to build and load a machine, after merging
/// `CommonArgs` with an optional `--game-config` file: any flag given
/// directly on the command line overrides the same setting from the
/// toml, rather than conflicting with it. Kept pure (no file IO, no clap)
/// so the merge logic is unit-testable on its own. ROM paths are no
/// longer resolved here -- they're plugin-declared `File`/`FileSet`
/// options now, resolved by `resolve_machine_options` alongside every
/// other machine-specific option.
struct Resolved {
    machine_id: String,
    region: RegionArg,
    /// Path to a snapshot/tape/disk file named by --game-config's `media`.
    /// `None` if an explicit --snapshot/--tape/--disk flag was given
    /// instead -- those three flags fully replace `media` rather than
    /// merging with it, since they collapse to the same one setting.
    media: Option<String>,
    tape_speed: u32,
}

fn resolve(args: &CommonArgs, config: &GameConfig) -> Result<Resolved, String> {
    let machine_id = match args.machine.clone() {
        Some(machine_id) => machine_id,
        None => match &config.machine {
            Some(name) => name.clone(),
            None => {
                return Err(
                    "no machine given; pass it as the first argument (e.g. `retroclawd cpc6128 \
                     --rom cpc6128.rom`) or set `machine` in --game-config"
                        .into(),
                );
            }
        },
    };

    let region = match args.region {
        Some(region) => region,
        None => match &config.region {
            Some(name) => RegionArg::from_str(name, true).map_err(|_| {
                format!("--game-config's `region` value {name:?} is not a valid --region")
            })?,
            None => RegionArg::Pal,
        },
    };

    let media = if args.snapshot.is_some() || args.tape.is_some() || args.disk.is_some() {
        None
    } else {
        config.media.clone()
    };

    let tape_speed = args.tape_speed.or(config.tape_speed).unwrap_or(1);

    Ok(Resolved {
        machine_id,
        region,
        media,
        tape_speed,
    })
}

/// GX4000 real hardware fact: its cartridge doubles as both "the ROM" and
/// "the media". A `.cpr` file supplied only via `media` (not a declared
/// `rom` option value) is still the intended cartridge image -- promotes
/// it into `rom_paths` under the `rom` key the same way an explicit
/// `--rom something.cpr` would have, so `retroclawd gx4000 --game-config
/// game.toml` boots correctly when `game.toml` only sets `media =
/// "game.cpr"`.
fn promote_gx4000_cpr_media(
    machine_id: &str,
    resolved: &mut Resolved,
    rom_paths: &mut Vec<(String, PathBuf)>,
) {
    if !machine_id.eq_ignore_ascii_case("gx4000") {
        return;
    }
    if rom_paths.iter().any(|(key, _)| key == "rom") {
        return;
    }
    let is_cpr = resolved.media.as_deref().is_some_and(|path| {
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cpr"))
    });
    if is_cpr {
        let path = resolved.media.take().expect("checked Some above");
        rom_paths.push(("rom".to_string(), PathBuf::from(path)));
    }
}

/// What the media really is -- the file inside a `.zip`, the file a URL names.
fn media_extension(path: &str) -> String {
    inputs::extension(path)
}

/// `resolved_options` gating on whether the "run" key is present at all
/// (rather than a hardcoded machine-id list) is what makes this
/// automatically CPC/C64-only: only profiles that actually declare a
/// `run` option (`cpc_machine_plugin::cpc_options`,
/// `c64_machine_plugin::machine_profile`) make `--run` exist as a
/// parseable flag at all -- clap itself rejects it with its own
/// "unexpected argument" error for any other machine before this
/// function ever runs.
fn resolve_auto_run_profile(
    args: &CommonArgs,
    resolved: &Resolved,
    resolved_options: &HashMap<String, DebugValue>,
) -> Result<Option<gui::AutoRunProfile>, String> {
    if !matches!(resolved_options.get("run"), Some(DebugValue::Bool(true))) {
        return Ok(None);
    }

    if [
        args.program.is_some(),
        args.tape.is_some(),
        args.disk.is_some(),
    ]
    .iter()
    .filter(|given| **given)
    .count()
        > 1
    {
        return Err(
            "--run with more than one of --program/--tape/--disk is ambiguous; pass only one"
                .to_string(),
        );
    }

    let media_path = args
        .program
        .as_deref()
        .or(args.tape.as_deref())
        .or(args.disk.as_deref())
        .or(resolved.media.as_deref())
        .ok_or_else(|| {
            "--run needs attached media: a .prg/.t64 program, a CPC .cdt/.tzx tape, a CPC \
             .dsk/.edsk disk, or a C64 .d64/.d71/.d81 disk or .tap tape"
                .to_string()
        })?;

    let extension = media_extension(media_path);
    if resolved.machine_id == "c128" {
        // The C128 boots into C128 mode (BASIC 7) or, with `mode = c64`, the
        // C64's; the startup command differs and a disk needs a true drive.
        let c64_mode = matches!(resolved_options.get("mode"),
            Some(DebugValue::Text(mode)) if mode == "c64");
        return match extension.as_str() {
            "d64" | "d71" | "d81" | "g64" | "x64" if c64_mode => {
                Ok(Some(gui::AutoRunProfile::C128C64Disk))
            }
            "d64" | "d71" | "d81" | "g64" | "x64" => Ok(Some(gui::AutoRunProfile::C128Disk)),
            "prg" if c64_mode => Ok(Some(gui::AutoRunProfile::C64Run)),
            "prg" => Ok(Some(gui::AutoRunProfile::C128Run)),
            ext if is_p00_extension(ext) && c64_mode => Ok(Some(gui::AutoRunProfile::C64Run)),
            ext if is_p00_extension(ext) => Ok(Some(gui::AutoRunProfile::C128Run)),
            _ => Err(format!(
                "--run does not know the startup command for {media_path} (.{extension}) on the \
                 c128; expected a .prg/.p00 program or a .d64/.d71/.d81 disk (with --true-drive)"
            )),
        };
    }
    match extension.as_str() {
        "cdt" | "tzx" => Ok(Some(gui::AutoRunProfile::CpcTape)),
        "dsk" | "edsk" => Ok(Some(gui::AutoRunProfile::CpcDisk)),
        // Same auto-type sequence (`LOAD"*",8,1` then `RUN`) regardless of
        // which real CBM DOS disk geometry is attached -- the fast-loader
        // trap's own `AttachedDisk` abstraction already made the format
        // irrelevant past "a disk is attached", see `c64::bus`'s own
        // doc comment.
        "d64" | "d71" | "d81" | "g64" | "x64" => Ok(Some(gui::AutoRunProfile::C64Load)),
        "prg" | "t64" => Ok(Some(gui::AutoRunProfile::C64Run)),
        ext if is_p00_extension(ext) => Ok(Some(gui::AutoRunProfile::C64Run)),
        // Instant when the fast tape trap is on (the default): a bare LOAD takes
        // the first program, RUN follows.
        "tap" => Ok(Some(gui::AutoRunProfile::C64Tape)),
        _ => Err(format!(
            "--run does not know the startup command for {media_path} (.{}); expected \
             .cdt/.tzx/.dsk/.edsk (CPC) or .prg/.p00/.t64/.d64/.d71/.d81 (C64)",
            extension
        )),
    }
}

/// Builds a `clap::Arg` for one plugin-declared machine option. No
/// `.required()`/`.default_value()` at the clap level for any kind --
/// "was this explicitly given on the CLI" must stay distinguishable from
/// "fall through to --game-config/retroclawd.toml/the plugin's own
/// default", which a clap-applied default would erase. `required`-ness
/// and defaulting are both enforced afterward, in `resolve_one_machine_option`.
fn machine_option_arg(info: &MachineOptionInfo) -> clap::Arg {
    let mut arg = clap::Arg::new(info.flag.clone())
        .long(info.flag.clone())
        .help(info.description.clone());
    if let Some(short) = info.short {
        arg = arg.short(short);
    }
    match &info.kind {
        MachineOptionKind::Bool => {
            arg = arg.action(clap::ArgAction::SetTrue);
        }
        MachineOptionKind::Int { min, max } => {
            let lo = min.unwrap_or(i64::MIN);
            let hi = max.unwrap_or(i64::MAX);
            arg = arg
                .value_parser(clap::value_parser!(i64).range(lo..=hi))
                .num_args(1);
        }
        MachineOptionKind::Text | MachineOptionKind::File => {
            arg = arg.value_parser(clap::value_parser!(String)).num_args(1);
        }
        MachineOptionKind::Enum { choices } => {
            arg = arg
                .value_parser(clap::builder::PossibleValuesParser::new(choices.clone()))
                .num_args(1);
        }
        MachineOptionKind::FileSet { .. } => {
            arg = arg
                .value_parser(clap::value_parser!(String))
                .action(clap::ArgAction::Append)
                .value_name("N=PATH")
                .num_args(1);
        }
    }
    arg
}

/// Merges every declared option of the selected machine into `cmd` as
/// real clap `Arg`s, on top of `CommonArgs`'s own fixed schema, under
/// their own `"<machine_id> options"` heading (`Command::next_help_heading`
/// -- clap's own mechanism for grouped help sections) -- what makes
/// `retroclawd cpc6128 --help` show CPC's own options in their own
/// section alongside the common ones, and what makes `retroclawd cpc6128
/// --ram-banks 3` parse at all despite `--ram-banks` never being declared
/// in this binary's own source.
fn build_command(machine_id: Option<&str>, machine_options: &[MachineOptionInfo]) -> Command {
    let mut cmd = CommonArgs::command()
        .next_help_heading("Global options")
        .arg(
            clap::Arg::new("help")
                .short('h')
                .long("help")
                .action(clap::ArgAction::Help)
                .help("Print help (see a summary with '-h')"),
        );
    if let Some(machine_id) = machine_id
        && !machine_options.is_empty()
    {
        cmd = cmd.next_help_heading(format!("{machine_id} options"));
    }
    for info in machine_options {
        cmd = cmd.arg(machine_option_arg(info));
    }
    cmd
}

fn toml_value_to_text(value: &toml::Value, key: &str) -> Result<String, String> {
    match value {
        toml::Value::String(s) => Ok(s.clone()),
        other => Err(format!(
            "option \"{key}\" must be a string in toml, got {other:?}"
        )),
    }
}

fn toml_value_to_bool(value: &toml::Value, key: &str) -> Result<bool, String> {
    match value {
        toml::Value::Boolean(b) => Ok(*b),
        other => Err(format!(
            "option \"{key}\" must be a bool in toml, got {other:?}"
        )),
    }
}

fn toml_value_to_int(value: &toml::Value, key: &str) -> Result<i64, String> {
    match value {
        toml::Value::Integer(i) => Ok(*i),
        other => Err(format!(
            "option \"{key}\" must be an integer in toml, got {other:?}"
        )),
    }
}

/// A `FileSet` option's toml shape is a nested table, slot number (as a
/// string key, toml has no integer-keyed tables) to path:
/// `[machines.cpc6128.rom_slot]` / `4 = "extra.rom"` -- mirrors
/// the CLI's own repeatable `--rom-slot 4=extra.rom` shorthand.
fn toml_value_to_file_set(value: &toml::Value, key: &str) -> Result<Vec<(u32, PathBuf)>, String> {
    let toml::Value::Table(table) = value else {
        return Err(format!(
            "option \"{key}\" must be a table of slot=path in toml, got {value:?}"
        ));
    };
    table
        .iter()
        .map(|(slot, path)| {
            let slot: u32 = slot
                .parse()
                .map_err(|_| format!("option \"{key}\" has invalid slot number {slot:?}"))?;
            let path = toml_value_to_text(path, key)?;
            Ok((slot, PathBuf::from(path)))
        })
        .collect()
}

fn debug_value_from_i64(value: i64) -> DebugValue {
    if value < 0 {
        DebugValue::I64(value)
    } else {
        DebugValue::U64(value as u64)
    }
}

/// One fully-resolved machine-plugin-declared option value: either a
/// scalar bound for `MachineCreateRequest::options`, or a path (or
/// numbered set of paths) bound for `MachineCreateRequest::roms` once
/// read from disk.
#[derive(Debug)]
enum ResolvedOption {
    Scalar(DebugValue),
    RomPath(PathBuf),
    RomPathSet(Vec<(u32, PathBuf)>),
}

/// Resolves one declared option's value through the documented precedence
/// chain: CLI flag > --game-config's `[options]` > retroclawd.toml's
/// `[machines.<id>]` > the plugin's own declared default.
fn resolve_one_machine_option(
    info: &MachineOptionInfo,
    matches: &clap::ArgMatches,
    game_config_options: &HashMap<String, toml::Value>,
    app_config_options: &HashMap<String, toml::Value>,
) -> Result<Option<ResolvedOption>, String> {
    match &info.kind {
        MachineOptionKind::Bool => {
            if matches
                .try_get_one::<bool>(&info.flag)
                .ok()
                .flatten()
                .copied()
                .unwrap_or(false)
            {
                return Ok(Some(ResolvedOption::Scalar(DebugValue::Bool(true))));
            }
            if let Some(value) = game_config_options.get(&info.key) {
                return Ok(Some(ResolvedOption::Scalar(DebugValue::Bool(
                    toml_value_to_bool(value, &info.key)?,
                ))));
            }
            if let Some(value) = app_config_options.get(&info.key) {
                return Ok(Some(ResolvedOption::Scalar(DebugValue::Bool(
                    toml_value_to_bool(value, &info.key)?,
                ))));
            }
            Ok(info.default.clone().map(ResolvedOption::Scalar))
        }
        MachineOptionKind::Int { .. } => {
            if let Some(value) = matches.try_get_one::<i64>(&info.flag).ok().flatten() {
                return Ok(Some(ResolvedOption::Scalar(debug_value_from_i64(*value))));
            }
            if let Some(value) = game_config_options.get(&info.key) {
                return Ok(Some(ResolvedOption::Scalar(debug_value_from_i64(
                    toml_value_to_int(value, &info.key)?,
                ))));
            }
            if let Some(value) = app_config_options.get(&info.key) {
                return Ok(Some(ResolvedOption::Scalar(debug_value_from_i64(
                    toml_value_to_int(value, &info.key)?,
                ))));
            }
            Ok(info.default.clone().map(ResolvedOption::Scalar))
        }
        MachineOptionKind::Text | MachineOptionKind::Enum { .. } => {
            if let Some(value) = matches.try_get_one::<String>(&info.flag).ok().flatten() {
                return Ok(Some(ResolvedOption::Scalar(DebugValue::Text(
                    value.clone(),
                ))));
            }
            if let Some(value) = game_config_options.get(&info.key) {
                return Ok(Some(ResolvedOption::Scalar(DebugValue::Text(
                    toml_value_to_text(value, &info.key)?,
                ))));
            }
            if let Some(value) = app_config_options.get(&info.key) {
                return Ok(Some(ResolvedOption::Scalar(DebugValue::Text(
                    toml_value_to_text(value, &info.key)?,
                ))));
            }
            Ok(info.default.clone().map(ResolvedOption::Scalar))
        }
        MachineOptionKind::File => {
            if let Some(value) = matches.try_get_one::<String>(&info.flag).ok().flatten() {
                return Ok(Some(ResolvedOption::RomPath(PathBuf::from(value))));
            }
            if let Some(value) = game_config_options.get(&info.key) {
                return Ok(Some(ResolvedOption::RomPath(PathBuf::from(
                    toml_value_to_text(value, &info.key)?,
                ))));
            }
            if let Some(value) = app_config_options.get(&info.key) {
                return Ok(Some(ResolvedOption::RomPath(PathBuf::from(
                    toml_value_to_text(value, &info.key)?,
                ))));
            }
            Ok(None)
        }
        MachineOptionKind::FileSet { .. } => {
            if let Some(values) = matches.try_get_many::<String>(&info.flag).ok().flatten() {
                let mut entries = Vec::new();
                for entry in values {
                    let (slot, path) = entry
                        .split_once('=')
                        .ok_or_else(|| format!("--{} expects N=path, got {entry:?}", info.flag))?;
                    let slot: u32 = slot
                        .parse()
                        .map_err(|_| format!("--{} slot number invalid: {slot:?}", info.flag))?;
                    entries.push((slot, PathBuf::from(path)));
                }
                return Ok(Some(ResolvedOption::RomPathSet(entries)));
            }
            if let Some(value) = game_config_options.get(&info.key) {
                return Ok(Some(ResolvedOption::RomPathSet(toml_value_to_file_set(
                    value, &info.key,
                )?)));
            }
            if let Some(value) = app_config_options.get(&info.key) {
                return Ok(Some(ResolvedOption::RomPathSet(toml_value_to_file_set(
                    value, &info.key,
                )?)));
            }
            Ok(None)
        }
    }
}

/// Host settings that may sit beside a machine's own options in a
/// `[machines.<id>]` section (typed fields of `app_config::MachineSettings`).
const HOST_SETTING_KEYS: [&str; 6] = [
    "shader",
    "shader_intensity",
    "scanline_intensity",
    "fullscreen",
    "vsync",
    "max_fps",
];

fn validate_declared_option_keys(
    declared: &[MachineOptionInfo],
    config_options: &HashMap<String, toml::Value>,
    source: &str,
) -> Result<(), String> {
    let declared_keys: HashSet<&str> = declared.iter().map(|info| info.key.as_str()).collect();
    let mut unknown: Vec<&String> = config_options
        .keys()
        .filter(|key| !declared_keys.contains(key.as_str()))
        .collect();
    unknown.sort();
    if let Some(key) = unknown.first() {
        if key.as_str() == "options" {
            return Err(format!(
                "{source}: the nested `options` table is gone -- put the machine's options \
                 directly in the section (e.g. `rom = \"...\"`)"
            ));
        }
        let mut available: Vec<&str> = declared_keys.iter().copied().collect();
        available.extend(HOST_SETTING_KEYS);
        available.sort_unstable();
        return Err(format!(
            "unknown option \"{key}\" in {source} (available: {})",
            available.join(", ")
        ));
    }
    Ok(())
}

/// Every resolved plugin-declared option for the selected machine, split
/// by destination: `values` bind `MachineCreateRequest::options`,
/// `rom_paths` are `(blob_key, fs_path)` pairs still needing to be read
/// from disk into `MachineCreateRequest::roms`.
#[derive(Debug)]
struct ResolvedMachineOptions {
    values: HashMap<String, DebugValue>,
    rom_paths: Vec<(String, PathBuf)>,
    /// Required file options nothing named; only filled when the caller asked
    /// to defer that error (`--firmware auto` may still supply them).
    missing_files: Vec<MachineOptionInfo>,
}

fn resolve_machine_options(
    machine_id: &str,
    declared: &[MachineOptionInfo],
    matches: &clap::ArgMatches,
    game_config_options: &HashMap<String, toml::Value>,
    app_config_options: &HashMap<String, toml::Value>,
    defer_missing_files: bool,
) -> Result<ResolvedMachineOptions, String> {
    validate_declared_option_keys(
        declared,
        game_config_options,
        &format!("--game-config's [options] for machine \"{machine_id}\""),
    )?;
    validate_declared_option_keys(
        declared,
        app_config_options,
        &format!("retroclawd.toml's [machines.{machine_id}]"),
    )?;

    let mut values = HashMap::new();
    let mut rom_paths = Vec::new();
    let mut missing_files = Vec::new();
    for info in declared {
        let resolved =
            resolve_one_machine_option(info, matches, game_config_options, app_config_options)?;
        match resolved {
            Some(ResolvedOption::Scalar(value)) => {
                values.insert(info.key.clone(), value);
            }
            Some(ResolvedOption::RomPath(path)) => {
                rom_paths.push((info.key.clone(), path));
            }
            Some(ResolvedOption::RomPathSet(entries)) => {
                for (slot, path) in entries {
                    rom_paths.push((format!("{}{}", info.key, slot), path));
                }
            }
            None => {
                if info.required {
                    if defer_missing_files && info.kind == MachineOptionKind::File {
                        missing_files.push(info.clone());
                    } else {
                        return Err(missing_option_error(machine_id, info));
                    }
                }
            }
        }
    }

    Ok(ResolvedMachineOptions {
        values,
        rom_paths,
        missing_files,
    })
}

/// `--firmware`, else `[romm] firmware`, else off.
fn resolve_firmware_mode(
    flag: Option<FirmwareMode>,
    romm: &app_config::RommConfig,
) -> Result<FirmwareMode, String> {
    if let Some(mode) = flag {
        return Ok(mode);
    }
    match romm.firmware.as_deref().map(str::trim) {
        None | Some("off") => Ok(FirmwareMode::Off),
        Some("auto") => Ok(FirmwareMode::Auto),
        Some(other) => Err(format!(
            "retroclawd.toml [romm] firmware must be \"auto\" or \"off\", not \"{other}\""
        )),
    }
}

fn missing_option_error(machine_id: &str, info: &MachineOptionInfo) -> String {
    let auto = if info.kind == MachineOptionKind::File {
        ", or --firmware auto (RomM)"
    } else {
        ""
    };
    format!(
        "missing required option \"{}\" for machine \"{machine_id}\": set via \
         --{}, [machines.{machine_id}] in retroclawd.toml, or \
         --game-config's [options]{auto}",
        info.key, info.flag
    )
}

const DEFAULT_SHADER_INTENSITY: f32 = 0.6;
const DEFAULT_SCANLINE_INTENSITY: f32 = 0.3;
const DEFAULT_WINDOW_SCALE_PERCENT: u32 = 60;

/// Everything the GUI needs to render the picture, after merging `Args`
/// with an optional --config file -- same "CLI beats file" shape as
/// `Resolved`/`resolve` above, just for shader/window-size settings
/// instead of which game to boot. `shader` stays `None` for the
/// passthrough default rather than defaulting to a name here, since
/// `gui::shader` already treats "no shader selected" as its own case.
pub(crate) struct AppSettings {
    /// Already-resolved WGSL source text (see `gui::shader::resolve_source`)
    /// -- resolved once here in `main`, not inside the GUI, so a bad
    /// `--shader <path>` fails the same clean "print an error, exit
    /// non-zero" way a bad `--rom` path already does, before any window
    /// ever opens.
    pub(crate) shader_source: String,
    /// The selected shader's name or path, `None` for the default.
    pub(crate) shader_name: Option<String>,
    pub(crate) shader_intensity: f32,
    pub(crate) scanline_intensity: f32,
    pub(crate) window_scale_percent: u32,
    /// `[keys]` / `[machines.<id>.keys]` resolved against the machine's key names.
    pub(crate) key_bindings: gui::keys::KeyBindings,
    /// Positional or symbolic host keyboard layout, and the machine's
    /// character map the symbolic one uses.
    pub(crate) layout: gui::keys::Layout,
    pub(crate) symbols: gui::keys::SymbolMap,
    /// How the paste hotkey types letters and how fast.
    pub(crate) paste_case: gui::paste::PasteCase,
    pub(crate) paste_delay_ms: u64,
    /// `[pads]` / `[machines.<id>.pads]` resolved against the machine's key names.
    pub(crate) pad_bindings: gui::pads::PadBindings,
    /// How media loaded from a file dialog is treated (tape speed, disk exit policy).
    pub(crate) media: emu_thread::MediaSettings,
    /// Where file dialogs start and what the machine supports.
    pub(crate) dialogs: dialogs::DialogSettings,
    /// Open the window borderless-fullscreen (`--fullscreen` or `fullscreen
    /// = true`).
    pub(crate) fullscreen: bool,
    /// Resolved hotkey table for the selected machine.
    pub(crate) hotkeys: gui::hotkeys::Hotkeys,
    /// Display VSync off (`--no-vsync` or `vsync = false`).
    pub(crate) no_vsync: bool,
    /// Render/update rate cap (`--max-fps`, else `max_fps`, else a 60 FPS
    /// preset when VSync is off).
    pub(crate) max_fps: Option<u32>,
}

/// Loads `--config`'s file, or `./retroclawd.toml` when it exists, or an
/// empty config.
fn load_app_config(explicit: Option<&PathBuf>) -> Result<AppConfig, String> {
    let path = explicit.cloned().or_else(|| {
        let default_path = PathBuf::from("retroclawd.toml");
        default_path.exists().then_some(default_path)
    });
    let Some(path) = path else {
        return Ok(AppConfig::default());
    };
    let text =
        fs::read_to_string(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    app_config::parse(&text).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

/// Rejects an --config file that can never apply correctly.
fn validate_app_config(config: &AppConfig) -> Result<(), String> {
    if config.keymap.is_some() {
        return Err(
            "--config: [keymap] was replaced by [keys] and [machines.<id>.keys] -- move \
             `host_key = \"machine_key\"` pairs there (see README \"Key bindings\")"
                .to_string(),
        );
    }
    if !config.default.keys.is_empty() {
        return Err(
            "--config: [default.keys] is not a section -- key bindings for every machine go in \
             [keys], per-machine ones in [machines.<id>.keys]"
                .to_string(),
        );
    }
    if let Some(key) = config.default.options.keys().min() {
        return Err(format!(
            "--config: unknown key \"{key}\" in [default] -- machine options such as ROM \
             paths belong in the machine's own [machines.<id>] section"
        ));
    }
    if config.default.input != app_config::InputConfig::default() {
        return Err(
            "--config: [default.input] is not a section -- `layout` and `keymap` for every machine \
             go in [input], per-machine ones in [machines.<id>.input]"
                .to_string(),
        );
    }
    if !config.default.pads.slots.is_empty() || config.default.pads.deadzone.is_some() {
        return Err(
            "--config: [default.pads] is not a section -- gamepad bindings for every machine go \
             in [pads], per-machine ones in [machines.<id>.pads]"
                .to_string(),
        );
    }
    if !config.default.hotkeys.bindings.is_empty() || config.default.hotkeys.modifier.is_some() {
        return Err(
            "--config: [default.hotkeys] is not a section -- global hotkeys go in [hotkeys], \
             per-machine ones in [machines.<id>.hotkeys]"
                .to_string(),
        );
    }
    let sections = std::iter::once(("default".to_string(), &config.default)).chain(
        config
            .machines
            .iter()
            .map(|(name, settings)| (format!("machines.{name}"), settings)),
    );
    for (name, settings) in sections {
        if settings.max_fps == Some(0) {
            return Err(format!("--config: [{name}].max_fps must be at least 1"));
        }
    }
    for (name, settings) in &config.machines {
        if settings.window_scale_percent.is_some() {
            return Err(format!(
                "--config: [machines.{name}].window_scale_percent is not supported -- \
                 window size applies to every machine equally, set it under [default] instead"
            ));
        }
    }
    Ok(())
}

/// The directory `retroclawd.toml` was read from (relative keymap paths in it
/// are relative to this); the working directory when there is no config file.
fn config_dir(config: Option<&PathBuf>) -> PathBuf {
    config
        .and_then(|path| path.parent())
        .filter(|dir| !dir.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Reads the keymap files: those listed in `[input] keymap` (global, then the
/// machine's; paths relative to the config file) become the lowest layers,
/// those given with `--keymap` the highest.
fn load_keymaps(
    args: &CommonArgs,
    config: &AppConfig,
    machine: &str,
) -> Result<Vec<gui::keys::LoadedKeymap>, String> {
    let base = config_dir(args.config.as_ref());
    let listed = config
        .input
        .keymap
        .iter()
        .chain(
            config
                .machines
                .get(machine)
                .and_then(|m| m.input.keymap.as_ref()),
        )
        .flat_map(|files| files.as_slice().iter())
        .map(|name| (base.join(name), false));
    let given = args.keymap.iter().map(|path| (path.clone(), true));
    let mut loaded = Vec::new();
    for (path, upper) in listed.chain(given) {
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read keymap file {}: {e}", path.display()))?;
        let file = app_config::parse_keymap(&text)
            .map_err(|e| format!("failed to parse keymap file {}: {e}", path.display()))?;
        loaded.push(gui::keys::LoadedKeymap {
            label: path.display().to_string(),
            upper,
            file,
        });
    }
    Ok(loaded)
}

/// `--layout`, else the machine's `[machines.<id>.input] layout`, else
/// `[input] layout`, else the last keymap file that names one, else
/// positional.
fn effective_layout(
    args: &CommonArgs,
    config: &AppConfig,
    machine: &str,
    files: &[gui::keys::LoadedKeymap],
) -> Result<gui::keys::Layout, String> {
    let chosen = args
        .layout
        .as_deref()
        .map(|text| (text, "--layout".to_string()))
        .or_else(|| {
            config
                .machines
                .get(machine)
                .and_then(|m| m.input.layout.as_deref())
                .map(|text| (text, format!("[machines.{machine}.input] layout")))
        })
        .or_else(|| {
            config
                .input
                .layout
                .as_deref()
                .map(|text| (text, "[input] layout".to_string()))
        })
        .or_else(|| {
            files.iter().rev().find_map(|f| {
                f.file
                    .layout
                    .as_deref()
                    .map(|t| (t, format!("layout in {}", f.label)))
            })
        });
    match chosen {
        None => Ok(gui::keys::Layout::Positional),
        Some((text, source)) => {
            gui::keys::Layout::parse(text).map_err(|e| format!("{source}: {e}"))
        }
    }
}

/// `paste_case` / `paste_delay_ms` from `[machines.<id>.input]`, else `[input]`,
/// else the defaults.
fn resolve_paste_settings(
    config: &AppConfig,
    machine: &str,
) -> Result<(gui::paste::PasteCase, u64), String> {
    let machine_input = config.machines.get(machine).map(|m| &m.input);
    let case = machine_input
        .and_then(|i| i.paste_case.as_deref())
        .map(|t| (t, format!("[machines.{machine}.input] paste_case")))
        .or_else(|| {
            config
                .input
                .paste_case
                .as_deref()
                .map(|t| (t, "[input] paste_case".to_string()))
        });
    let case = match case {
        None => gui::paste::PasteCase::default(),
        Some((text, source)) => {
            gui::paste::PasteCase::parse(text).map_err(|e| format!("{source}: {e}"))?
        }
    };
    let delay = machine_input
        .and_then(|i| i.paste_delay_ms)
        .or(config.input.paste_delay_ms)
        .unwrap_or(gui::paste::DEFAULT_PASTE_DELAY_MS);
    if delay == 0 {
        return Err("paste_delay_ms must be at least 1".to_string());
    }
    Ok((case, delay))
}

fn resolve_app_settings(
    args: &CommonArgs,
    config: &AppConfig,
    machine: &str,
    key_names: &gui::keys::KeyNames,
    keymaps: &[gui::keys::LoadedKeymap],
) -> Result<AppSettings, String> {
    let machine_settings = config.machines.get(machine);

    let shader = args
        .shader
        .clone()
        .or_else(|| machine_settings.and_then(|m| m.shader.clone()))
        .or_else(|| config.default.shader.clone());
    let shader_source = gui::shader::resolve_source(shader.as_deref())?;

    let shader_intensity = args
        .shader_intensity
        .or_else(|| machine_settings.and_then(|m| m.shader_intensity))
        .or(config.default.shader_intensity)
        .unwrap_or(DEFAULT_SHADER_INTENSITY);

    let scanline_intensity = args
        .scanline_intensity
        .or_else(|| machine_settings.and_then(|m| m.scanline_intensity))
        .or(config.default.scanline_intensity)
        .unwrap_or(DEFAULT_SCANLINE_INTENSITY);

    // No per-machine override, no CLI flag either (only --config's
    // [default] can set this) -- see `validate_app_config`'s own doc
    // comment for why per-machine is rejected outright rather than just
    // unread.
    let window_scale_percent = config
        .default
        .window_scale_percent
        .unwrap_or(DEFAULT_WINDOW_SCALE_PERCENT);

    // Same "CLI beats file" order as the shader settings. A bool CLI switch
    // can only force the non-default value (`--fullscreen`, `--no-vsync`);
    // turning it back off has to come from the file or the default.
    let from_file = |pick: fn(&app_config::MachineSettings) -> Option<bool>| {
        machine_settings
            .and_then(pick)
            .or_else(|| pick(&config.default))
    };
    let fullscreen = args.fullscreen || from_file(|s| s.fullscreen).unwrap_or(false);
    let no_vsync = args.no_vsync || !from_file(|s| s.vsync).unwrap_or(true);
    let max_fps = args
        .max_fps
        .or_else(|| machine_settings.and_then(|m| m.max_fps))
        .or(config.default.max_fps)
        .or_else(|| no_vsync.then_some(60));

    let hotkeys = gui::hotkeys::Hotkeys::resolve(config, machine)?;
    let key_bindings =
        gui::keys::KeyBindings::resolve(config, machine, key_names, &hotkeys, keymaps)?;
    let layout = effective_layout(args, config, machine, keymaps)?;
    let symbols = gui::keys::SymbolMap::from_names(&key_names.machine);
    let (paste_case, paste_delay_ms) = resolve_paste_settings(config, machine)?;
    let pad_bindings = gui::pads::PadBindings::resolve(config, machine, key_names)?;

    Ok(AppSettings {
        hotkeys,
        shader_source,
        shader_name: shader,
        shader_intensity,
        scanline_intensity,
        window_scale_percent,
        key_bindings,
        layout,
        symbols,
        paste_case,
        paste_delay_ms,
        pad_bindings,
        media: emu_thread::MediaSettings::default(),
        dialogs: dialogs::DialogSettings::default(),
        fullscreen,
        no_vsync,
        max_fps,
    })
}

fn machine_option_value_hint(kind: &MachineOptionKind) -> String {
    match kind {
        MachineOptionKind::Bool => "bool".to_string(),
        MachineOptionKind::Int { min, max } => match (min, max) {
            (Some(lo), Some(hi)) => format!("int {lo}-{hi}"),
            (Some(lo), None) => format!("int >= {lo}"),
            (None, Some(hi)) => format!("int <= {hi}"),
            (None, None) => "int".to_string(),
        },
        MachineOptionKind::Text => "text".to_string(),
        MachineOptionKind::Enum { choices } => format!("one of: {}", choices.join(", ")),
        MachineOptionKind::File => "file path".to_string(),
        MachineOptionKind::FileSet {
            min_index,
            max_index,
        } => {
            format!("repeatable N=path, N={min_index}-{max_index}")
        }
    }
}

/// Renders every discovered machine's own declared options as its own
/// section, appended to `--help` output when no specific machine was
/// selected. Deliberately plain hand-formatted text, not a second clap
/// `Command` -- multiple machines commonly declare the same flag name
/// (every machine has its own `rom` option), which a single merged
/// `Command` can't represent (clap requires unique long-flag names), and
/// this text is display-only regardless (real parsing only ever merges
/// the ONE selected machine's options, via `build_command`).
fn render_all_machine_options_overview(profiles: &[MachineProfile]) -> String {
    let mut profiles: Vec<&MachineProfile> = profiles.iter().collect();
    profiles.sort_by(|a, b| a.machine_id.cmp(&b.machine_id));

    let mut out = String::new();
    out.push_str("\nMachine options (run `retroclawd <machine> --help` for just one):\n");
    for profile in profiles {
        if profile.options.is_empty() {
            continue;
        }
        out.push_str(&format!("\n{} options:\n", profile.machine_id));
        for opt in &profile.options {
            out.push_str(&format!(
                "      --{} <{}>\n          {}\n",
                opt.flag,
                machine_option_value_hint(&opt.kind),
                opt.description
            ));
        }
    }
    out
}

fn run_text_verb(args: &[OsString]) -> ExitCode {
    let words: Vec<String> = args
        .iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if words.iter().any(|w| w == "-h" || w == "--help") {
        println!("{}", romm::USAGE);
        return ExitCode::SUCCESS;
    }
    match load_app_config(None) {
        Ok(config) => romm::configure(&config.romm),
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }
    let _guard = inputs::CleanupGuard;
    match romm::run_verb(&words, &mut std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    // No arguments at all (e.g. double-clicked on Windows with no console
    // attached) is not a usage error -- just show how to use it and exit
    // cleanly instead of failing with no visible error message.
    let normalized_args = normalize_cli_args();
    if normalized_args.len() <= 1 {
        print!(
            "{}",
            render_help_with_single_dash_aliases(&mut CommonArgs::command())
        );
        return ExitCode::SUCCESS;
    }

    // `retroclawd romm ...` / `retroclawd cache ...`: plain text verbs, no machine.
    if matches!(
        normalized_args.get(1).and_then(|a| a.to_str()),
        Some("romm" | "cache")
    ) {
        return run_text_verb(&normalized_args);
    }

    // Both prescans work off raw argv, without a full clap parse: which
    // machine (if any) was selected decides which plugin-declared options
    // the real `Command` below needs merged in, and `--plugin
    // machine=<path>` may itself be what makes that machine discoverable
    // at all -- both must run before `plugin_bridge::startup_report`.
    let machine_hint = prescan_machine_arg(&normalized_args);
    let plugin_values = prescan_plugin_values(&normalized_args);
    let plugin_runtime = match apply_plugin_runtime(&plugin_values) {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let machine_report = plugin_bridge::startup_report();
    let hinted_options: &[MachineOptionInfo] = match (machine_hint.as_deref(), &machine_report) {
        (Some(id), Ok(report)) => report
            .profiles
            .iter()
            .find(|p| p.machine_id == id)
            .map(|profile| profile.options.as_slice())
            .unwrap_or(&[]),
        _ => &[],
    };
    // A named machine whose plugin cannot be found would otherwise surface as
    // clap rejecting that machine's options ("unexpected argument --rom").
    if let Some(id) = machine_hint.as_deref()
        && !help_requested(&normalized_args)
    {
        match &machine_report {
            Err(e) => {
                eprintln!("machine plugin discovery failed: {e}");
                return ExitCode::FAILURE;
            }
            Ok(report) if !report.profiles.iter().any(|p| p.machine_id == id) => {
                let mut ids: Vec<&str> = report
                    .profiles
                    .iter()
                    .map(|p| p.machine_id.as_str())
                    .collect();
                ids.sort();
                eprintln!(
                    "unknown machine \"{id}\"; discovered machines: {}",
                    ids.join(", ")
                );
                return ExitCode::FAILURE;
            }
            Ok(_) => {}
        }
    }
    let mut cmd = build_command(machine_hint.as_deref(), hinted_options);

    if help_requested(&normalized_args) {
        let mut text = version::summary(machine_report.as_ref().ok());
        text.push('\n');
        text.push_str(&render_help_with_single_dash_aliases(&mut cmd));
        // No specific machine was selected: append every discovered
        // machine's own options as its own section (clap itself can't
        // render this in one Command -- every machine that declares a
        // `rom` option would collide on that same flag name), so `--help`
        // alone still surfaces the full picture instead of only the
        // common flags.
        if machine_hint.is_none()
            && let Ok(report) = &machine_report
        {
            text.push_str(&render_all_machine_options_overview(&report.profiles));
        }
        print!("{text}");
        return ExitCode::SUCCESS;
    }

    // Consumes `cmd`; clap prints its own error and exits(2) on a parse
    // failure, matching `Parser::parse_from`'s prior behavior exactly.
    let matches = cmd.get_matches_from(normalized_args);
    let mut args = match CommonArgs::from_arg_matches(&matches) {
        Ok(args) => args,
        Err(e) => {
            e.exit();
        }
    };

    // Zips unpacked while loading live in one temporary folder; this removes
    // it when `main` returns (also on a panic).
    let _unzipped_files = inputs::CleanupGuard;
    if args.progress {
        inputs::enable_stderr_progress();
    }

    // `--disk list.m3u`: attach the first disk now, keep the rest for the
    // next_disk / prev_disk hotkeys. The list and its entries may be URLs.
    let mut disk_playlist: Vec<std::path::PathBuf> = Vec::new();
    if let Some(list) = args.disk.clone().filter(|d| playlist::is_playlist(d)) {
        let parsed = inputs::load(&list)
            .and_then(|loaded| {
                String::from_utf8(loaded.bytes.clone())
                    .map_err(|e| format!("{list} is not valid UTF-8 text: {e}"))
            })
            .and_then(|text| playlist::parse_m3u(&text, &list));
        match parsed {
            Ok(entries) => {
                args.disk = Some(entries[0].to_string_lossy().into_owned());
                disk_playlist = entries;
            }
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    }

    if args.show_version {
        print!("{}", version::summary(machine_report.as_ref().ok()));
        return ExitCode::SUCCESS;
    }

    if args.list_plugins {
        println!(
            "host compatibility: plugin ABI {}.{} (plugins need the same major and at least this minor), protocol {}, machine profile contract {}",
            plugin_api::ABI_MAJOR_V1,
            plugin_api::ABI_MINOR_V1,
            plugin_api::PROTOCOL_VERSION,
            plugin_api::MACHINE_PROFILE_CONTRACT_VERSION
        );
        println!("machine plugins:");
        match &machine_report {
            Ok(report) => {
                if report.plugins.is_empty() {
                    println!("- (none)");
                } else {
                    for plugin in &report.plugins {
                        println!(
                            "- {} {} (ABI {}.{}) [{}] machines: {}",
                            plugin.plugin_id,
                            build_info::Info::parse(&plugin.plugin_version),
                            plugin.abi.0,
                            plugin.abi.1,
                            plugin.source,
                            plugin.machine_ids.join(", ")
                        );
                    }
                }
                if report.profiles.iter().any(|p| !p.options.is_empty()) {
                    println!("machine options:");
                    for profile in &report.profiles {
                        if profile.options.is_empty() {
                            continue;
                        }
                        println!("- {}:", profile.machine_id);
                        for opt in &profile.options {
                            println!(
                                "    --{} ({}) {}",
                                opt.flag,
                                machine_option_value_hint(&opt.kind),
                                opt.description
                            );
                        }
                    }
                }
                for warning in &report.warnings {
                    eprintln!("machine plugin scan warning: {warning}");
                }
            }
            Err(e) => eprintln!("machine plugin discovery failed: {e}"),
        }

        println!("debuggers (built into this executable, selected with --plugin):");
        let debuggers = debuggers::built_in();
        if debuggers.is_empty() {
            println!("- (none: this build has no debug-ui, debug-zrcp or debug-log feature)");
        }
        for info in &debuggers {
            println!("- {} ({})", info.selector, info.display_name);
            println!("    {}", info.summary);
            if info.options.is_empty() {
                println!("    options: (none)");
            } else {
                println!("    options:");
                for opt in &info.options {
                    println!(
                        "      - {} ({}) {}",
                        opt.key, opt.value_hint, opt.description
                    );
                }
            }
        }
        return ExitCode::SUCCESS;
    }

    if args.list_hotkeys {
        let mut config = match load_app_config(args.config.as_ref()) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let machine = args.machine.as_deref().unwrap_or("");
        if let Err(e) = overrides::apply_hotkey_flags(&mut config, machine, &args.hotkey) {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
        return match gui::hotkeys::Hotkeys::resolve(&config, machine) {
            Ok(hotkeys) => {
                if machine.is_empty() {
                    println!("hotkeys (global):");
                } else {
                    println!("hotkeys for {machine}:");
                }
                print!("{}", hotkeys.describe());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }

    if args.list_machine {
        let report = match &machine_report {
            Ok(report) => report,
            Err(e) => {
                eprintln!("failed to discover machine plugins: {e}");
                return ExitCode::FAILURE;
            }
        };

        if report.profiles.is_empty() {
            println!("no machines discovered");
        } else {
            println!("available machines:");
            for profile in &report.profiles {
                println!("- {}", profile.machine_id);
            }
        }
        for warning in &report.warnings {
            eprintln!("machine plugin scan warning: {warning}");
        }
        return ExitCode::SUCCESS;
    }

    if args.game_config.is_some() && args.program.is_some() {
        // Clap's own `conflicts_with_all` on `program` can't see into a
        // merged toml value, so this one combination needs a manual check
        // -- a toml `media` and a standalone program are exactly as
        // mutually exclusive as --snapshot/--tape and a program already
        // are above.
        eprintln!("a standalone program and --game-config cannot be combined");
        return ExitCode::FAILURE;
    }

    let config = match &args.game_config {
        Some(path) => match inputs::load(&path.to_string_lossy())
            .and_then(|l| String::from_utf8(l.bytes.clone()).map_err(|e| e.to_string()))
        {
            Ok(text) => match game_config::parse(&text) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("failed to parse {}: {e}", path.display());
                    return ExitCode::FAILURE;
                }
            },
            Err(e) => {
                eprintln!("failed to read {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        },
        None => GameConfig::default(),
    };

    let mut resolved = match resolve(&args, &config) {
        Ok(resolved) => resolved,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let startup = match &machine_report {
        Ok(report) => report,
        Err(e) => {
            eprintln!("failed to discover machine plugins: {e}");
            return ExitCode::FAILURE;
        }
    };
    for warning in &startup.warnings {
        tracing::warn!(target: "retroclawd.startup", "plugin scan warning: {warning}");
    }
    for plugin in &startup.plugins {
        tracing::info!(
            target: "retroclawd.startup",
            "machine plugin {}@{} (ABI {}.{}) [{}] -> {}",
            plugin.plugin_id,
            plugin.plugin_version,
            plugin.abi.0,
            plugin.abi.1,
            plugin.source,
            plugin.machine_ids.join(", ")
        );
    }

    let selected_profile = startup
        .profiles
        .iter()
        .find(|p| p.machine_id == resolved.machine_id);
    let Some(selected_profile) = selected_profile else {
        let mut ids: Vec<String> = startup
            .profiles
            .iter()
            .map(|profile| profile.machine_id.clone())
            .collect();
        ids.sort();
        eprintln!(
            "unknown machine \"{}\"; discovered machines: {}",
            resolved.machine_id,
            ids.join(", ")
        );
        return ExitCode::FAILURE;
    };
    let declared_options = selected_profile.options.clone();
    let selected_media_kinds = selected_profile.media_kinds.clone();
    let selected_key_names = gui::keys::KeyNames {
        machine: selected_profile.key_names.clone(),
        all: machine_report
            .as_ref()
            .map(|report| {
                report
                    .profiles
                    .iter()
                    .flat_map(|profile| profile.key_names.iter().cloned())
                    .collect()
            })
            .unwrap_or_default(),
        axes: selected_profile.axis_names.clone(),
        all_axes: machine_report
            .as_ref()
            .map(|report| {
                report
                    .profiles
                    .iter()
                    .flat_map(|profile| profile.axis_names.iter().cloned())
                    .collect()
            })
            .unwrap_or_default(),
    };

    // `--config` names which file to load; with no flag, a `retroclawd.toml`
    // in the current directory is loaded automatically if present (unlike
    // --game-config, which is per-session and always explicit) -- see
    // `CommonArgs::config`'s own doc comment.
    let mut app_config = match load_app_config(args.config.as_ref()) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    romm::configure(&app_config.romm);

    if let Err(e) = validate_app_config(&app_config) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }

    // This run's own settings sit on top of the machine's sections: the game config's `[keys]`/`layout`, then `--hotkey`.
    overrides::apply_game_config(&mut app_config, &resolved.machine_id, &config);
    if let Err(e) =
        overrides::apply_hotkey_flags(&mut app_config, &resolved.machine_id, &args.hotkey)
    {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }

    if args.list_keys {
        let machine_id = &resolved.machine_id;
        let keymaps = match load_keymaps(&args, &app_config, machine_id) {
            Ok(files) => files,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let layout = match effective_layout(&args, &app_config, machine_id, &keymaps) {
            Ok(layout) => layout,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let listing = gui::hotkeys::Hotkeys::resolve(&app_config, machine_id)
            .and_then(|hotkeys| {
                gui::keys::KeyBindings::resolve(
                    &app_config,
                    machine_id,
                    &selected_key_names,
                    &hotkeys,
                    &keymaps,
                )
            })
            .and_then(|bindings| {
                gui::pads::PadBindings::resolve(&app_config, machine_id, &selected_key_names)
                    .map(|pads| (bindings, pads))
            });
        return match listing {
            Ok((bindings, pads)) => {
                println!(
                    "keyboard layout: {} ({})",
                    layout.name(),
                    match layout {
                        gui::keys::Layout::Positional =>
                            "a host key is the machine key at the same position",
                        gui::keys::Layout::Symbolic =>
                            "what a host key types picks the machine key",
                    }
                );
                for file in &keymaps {
                    println!(
                        "keymap file: {} ({})",
                        file.label,
                        if file.upper {
                            "--keymap"
                        } else {
                            "[input] keymap"
                        }
                    );
                }
                println!();
                println!("machine keys for {machine_id} (right side of [keys]):");
                print!(
                    "{}",
                    gui::keys::describe_names(machine_id, &selected_key_names.machine)
                );
                println!(
                    "\nhost keys (left side of [keys], optionally with ctrl+/shift+/alt+/super+):"
                );
                print!("{}", gui::keys::describe_host_keys());
                println!("\nbindings in effect:");
                print!("{}", bindings.describe());
                println!("\ngamepad buttons (left side of [pads.1], [pads.2], ...):");
                print!("{}", gui::pads::describe_buttons());
                println!("\npad bindings in effect (deadzone {}):", pads.deadzone);
                print!("{}", pads.describe());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }

    let firmware_mode = match resolve_firmware_mode(args.firmware, &app_config.romm) {
        Ok(mode) => mode,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let empty_options: HashMap<String, toml::Value> = HashMap::new();
    let app_config_options = app_config
        .machines
        .get(&resolved.machine_id)
        .map(|m| &m.options)
        .unwrap_or(&empty_options);
    let mut resolved_options = match resolve_machine_options(
        &resolved.machine_id,
        &declared_options,
        &matches,
        &config.options,
        app_config_options,
        firmware_mode == FirmwareMode::Auto,
    ) {
        Ok(resolved) => resolved,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let machine_id_for_gx4000_check = resolved.machine_id.clone();
    promote_gx4000_cpr_media(
        &machine_id_for_gx4000_check,
        &mut resolved,
        &mut resolved_options.rom_paths,
    );

    let mut firmware_blobs = Vec::new();
    if firmware_mode == FirmwareMode::Auto {
        let have: std::collections::HashSet<String> = resolved_options
            .rom_paths
            .iter()
            .map(|(key, _)| key.clone())
            .collect();
        match firmware::auto_fill(
            &resolved.machine_id,
            region_name(resolved.region.into()),
            &declared_options,
            &resolved_options.values,
            &have,
        ) {
            Ok(blobs) => firmware_blobs = blobs,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    }
    // Whatever is still missing after the firmware lookup is the usual error.
    for info in &resolved_options.missing_files {
        if !firmware_blobs.iter().any(|blob| blob.key == info.key) {
            eprintln!("{}", missing_option_error(&resolved.machine_id, info));
            return ExitCode::FAILURE;
        }
    }

    let auto_run_profile =
        match resolve_auto_run_profile(&args, &resolved, &resolved_options.values) {
            Ok(profile) => profile,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };

    if let Err(e) = debug_symbols::load_user_files(&overrides::symbol_files(&config, &args.symbols))
    {
        eprintln!("--symbols: {e}");
        return ExitCode::FAILURE;
    }

    let keymaps = match load_keymaps(&args, &app_config, &resolved.machine_id) {
        Ok(files) => files,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let mut app_settings = match resolve_app_settings(
        &args,
        &app_config,
        &resolved.machine_id,
        &selected_key_names,
        &keymaps,
    ) {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    // A machine that declares `MediaKind::Program` takes a `.prg` (`.tos`, `.ttp`) as media and decides how to run it
    // (the Atari ST builds a boot floppy around it); the C64 header path stays for every machine that does not.
    let mut program_media: Option<(String, Vec<u8>)> = None;
    let program = match &args.program {
        Some(path) => match inputs::load(path) {
            Ok(loaded) if program_goes_to_plugin(&loaded.name, &selected_media_kinds) => {
                program_media = Some((loaded.name.clone(), loaded.bytes.clone()));
                None
            }
            Ok(loaded) => {
                match resolve_program_load(&loaded.name, loaded.bytes.clone(), args.org) {
                    Ok(resolved) => Some(resolved),
                    Err(e) => {
                        eprintln!("{e}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            Err(e) => {
                eprintln!("failed to read {path}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };

    let mut rom_blobs = Vec::new();
    for (key, path) in &resolved_options.rom_paths {
        let bytes = match inputs::load(&path.to_string_lossy()) {
            Ok(loaded) => loaded.bytes.clone(),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        rom_blobs.push(BinaryBlob {
            key: key.clone(),
            bytes,
        });
    }

    rom_blobs.extend(firmware_blobs);

    let machine_options: Vec<NamedValue> = resolved_options
        .values
        .into_iter()
        .map(|(key, value)| NamedValue { key, value })
        .collect();
    let mut machine = match build_machine_from_rom_blobs(
        &resolved.machine_id,
        resolved.region.into(),
        rom_blobs,
        machine_options,
    ) {
        Ok(machine) => machine,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    machine.set_border(!args.no_border);

    if let Some(path) = &args.snapshot {
        let bytes = match inputs::load(path) {
            Ok(loaded) => loaded.bytes.clone(),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let extension = inputs::extension(path);
        if let Err(e) = machine.apply_snapshot(&extension, &bytes) {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }

    if let Some(path) = &args.tape {
        let bytes = match inputs::load(path) {
            Ok(loaded) => loaded.bytes.clone(),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let extension = inputs::extension(path);
        if let Err(e) = machine.apply_tape(&extension, &bytes, resolved.tape_speed) {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }

    match tape_save_policy(&args) {
        Ok(Some(policy)) => machine.set_tape_exit_policy(policy),
        Ok(None) => {}
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }

    if let Some(path) = &args.disk {
        let bytes = match inputs::load(path) {
            Ok(loaded) => loaded.bytes.clone(),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let extension = inputs::extension(path);
        if let Err(e) = machine.apply_disk(&extension, &bytes) {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
        if args.disk_writeback && !inputs::is_writable_file(path) {
            eprintln!(
                "--disk-writeback cannot rewrite {path}: it is a URL or a zip, not a plain file; \
                 use --disk-save <file> to write the modified image elsewhere"
            );
            return ExitCode::FAILURE;
        }
        if args.disk_writeback && extension == "g64" {
            eprintln!(
                "--disk-writeback cannot rewrite {path}: a .g64 is decoded to sectors when attached \
                 and is not written back as raw tracks; use --disk-save <file.d64> instead"
            );
            return ExitCode::FAILURE;
        }
        machine.set_disk_exit_policy(plugin_bridge::DiskExitPolicy {
            save_to: disk_save_target(&args, path),
        });
    }

    if let Err(e) = disk_writeback_needs_a_disk(&args) {
        eprintln!("{e}");
        return ExitCode::from(2);
    }
    if let Some(path) = &args.disk_b {
        let bytes = match inputs::load(path) {
            Ok(loaded) => loaded.bytes.clone(),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let extension = inputs::extension(path);
        if let Err(e) = machine.apply_disk_in(1, &extension, &bytes) {
            eprintln!("--disk-b: {e}");
            return ExitCode::FAILURE;
        }
        if args.disk_writeback && !inputs::is_writable_file(path) {
            eprintln!(
                "--disk-writeback cannot rewrite {path}: it is a URL or a zip, not a plain file; \
                 use --disk-b-save <file> to write the modified image elsewhere"
            );
            return ExitCode::FAILURE;
        }
        machine.set_disk_exit_policy_for(
            1,
            plugin_bridge::DiskExitPolicy {
                save_to: match (&args.disk_b_save, args.disk_writeback) {
                    (Some(save), _) => Some(std::path::PathBuf::from(save)),
                    (None, true) => Some(std::path::PathBuf::from(path)),
                    (None, false) => None,
                },
            },
        );
    }

    match parse_acsi_save(&args.acsi_save) {
        Ok(saves) => {
            for (target, path) in saves {
                machine.set_acsi_save(target, path);
            }
        }
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    }

    if let Some(path) = &args.cartridge {
        let bytes = match inputs::load(path) {
            Ok(loaded) => loaded.bytes.clone(),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let extension = inputs::extension(path);
        // `.img` is the Atari ST's other cartridge extension; it is only taken here, where the flag says what it is.
        let media_extension = if extension == "img" {
            "stc"
        } else {
            extension.as_str()
        };
        if !matches!(media_extension, "crt" | "cpr" | "stc") {
            eprintln!(
                "--cartridge expects a .crt (C64), .cpr (CPC Plus) or .stc/.img (Atari ST) file, not .{extension}"
            );
            return ExitCode::FAILURE;
        }
        if let Err(e) = machine.apply_media(media_extension, &bytes, resolved.tape_speed) {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
        machine.set_cartridge_exit_policy(plugin_bridge::CartridgeExitPolicy {
            save_to: args.cartridge_save.as_ref().map(std::path::PathBuf::from),
        });
    }

    if let Some(path) = &args.printer_out {
        machine.set_printer_out(std::path::PathBuf::from(path));
    }

    if let Some(path) = &resolved.media {
        let bytes = match inputs::load(path) {
            Ok(loaded) => loaded.bytes.clone(),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let extension = inputs::extension(path);
        if let Err(e) = machine.apply_media(&extension, &bytes, resolved.tape_speed) {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }

    // After --tape/--disk so save-state tape/FDC phase restores onto media
    // already attached to the machine; save states never embed media bytes.
    if let Some(path) = &args.load_state {
        let bytes = match inputs::load(path) {
            Ok(loaded) => loaded.bytes.clone(),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        if let Err(e) = machine.load_state(&bytes) {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }

    if let Some((name, bytes)) = &program_media
        && let Err(e) = machine.attach_program_file(name, bytes)
    {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }

    if let Some((org, program)) = program {
        if program.is_empty() {
            eprintln!(
                "{} is empty -- refusing to run",
                args.program.as_ref().unwrap()
            );
            return ExitCode::FAILURE;
        }
        if args.program.is_some() && auto_run_profile.is_some() {
            // `--run` with a program: the machine injects it once it has booted
            // (a BASIC program) and the startup profile types `RUN`; it must not
            // be jumped to as machine code.
            if let Err(e) = machine.attach_program(u64::from(org), &program) {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        } else {
            let written = machine.load(u64::from(org), &program);
            if written != program.len() {
                // A partially loaded program is worse than none: the missing tail is
                // usually the HALT, so the CPU would run off into unmapped 0xFF bytes
                // (RST 38h) and never stop.
                eprintln!(
                    "only {written} of {} program bytes were loaded at 0x{:04X} (some fell outside RAM) -- refusing to run",
                    program.len(),
                    org
                );
                return ExitCode::FAILURE;
            }
            machine.set_pc(u64::from(org));
        }
    }
    // No program given: boot straight from the ROM's own reset vector
    // (PC = 0x0000, the Z80 power-on default) -- e.g. into ZX81/Spectrum BASIC.

    if args.headless {
        if !plugin_runtime.debugger_plugins.is_empty() {
            // Debugger plugins (including the `debug-log` sink) attach to the
            // GUI's emulation thread; a batch run has none, so say so rather
            // than silently producing no log.
            eprintln!(
                "warning: --plugin debug-log/debug-ui/debug-zrcp selections are ignored with --headless (they attach to the GUI emulation thread)"
            );
        }
        let typing = match &args.type_text {
            Some(text) => {
                let router = gui::keys::KeyRouter::new(app_settings.key_bindings.clone())
                    .with_layout(app_settings.layout, app_settings.symbols.clone());
                let plan = gui::paste::plan(
                    &unescape_typed(text),
                    router.symbols(),
                    &router.plain_names(bevy::input::keyboard::KeyCode::Enter),
                    &router.plain_names(bevy::input::keyboard::KeyCode::ShiftLeft),
                    app_settings.paste_case,
                );
                if plan.typed == 0 {
                    eprintln!(
                        "--type: {} has no key for any of the {} characters",
                        resolved.machine_id, plan.skipped
                    );
                    return ExitCode::FAILURE;
                }
                if plan.skipped > 0 {
                    eprintln!(
                        "warning: --type: {} character(s) have no key on {} and are skipped",
                        plan.skipped, resolved.machine_id
                    );
                }
                // One key event per `paste_delay_ms`, like the paste hotkey, but in whole frames.
                let frame_ms = 1000 / machine.frame_hz().max(1);
                let delay_ms = args.type_delay.unwrap_or(app_settings.paste_delay_ms);
                let pace = (delay_ms / frame_ms.max(1)).max(1) as u32;
                Some(Typing {
                    events: plan.events,
                    after: args.type_after,
                    pace,
                })
            }
            None => None,
        };
        if let Some(frames) = args.frames
            && let Some(late) = args.at.iter().find(|a| a.frame > frames)
        {
            eprintln!(
                "--at: frame {} is after the last frame ({frames})",
                late.frame
            );
            return ExitCode::from(2);
        }
        let actions = scripted::Runner::new(
            &resolved.machine_id,
            emu_thread::MediaSettings {
                tape_speed: resolved.tape_speed,
                disk_writeback: args.disk_writeback,
                disk_save: args.disk_save.as_ref().map(std::path::PathBuf::from),
                disk_path: args.disk.as_ref().map(std::path::PathBuf::from),
                disk_b_save: args.disk_b_save.as_ref().map(std::path::PathBuf::from),
                disk_b_path: args.disk_b.as_ref().map(std::path::PathBuf::from),
                game: resolved
                    .media
                    .as_deref()
                    .or(args.program.as_deref())
                    .or(args.cartridge.as_deref())
                    .and_then(host_files::game_stem),
            },
            args.at.clone(),
        );
        run_headless(machine, &args, &resolved.machine_id, typing, actions)
    } else {
        if machine.render_frame().is_none() {
            eprintln!(
                "no video output implemented for {} yet",
                resolved.machine_id
            );
            return ExitCode::FAILURE;
        }
        let overlay_panels = match args.show_overlay.as_deref() {
            None => None,
            Some(spec) => match gui::OverlayPanels::parse(spec) {
                Ok(panels) => Some(panels),
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(2);
                }
            },
        };
        app_settings.media = emu_thread::MediaSettings {
            tape_speed: resolved.tape_speed,
            disk_writeback: args.disk_writeback,
            disk_save: args.disk_save.as_ref().map(std::path::PathBuf::from),
            disk_path: args.disk.as_ref().map(std::path::PathBuf::from),
            disk_b_save: args.disk_b_save.as_ref().map(std::path::PathBuf::from),
            disk_b_path: args.disk_b.as_ref().map(std::path::PathBuf::from),
            game: resolved
                .media
                .as_deref()
                .or(args.program.as_deref())
                .or(args.cartridge.as_deref())
                .and_then(host_files::game_stem),
        };
        app_settings.dialogs = dialogs::DialogSettings {
            machine_id: resolved.machine_id.clone(),
            media_kinds: selected_media_kinds,
            state_path: dialog_state_path(args.config.as_ref()),
            seeds: dialog_seeds(&args, &resolved),
            playlist: disk_playlist.clone(),
            work_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        };
        let (fullscreen, no_vsync, max_fps) = (
            app_settings.fullscreen,
            app_settings.no_vsync,
            app_settings.max_fps,
        );
        gui::run_gui(
            machine,
            &resolved.machine_id,
            overlay_panels,
            fullscreen,
            no_vsync,
            max_fps,
            args.run_ahead,
            auto_run_profile,
            plugin_runtime.debugger_plugins.clone(),
            app_settings,
        );
        ExitCode::SUCCESS
    }
}

/// The file dialogs' "last folder" memory lives next to the config file
/// (never inside it: the config is the user's to edit).
fn dialog_state_path(config: Option<&PathBuf>) -> PathBuf {
    config_dir(config).join("retroclawd.state.toml")
}

/// The folders of the media the session was started with: where a file
/// dialog opens the first time, before it has a remembered folder.
fn dialog_seeds(args: &CommonArgs, resolved: &Resolved) -> Vec<(dialogs::FileKind, PathBuf)> {
    let mut seeds = Vec::new();
    let mut add = |kind, path: &str| seeds.push((kind, PathBuf::from(path)));
    if let Some(path) = &resolved.media
        && let Some(kind) = dialogs::kind_of_extension(path)
    {
        add(kind, path);
    }
    if let Some(path) = &args.tape {
        add(dialogs::FileKind::Tape, path);
    }
    if let Some(path) = &args.disk {
        add(dialogs::FileKind::Disk, path);
    }
    if let Some(path) = &args.disk_b {
        add(dialogs::FileKind::Disk, path);
    }
    if let Some(path) = &args.snapshot {
        add(dialogs::FileKind::Snapshot, path);
    }
    if let Some(path) = &args.load_state {
        add(dialogs::FileKind::State, path);
    }
    seeds
}

/// `--disk-writeback` rewrites the file of every disk given, so it needs at least one.
fn disk_writeback_needs_a_disk(args: &CommonArgs) -> Result<(), String> {
    if args.disk_writeback && args.disk.is_none() && args.disk_b.is_none() {
        Err("--disk-writeback needs a --disk or --disk-b to write back".to_string())
    } else {
        Ok(())
    }
}

/// Where a modified `--disk` image should be written when the session ends:
/// `--acsi-save N=FILE` (repeatable): the ACSI target (0 to 7) and where its modified hard disk goes.
fn parse_acsi_save(specs: &[String]) -> Result<Vec<(u8, std::path::PathBuf)>, String> {
    specs
        .iter()
        .map(|spec| {
            let (n, file) = spec
                .split_once('=')
                .ok_or_else(|| format!("--acsi-save {spec:?}: expected N=FILE, N being 0 to 7"))?;
            let target: u8 = n
                .trim()
                .parse()
                .ok()
                .filter(|t| *t < 8)
                .ok_or_else(|| format!("--acsi-save {spec:?}: the target must be 0 to 7"))?;
            if file.is_empty() {
                return Err(format!("--acsi-save {spec:?}: no file after ="));
            }
            Ok((target, std::path::PathBuf::from(file)))
        })
        .collect()
}

/// `--disk-save`'s path, or the `--disk` file itself under
/// `--disk-writeback`; `None` (discard, with a warning) otherwise.
fn disk_save_target(args: &CommonArgs, disk_path: &str) -> Option<std::path::PathBuf> {
    match (&args.disk_save, args.disk_writeback) {
        (Some(path), _) => Some(std::path::PathBuf::from(path)),
        (None, true) => Some(std::path::PathBuf::from(disk_path)),
        (None, false) => None,
    }
}

/// `--tape-save`'s target and format (from the extension), or an error for an
/// extension that isn't a tape format the recorder can write.
fn tape_save_policy(args: &CommonArgs) -> Result<Option<plugin_bridge::TapeExitPolicy>, String> {
    let Some(path) = &args.tape_save else {
        return Ok(None);
    };
    let format = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !matches!(
        format.as_str(),
        "cdt" | "tzx" | "voc" | "tap" | "p" | "81" | "p81"
    ) {
        return Err(format!(
            "--tape-save {path}: the extension must be .cdt, .tzx or .voc (CPC), .tap (C64) or .p, .81 or .p81 (ZX81)"
        ));
    }
    Ok(Some(plugin_bridge::TapeExitPolicy {
        save_to: std::path::PathBuf::from(path),
        format,
    }))
}

/// Key events for `--type`, sent one every `pace` frames from frame `after`.
struct Typing {
    events: std::collections::VecDeque<(String, bool)>,
    after: u32,
    pace: u32,
}

/// `\n` in a `--type` argument is Enter (a shell cannot easily pass a newline) and `\\` a backslash.
fn unescape_typed(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// A 16-bit PCM `.wav` of interleaved `samples` (clamped to -1..1).
fn wav_bytes(samples: &[f32], format: machine::AudioFormat) -> Vec<u8> {
    let channels = u16::from(format.channels.max(1));
    let rate = format.sample_rate;
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2 * u32::from(channels)).to_le_bytes());
    out.extend_from_slice(&(2 * channels).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// `--capture-at ADDR:REG=FILE`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CaptureAt {
    addr: u16,
    reg: String,
    path: String,
}

fn parse_capture_at(text: &str) -> Result<CaptureAt, String> {
    let bad = || format!("`{text}`: expected ADDR:REG=FILE, e.g. 0xBB5A:A=out.txt");
    let (head, path) = text.split_once('=').ok_or_else(bad)?;
    let (addr, reg) = head.split_once(':').ok_or_else(bad)?;
    let digits = addr.trim_start_matches("0x").trim_start_matches('$');
    let radix = if digits.len() != addr.len() { 16 } else { 10 };
    let addr = u16::from_str_radix(digits, radix).map_err(|_| bad())?;
    if reg.is_empty() || path.is_empty() {
        return Err(bad());
    }
    Ok(CaptureAt {
        addr,
        reg: reg.to_string(),
        path: path.to_string(),
    })
}

/// The low byte of the register named `reg`: an exact name first (a 6502's `A`), else a Z80 half-register
/// (`A`/`F`/`B`/`C`/`D`/`E`/`H`/`L` out of `AF`/`BC`/`DE`/`HL`).
fn register_byte(regs: &[(&'static str, u64)], reg: &str) -> Option<u8> {
    let find = |name: &str| {
        regs.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|&(_, v)| v)
    };
    if let Some(v) = find(reg) {
        return Some(v as u8);
    }
    let upper = reg.to_ascii_uppercase();
    let (pair, high) = match upper.as_str() {
        "A" => ("AF", true),
        "F" => ("AF", false),
        "B" => ("BC", true),
        "C" => ("BC", false),
        "D" => ("DE", true),
        "E" => ("DE", false),
        "H" => ("HL", true),
        "L" => ("HL", false),
        _ => return None,
    };
    let v = find(pair)?;
    Some(if high { (v >> 8) as u8 } else { v as u8 })
}

fn run_headless(
    mut machine: plugin_bridge::HostedMachine,
    args: &CommonArgs,
    machine_id: &str,
    mut typing: Option<Typing>,
    mut actions: scripted::Runner,
) -> ExitCode {
    if let Some(frames) = args.frames {
        let started = std::time::Instant::now();
        let mut cycles = 0u64;
        let mut audio: Vec<f32> = Vec::new();
        let mut captured: Vec<u8> = Vec::new();
        for frame in 0..frames {
            actions.run_due(frame, &mut machine);
            if let Some(t) = typing.as_mut()
                && frame >= t.after
                && (frame - t.after) % t.pace == 0
                && let Some((key, pressed)) = t.events.pop_front()
            {
                machine.key_event_name(&key, pressed);
            }
            cycles += match &args.capture_at {
                None => emu_thread::advance_one_frame(&mut machine, |_| false),
                Some(cap) => emu_thread::advance_one_frame(&mut machine, |m| {
                    if m.pc() == u64::from(cap.addr)
                        && let Some(value) = register_byte(&m.registers(), &cap.reg)
                    {
                        captured.push(value);
                    }
                    false
                }),
            };
            if args.dump_audio.is_some() {
                audio.extend(machine.drain_audio());
            }
        }
        if let Some(path) = &args.dump_audio {
            match fs::write(
                path,
                wav_bytes(&audio, machine::Machine::audio_format(&machine)),
            ) {
                Ok(()) => println!("wrote {} audio samples to {path}", audio.len()),
                Err(e) => {
                    eprintln!("failed to write {path}: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        if let Some(cap) = &args.capture_at {
            match fs::write(&cap.path, &captured) {
                Ok(()) => println!("wrote {} captured bytes to {}", captured.len(), cap.path),
                Err(e) => {
                    eprintln!("failed to write {}: {e}", cap.path);
                    return ExitCode::FAILURE;
                }
            }
        }
        actions.run_due(frames, &mut machine); // `--at FRAMES:...`: after the last frame, before the dumps
        println!(
            "ran {frames} frames, {cycles} cycles, {:.3}s elapsed",
            started.elapsed().as_secs_f64()
        );
    } else {
        let mut executed = 0u64;
        let stats = machine.run(&mut |m| {
            if m.halted() {
                return false;
            }
            if args.max_cycles.is_some_and(|max| executed >= max) {
                return false;
            }
            executed += 1;
            true
        });

        let avg_hz = if stats.elapsed.as_secs_f64() > 0.0 {
            stats.cycles as f64 / stats.elapsed.as_secs_f64()
        } else {
            0.0
        };

        if machine.halted() {
            println!("halted");
        } else {
            println!(
                "stopped: reached --max-cycles limit ({}) without halting",
                args.max_cycles.unwrap()
            );
        }
        println!(
            "{} instructions, {} cycles, {:.3}s elapsed (~{:.0} Hz effective)",
            stats.instructions,
            stats.cycles,
            stats.elapsed.as_secs_f64(),
            avg_hz
        );
    }

    println!("registers:");
    for (name, value) in machine.registers() {
        println!("  {name:<4} = 0x{value:04X}");
    }

    if let Some((start, len)) = args.dump_memory {
        println!("memory 0x{start:04X}..0x{:04X}:", start as u32 + len as u32);
        for offset in 0..len {
            let addr = start.wrapping_add(offset);
            if offset % 16 == 0 {
                print!("  0x{addr:04X}:");
            }
            print!(" {:02X}", machine.read_memory(u64::from(addr)));
            if offset % 16 == 15 {
                println!();
            }
        }
        if len % 16 != 0 {
            println!();
        }
    }

    if let Some(path) = &args.dump_screen {
        match machine.render_frame() {
            Some((width, height, rgb)) => {
                if let Err(e) = write_png(path, width, height, &rgb) {
                    eprintln!("failed to write {path}: {e}");
                    return ExitCode::FAILURE;
                }
                println!("wrote {width}x{height} screen to {path}");
            }
            None => {
                eprintln!("no video output implemented for {} yet", machine_id);
                return ExitCode::FAILURE;
            }
        }
    }

    if let Some(path) = &args.dump_snapshot {
        let extension = inputs::extension(path);
        match machine.save_snapshot(&extension) {
            Ok(bytes) => {
                if let Err(e) = fs::write(path, &bytes) {
                    eprintln!("failed to write {path}: {e}");
                    return ExitCode::FAILURE;
                }
                println!("wrote {} bytes to {path}", bytes.len());
            }
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    }

    if let Some(path) = &args.dump_state {
        let bytes = machine.save_state();
        if let Err(e) = fs::write(path, &bytes) {
            eprintln!("failed to write {path}: {e}");
            return ExitCode::FAILURE;
        }
        println!("wrote {} bytes to {path}", bytes.len());
    }

    if actions.failed {
        eprintln!("an --at action failed");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_with(flags: &[&str]) -> CommonArgs {
        let mut argv = vec!["retroclawd"];
        argv.extend_from_slice(flags);
        CommonArgs::parse_from(argv)
    }

    fn app_settings_for(toml_text: &str, flags: &[&str], machine: &str) -> AppSettings {
        let config = app_config::parse(toml_text).unwrap();
        validate_app_config(&config).unwrap();
        resolve_app_settings(
            &args_with(flags),
            &config,
            machine,
            &gui::keys::KeyNames::default(),
            &[],
        )
        .unwrap()
    }

    #[test]
    fn paste_settings_default_then_machine_overrides_global() {
        let s = app_settings_for("", &[], "c64");
        assert_eq!(s.paste_case, gui::paste::PasteCase::Fold);
        assert_eq!(s.paste_delay_ms, gui::paste::DEFAULT_PASTE_DELAY_MS);

        let toml_text = r#"
            [input]
            paste_case = "exact"
            paste_delay_ms = 90

            [machines.c64.input]
            paste_case = "fold"
        "#;
        let c64 = app_settings_for(toml_text, &[], "c64");
        assert_eq!(c64.paste_case, gui::paste::PasteCase::Fold);
        assert_eq!(c64.paste_delay_ms, 90);
        let spectrum = app_settings_for(toml_text, &[], "zx-spectrum");
        assert_eq!(spectrum.paste_case, gui::paste::PasteCase::Exact);

        let bad = app_config::parse("[input]\npaste_case = \"upper\"").unwrap();
        assert!(
            resolve_paste_settings(&bad, "c64")
                .unwrap_err()
                .contains("paste_case")
        );
        let zero = app_config::parse("[input]\npaste_delay_ms = 0").unwrap();
        assert!(resolve_paste_settings(&zero, "c64").is_err());
    }

    fn window_and_pacing_settings_default_to_vsync_windowed() {
        let s = app_settings_for("", &[], "zx-spectrum");
        assert!(!s.fullscreen && !s.no_vsync);
        assert_eq!(s.max_fps, None);
    }

    fn toml_sets_fullscreen_vsync_and_max_fps_with_machine_over_default() {
        let toml_text = r#"
            [default]
            fullscreen = true
            max_fps = 50

            [machines.c64]
            fullscreen = false
            vsync = false
            max_fps = 120
        "#;
        let s = app_settings_for(toml_text, &[], "zx-spectrum");
        assert!(s.fullscreen && !s.no_vsync);
        assert_eq!(s.max_fps, Some(50));
        let s = app_settings_for(toml_text, &[], "c64");
        assert!(!s.fullscreen && s.no_vsync);
        assert_eq!(s.max_fps, Some(120));
    }

    fn vsync_off_in_toml_gets_the_same_60_fps_preset_as_the_flag() {
        let s = app_settings_for("[default]\nvsync = false", &[], "c64");
        assert!(s.no_vsync);
        assert_eq!(s.max_fps, Some(60));
    }

    fn cli_flags_beat_the_toml_for_window_and_pacing() {
        let toml_text = "[default]\nmax_fps = 50";
        let s = app_settings_for(
            toml_text,
            &["--fullscreen", "--no-vsync", "--max-fps", "75"],
            "c64",
        );
        assert!(s.fullscreen && s.no_vsync);
        assert_eq!(s.max_fps, Some(75));
    }
    /// Merged (every check kept): `window_and_pacing_settings_default_to_vsync_windowed`, `toml_sets_fullscreen_vsync_and_max_fps_with_machine_over_default`, `vsync_off_in_toml_gets_the_same_60_fps_preset_as_the_flag`, `cli_flags_beat_the_toml_for_window_and_pacing`.
    #[test]
    fn window_and_pacing_settings() {
        window_and_pacing_settings_default_to_vsync_windowed();
        toml_sets_fullscreen_vsync_and_max_fps_with_machine_over_default();
        vsync_off_in_toml_gets_the_same_60_fps_preset_as_the_flag();
        cli_flags_beat_the_toml_for_window_and_pacing();
    }

    #[test]
    fn app_config_rejects_zero_max_fps_and_machine_options_under_default() {
        let zero = app_config::parse("[machines.c64]\nmax_fps = 0").unwrap();
        assert!(validate_app_config(&zero).unwrap_err().contains("max_fps"));
        let rom_in_default = app_config::parse("[default]\nrom = \"x.rom\"").unwrap();
        assert!(
            validate_app_config(&rom_in_default)
                .unwrap_err()
                .contains("[machines.<id>]")
        );
    }

    #[test]
    fn hotkeys_resolve_per_machine_and_reject_a_default_hotkeys_section() {
        let toml_text = "[hotkeys]\nmute = \"f8\"\n[machines.c64.hotkeys]\nreset = \"ctrl+f8\"";
        let spectrum = app_settings_for(toml_text, &[], "zx-spectrum");
        let c64 = app_settings_for(toml_text, &[], "c64");
        let reset_text = |s: &AppSettings| {
            s.hotkeys
                .chords_for(gui::hotkeys::Action::Reset)
                .iter()
                .map(|c| c.to_text())
                .collect::<Vec<_>>()
        };
        assert_eq!(reset_text(&spectrum), ["KC_F12"]);
        assert_eq!(reset_text(&c64), ["ctrl+KC_F8"]);
        assert_eq!(
            c64.hotkeys.chords_for(gui::hotkeys::Action::Mute)[0].to_text(),
            "KC_F8"
        );

        let bad = app_config::parse("[default.hotkeys]\nreset = \"f1\"").unwrap();
        assert!(validate_app_config(&bad).unwrap_err().contains("[hotkeys]"));
        // A bad hotkey in the file is a startup error, not a silent no-op.
        let config = app_config::parse("[hotkeys]\nreset = \"f99\"").unwrap();
        assert!(
            resolve_app_settings(
                &args_with(&[]),
                &config,
                "c64",
                &gui::keys::KeyNames::default(),
                &[]
            )
            .err()
            .is_some_and(|e| e.contains("unknown key"))
        );
    }

    #[test]
    fn key_bindings_resolve_against_the_machines_key_names() {
        let names =
            gui::keys::KeyNames::only(&plugin_api::key_infos(["KeyA", "SymShift", "Colon"]));
        let toml_text =
            "[keys]\nsemicolon = \"colon\"\n[machines.c64.keys]\ncaps_lock = \"symbol_shift\"";
        let config = app_config::parse(toml_text).unwrap();
        validate_app_config(&config).unwrap();
        // `symbol_shift` is not one of these names' spellings, so c64 is an error...
        let err = resolve_app_settings(&args_with(&[]), &config, "c64", &names, &[])
            .err()
            .expect("unknown machine key");
        assert!(
            err.contains("symbol_shift") && err.contains("[machines.c64.keys]"),
            "{err}"
        );
        // ...but another machine only sees the global layer, which is fine.
        assert!(resolve_app_settings(&args_with(&[]), &config, "zx81", &names, &[]).is_ok());
    }

    fn the_old_keymap_section_and_a_default_keys_section_are_explained() {
        let old = app_config::parse("[keymap.default]\nleft = \"left\"").unwrap();
        let err = validate_app_config(&old).unwrap_err();
        assert!(err.contains("[keys]") && err.contains("[keymap]"), "{err}");
        let default_keys = app_config::parse("[default.keys]\nq = \"a\"").unwrap();
        assert!(
            validate_app_config(&default_keys)
                .unwrap_err()
                .contains("[keys]")
        );
    }

    #[test]
    fn the_layout_comes_from_the_flag_then_config_then_files_then_positional() {
        use gui::keys::{Layout, LoadedKeymap};
        let file = |layout: Option<&str>| LoadedKeymap {
            label: "f.toml".to_string(),
            upper: false,
            file: app_config::KeymapFile {
                layout: layout.map(str::to_string),
                ..Default::default()
            },
        };
        let none = app_config::parse("").unwrap();
        let global = app_config::parse("[input]\nlayout = \"symbolic\"").unwrap();
        let machine = app_config::parse(
            "[input]\nlayout = \"symbolic\"\n[machines.c64.input]\nlayout = \"positional\"",
        )
        .unwrap();
        let pick = |flags: &[&str], config: &AppConfig, files: &[LoadedKeymap]| {
            effective_layout(&args_with(flags), config, "c64", files)
        };
        assert_eq!(pick(&[], &none, &[]), Ok(Layout::Positional));
        assert_eq!(
            pick(&[], &none, &[file(Some("symbolic"))]),
            Ok(Layout::Symbolic)
        );
        assert_eq!(
            pick(&[], &global, &[file(Some("positional"))]),
            Ok(Layout::Symbolic)
        );
        assert_eq!(pick(&[], &machine, &[]), Ok(Layout::Positional));
        assert_eq!(
            pick(&["--layout", "symbolic"], &machine, &[]),
            Ok(Layout::Symbolic)
        );
        let err = pick(&["--layout", "qwertz"], &none, &[]).unwrap_err();
        assert!(err.contains("--layout") && err.contains("qwertz"), "{err}");
        let err = pick(
            &[],
            &app_config::parse("[input]\nlayout = \"x\"").unwrap(),
            &[],
        )
        .unwrap_err();
        assert!(err.contains("[input] layout"), "{err}");
    }

    #[test]
    fn keymap_files_are_read_relative_to_the_config_and_errors_name_the_file() {
        let dir = std::env::temp_dir().join(format!("retroclawd-keymaps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.toml"),
            "layout = \"symbolic\"\n[keys]\nsemicolon = \"colon\"",
        )
        .unwrap();
        std::fs::write(dir.join("bad.toml"), "[hotkeys]\nreset = \"f1\"").unwrap();
        let config_path = dir.join("retroclawd.toml");
        let config = app_config::parse(
            "[input]\nkeymap = \"a.toml\"\n[machines.c64.input]\nkeymap = [\"a.toml\"]",
        )
        .unwrap();
        let mut args = args_with(&["--config", config_path.to_str().unwrap()]);
        let files = load_keymaps(&args, &config, "c64").unwrap();
        assert_eq!(files.len(), 2, "the global and the machine entry");
        assert!(
            files
                .iter()
                .all(|f| !f.upper && f.file.layout.as_deref() == Some("symbolic"))
        );
        assert!(files[0].label.ends_with("a.toml"));

        // A file given with --keymap is the highest layer, relative to the working directory.
        args.keymap = vec![dir.join("a.toml")];
        let files = load_keymaps(&args, &app_config::parse("").unwrap(), "c64").unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].upper);

        args.keymap = vec![dir.join("bad.toml")];
        let err = load_keymaps(&args, &app_config::parse("").unwrap(), "c64").unwrap_err();
        assert!(err.contains("bad.toml") && err.contains("hotkeys"), "{err}");
        args.keymap = vec![dir.join("missing.toml")];
        let err = load_keymaps(&args, &app_config::parse("").unwrap(), "c64").unwrap_err();
        assert!(err.contains("missing.toml"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn a_default_input_section_is_explained() {
        let config = app_config::parse("[default.input]\nlayout = \"symbolic\"").unwrap();
        let err = validate_app_config(&config).unwrap_err();
        assert!(
            err.contains("[input]") && err.contains("[default.input]"),
            "{err}"
        );
        let ok = app_config::parse("[input]\nlayout = \"symbolic\"").unwrap();
        assert!(validate_app_config(&ok).is_ok());
    }

    fn a_default_pads_section_is_explained() {
        let config = app_config::parse("[default.pads.1]\nsouth = \"x\"").unwrap();
        let err = validate_app_config(&config).unwrap_err();
        assert!(
            err.contains("[pads]") && err.contains("[default.pads]"),
            "{err}"
        );
        // Real pad sections are fine, and the machine's `pads` table is not
        // mistaken for a machine option.
        let ok = app_config::parse("[pads.1]\nsouth = \"x\"\n[machines.c64.pads.2]\nstart = \"x\"")
            .unwrap();
        assert!(validate_app_config(&ok).is_ok());
        assert!(!ok.machines["c64"].options.contains_key("pads"));
    }
    /// Merged (every check kept): `the_old_keymap_section_and_a_default_keys_section_are_explained`, `a_default_input_section_is_explained`, `a_default_pads_section_is_explained`.
    #[test]
    fn default_sections_are_explained() {
        the_old_keymap_section_and_a_default_keys_section_are_explained();
        a_default_input_section_is_explained();
        a_default_pads_section_is_explained();
    }

    #[test]
    fn flat_machine_section_options_are_validated_against_the_declared_schema() {
        let declared = [enum_option("tape_mode", "tape-mode", &["a", "b"], "a")];
        let config = app_config::parse(
            "[machines.cpc464]\nshader = \"crt\"\ntape_mode = \"b\"\ntape_mdoe = \"a\"",
        )
        .unwrap();
        let options = &config.machines["cpc464"].options;
        assert!(!options.contains_key("shader"));
        let error = validate_declared_option_keys(&declared, options, "cfg").unwrap_err();
        assert!(
            error.contains("tape_mdoe") && error.contains("max_fps"),
            "{error}"
        );

        let old_table = app_config::parse("[machines.cpc464.options]\nrom = \"x\"").unwrap();
        let error =
            validate_declared_option_keys(&declared, &old_table.machines["cpc464"].options, "cfg")
                .unwrap_err();
        assert!(error.contains("nested `options` table is gone"), "{error}");
    }

    fn debug_log_shorthand_maps_output_then_level() {
        // The exact selector shape from a real bug report (relative Windows path).
        let runtime = apply_plugin_runtime(&[r"debug-log:.\output.log:debug".to_string()]).unwrap();
        let log = &runtime.debugger_plugins[0];
        assert_eq!(log.selector, "debug-log");
        let pairs: Vec<(&str, &plugin_api::DebugValue)> = log
            .options
            .iter()
            .map(|o| (o.key.as_str(), &o.value))
            .collect();
        assert_eq!(
            pairs,
            vec![
                (
                    "output",
                    &plugin_api::DebugValue::Text(r".\output.log".into())
                ),
                ("level", &plugin_api::DebugValue::Text("debug".into())),
            ]
        );
    }

    fn windows_drive_letters_stay_attached_to_their_path() {
        assert_eq!(
            split_plugin_tokens(r"C:\logs\out.log:debug"),
            vec![r"C:\logs\out.log".to_string(), "debug".to_string()]
        );
        assert_eq!(
            split_plugin_tokens("d:/x/out.log"),
            vec!["d:/x/out.log".to_string()]
        );
        // A one-letter token that isn't followed by a path separator is just
        // a token (e.g. a one-letter level/value).
        assert_eq!(
            split_plugin_tokens("a:b"),
            vec!["a".to_string(), "b".to_string()]
        );
        let runtime =
            apply_plugin_runtime(&[r"debug-log:C:\logs\out.log:debug".to_string()]).unwrap();
        let log = &runtime.debugger_plugins[0];
        assert_eq!(log.options[0].key, "output");
        assert_eq!(
            log.options[0].value,
            plugin_api::DebugValue::Text(r"C:\logs\out.log".into())
        );
        assert_eq!(log.options[1].key, "level");
    }
    /// Merged (every check kept): `debug_log_shorthand_maps_output_then_level`, `windows_drive_letters_stay_attached_to_their_path`.
    #[test]
    fn debug_log_shorthand() {
        debug_log_shorthand_maps_output_then_level();
        windows_drive_letters_stay_attached_to_their_path();
    }

    #[test]
    fn acsi_save_takes_a_target_and_a_file() {
        let ok = parse_acsi_save(&["0=a.img".to_string(), "7=dir/b.img".to_string()]).unwrap();
        assert_eq!(ok.len(), 2);
        assert_eq!(ok[1].0, 7);
        for bad in ["8=a.img", "x=a.img", "a.img", "0=", "-1=a"] {
            assert!(parse_acsi_save(&[bad.to_string()]).is_err(), "{bad}");
        }
    }

    #[test]
    fn disk_save_target_prefers_disk_save_then_writeback_then_nothing() {
        let a = args_with(&["--disk", "g.dsk", "--disk-save", "out.dsk"]);
        assert_eq!(
            disk_save_target(&a, "g.dsk"),
            Some(std::path::PathBuf::from("out.dsk"))
        );
        let a = args_with(&["--disk", "g.dsk", "--disk-writeback"]);
        assert_eq!(
            disk_save_target(&a, "g.dsk"),
            Some(std::path::PathBuf::from("g.dsk"))
        );
        let a = args_with(&["--disk", "g.dsk"]);
        assert_eq!(disk_save_target(&a, "g.dsk"), None, "default never writes");
    }

    #[test]
    fn tape_save_picks_its_format_from_the_extension() {
        let none = args_with(&[]);
        assert!(tape_save_policy(&none).unwrap().is_none());
        for (path, format) in [
            ("a.cdt", "cdt"),
            ("b.TZX", "tzx"),
            ("c.voc", "voc"),
            ("d.tap", "tap"),
            ("e.p", "p"),
            ("f.81", "81"),
            ("g.P81", "p81"),
        ] {
            let policy = tape_save_policy(&args_with(&["--tape-save", path]))
                .unwrap()
                .unwrap();
            assert_eq!(policy.format, format);
            assert_eq!(policy.save_to, std::path::PathBuf::from(path));
        }
        let error = tape_save_policy(&args_with(&["--tape-save", "out.wav"])).unwrap_err();
        assert!(error.contains(".cdt"), "{error}");
        assert!(tape_save_policy(&args_with(&["--tape-save", "noext"])).is_err());
    }

    #[test]
    fn disk_save_and_writeback_need_a_disk_and_exclude_each_other() {
        let try_parse = |flags: &[&str]| {
            let mut argv = vec!["retroclawd"];
            argv.extend_from_slice(flags);
            CommonArgs::try_parse_from(argv)
        };
        assert!(try_parse(&["--disk-save", "o.dsk"]).is_err(), "no --disk");
        // Writeback rewrites whichever disks were given (the check is `disk_writeback_needs_a_disk`).
        let needs = |flags: &[&str]| disk_writeback_needs_a_disk(&try_parse(flags).unwrap());
        assert!(needs(&["--disk-writeback"]).is_err(), "no disk at all");
        assert!(needs(&["--disk", "a.dsk", "--disk-writeback"]).is_ok());
        assert!(needs(&["--disk-b", "b.dsk", "--disk-writeback"]).is_ok());
        assert!(needs(&["--disk", "a.dsk"]).is_ok());
        assert!(
            try_parse(&["--disk-b-save", "o.dsk"]).is_err(),
            "no --disk-b"
        );
        assert!(
            try_parse(&[
                "--disk-b",
                "b.dsk",
                "--disk-b-save",
                "o.dsk",
                "--disk-writeback"
            ])
            .is_err(),
            "mutually exclusive"
        );
        assert!(try_parse(&["--disk-b", "b.dsk", "--disk-b-save", "o.dsk"]).is_ok());
        assert!(
            try_parse(&[
                "--disk",
                "g.dsk",
                "--disk-save",
                "o.dsk",
                "--disk-writeback"
            ])
            .is_err(),
            "mutually exclusive"
        );
        assert!(try_parse(&["--disk", "g.dsk", "--disk-writeback"]).is_ok());
    }

    #[test]
    fn a_wav_is_a_16_bit_pcm_riff_of_the_clamped_samples() {
        let stereo = wav_bytes(
            &[0.0, 0.5],
            machine::AudioFormat {
                channels: 2,
                sample_rate: 48_000,
            },
        );
        assert_eq!(u16::from_le_bytes([stereo[22], stereo[23]]), 2, "stereo");
        assert_eq!(
            u32::from_le_bytes(stereo[28..32].try_into().unwrap()),
            192_000,
            "byte rate"
        );
        assert_eq!(
            u16::from_le_bytes([stereo[32], stereo[33]]),
            4,
            "block align"
        );

        let wav = wav_bytes(&[0.0, 0.5, -1.0, 2.0], machine::AudioFormat::MONO);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([wav[20], wav[21]]), 1, "PCM");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1, "mono");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 44_100);
        assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16, "bits");
        assert_eq!(
            u32::from_le_bytes(wav[40..44].try_into().unwrap()),
            8,
            "data bytes"
        );
        let samples: Vec<i16> = wav[44..]
            .chunks(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(samples, [0, 16383, -32767, 32767], "clamped to -1..1");
        assert_eq!(
            u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize,
            wav.len() - 8
        );
    }

    #[test]
    fn capture_at_parses_an_address_register_and_file() {
        let c = parse_capture_at("0xBB5A:A=out.txt").unwrap();
        assert_eq!(
            (c.addr, c.reg.as_str(), c.path.as_str()),
            (0xBB5A, "A", "out.txt")
        );
        assert_eq!(parse_capture_at("$FFD2:a=x").unwrap().addr, 0xFFD2);
        assert_eq!(parse_capture_at("16:A=x").unwrap().addr, 16);
        assert!(parse_capture_at("0xBB5A:A").is_err() && parse_capture_at("zz:A=x").is_err());
        let z80 = [("AF", 0x4182u64), ("BC", 0x1234)];
        assert_eq!(register_byte(&z80, "A"), Some(0x41));
        assert_eq!(register_byte(&z80, "f"), Some(0x82));
        assert_eq!(register_byte(&z80, "C"), Some(0x34));
        assert_eq!(
            register_byte(&[("A", 0x1FF)], "A"),
            Some(0xFF),
            "an exact name wins (6502)"
        );
        assert_eq!(register_byte(&z80, "X"), None);
        assert!(
            CommonArgs::try_parse_from([
                "retroclawd",
                "zx81",
                "--headless",
                "--capture-at",
                "0:A=x"
            ])
            .is_err(),
            "needs --frames"
        );
    }

    fn dump_audio_needs_frames() {
        let try_parse = |flags: &[&str]| {
            let mut argv = vec!["retroclawd", "zx81"];
            argv.extend_from_slice(flags);
            CommonArgs::try_parse_from(argv)
        };
        assert!(try_parse(&["--headless", "--dump-audio", "a.wav"]).is_err());
        assert!(try_parse(&["--headless", "--frames", "5", "--dump-audio", "a.wav"]).is_ok());
    }

    fn frames_is_headless_only_and_excludes_max_cycles() {
        let try_parse = |flags: &[&str]| {
            let mut argv = vec!["retroclawd"];
            argv.extend_from_slice(flags);
            CommonArgs::try_parse_from(argv)
        };
        assert!(try_parse(&["--frames", "50"]).is_err(), "needs --headless");
        assert!(
            try_parse(&["--headless", "--frames", "50", "--max-cycles", "9"]).is_err(),
            "mutually exclusive"
        );
        let args = try_parse(&["--headless", "--frames", "50"]).unwrap();
        assert_eq!(args.frames, Some(50));
    }

    fn type_needs_frames_and_type_after_needs_type() {
        let try_parse = |flags: &[&str]| {
            let mut argv = vec!["retroclawd"];
            argv.extend_from_slice(flags);
            CommonArgs::try_parse_from(argv)
        };
        assert!(
            try_parse(&["--headless", "--type", "RUN"]).is_err(),
            "--type needs --frames: a run that stops at the first HALT never reaches the prompt"
        );
        assert!(
            try_parse(&["--frames", "50", "--type", "RUN"]).is_err(),
            "needs --headless"
        );
        assert!(
            try_parse(&["--headless", "--frames", "50", "--type-after", "9"]).is_err(),
            "--type-after without --type"
        );
        let args = try_parse(&["--headless", "--frames", "400", "--type", "RUN"]).unwrap();
        assert_eq!(args.type_text.as_deref(), Some("RUN"));
        assert_eq!(args.type_after, 150, "default");
        let args = try_parse(&[
            "--headless",
            "--frames",
            "900",
            "--type",
            "RUN",
            "--type-after",
            "500",
        ])
        .unwrap();
        assert_eq!(args.type_after, 500);
        assert_eq!(
            args.type_delay, None,
            "defaults to the configuration's paste delay"
        );
        let args = try_parse(&[
            "--headless",
            "--frames",
            "900",
            "--type",
            "RUN",
            "--type-delay",
            "160",
        ])
        .unwrap();
        assert_eq!(args.type_delay, Some(160));
        assert!(
            try_parse(&["--headless", "--frames", "9", "--type-delay", "160"]).is_err(),
            "--type-delay without --type"
        );
    }
    /// Merged (every check kept): `dump_audio_needs_frames`, `frames_is_headless_only_and_excludes_max_cycles`, `type_needs_frames_and_type_after_needs_type`.
    #[test]
    fn options_that_need_frames() {
        dump_audio_needs_frames();
        frames_is_headless_only_and_excludes_max_cycles();
        type_needs_frames_and_type_after_needs_type();
    }

    #[test]
    fn typed_text_understands_backslash_n_and_backslash_backslash() {
        assert_eq!(unescape_typed("RUN\\n"), "RUN\n");
        assert_eq!(unescape_typed("A\\\\B"), "A\\B");
        assert_eq!(
            unescape_typed("a\\qb"),
            "a\\qb",
            "an unknown escape is kept"
        );
        assert_eq!(
            unescape_typed("end\\"),
            "end\\",
            "a trailing backslash is kept"
        );
        assert_eq!(unescape_typed("plain"), "plain");
    }

    fn resolved_with_media(media: Option<&str>) -> Resolved {
        Resolved {
            machine_id: "c64".to_string(),
            region: RegionArg::Pal,
            media: media.map(str::to_string),
            tape_speed: 1,
        }
    }

    fn run_requested() -> HashMap<String, DebugValue> {
        let mut options = HashMap::new();
        options.insert("run".to_string(), DebugValue::Bool(true));
        options
    }

    #[test]
    fn resolve_auto_run_profile_returns_none_when_run_was_not_requested() {
        let args = args_with(&[]);
        let resolved = resolved_with_media(None);
        assert_eq!(
            resolve_auto_run_profile(&args, &resolved, &HashMap::new()).unwrap(),
            None
        );
    }

    fn c128_resolved() -> Resolved {
        Resolved {
            machine_id: "c128".to_string(),
            ..resolved_with_media(None)
        }
    }

    #[test]
    fn resolve_auto_run_profile_picks_the_c128_startup_command_by_mode_and_media() {
        let mut c64_mode = run_requested();
        c64_mode.insert("mode".to_string(), DebugValue::Text("c64".to_string()));
        let disk = args_with(&["--disk", "game.d64"]);
        let prg = args_with(&["--program", "game.prg"]);
        let pick = |args: &CommonArgs, options: &HashMap<String, DebugValue>| {
            resolve_auto_run_profile(args, &c128_resolved(), options).unwrap()
        };
        assert_eq!(
            pick(&disk, &run_requested()),
            Some(gui::AutoRunProfile::C128Disk)
        );
        assert_eq!(
            pick(&disk, &c64_mode),
            Some(gui::AutoRunProfile::C128C64Disk)
        );
        assert_eq!(
            pick(&prg, &run_requested()),
            Some(gui::AutoRunProfile::C128Run)
        );
        assert_eq!(pick(&prg, &c64_mode), Some(gui::AutoRunProfile::C64Run));
        let tape = args_with(&["--tape", "x.tap"]);
        assert!(resolve_auto_run_profile(&tape, &c128_resolved(), &run_requested()).is_err());
    }

    fn resolve_auto_run_profile_picks_c64_load_for_an_attached_d64_disk() {
        let args = args_with(&["--disk", "game.d64"]);
        let resolved = resolved_with_media(None);
        assert_eq!(
            resolve_auto_run_profile(&args, &resolved, &run_requested()).unwrap(),
            Some(gui::AutoRunProfile::C64Load)
        );
    }

    fn resolve_auto_run_profile_picks_c64_load_for_an_attached_d71_disk() {
        let args = args_with(&["--disk", "game.d71"]);
        let resolved = resolved_with_media(None);
        assert_eq!(
            resolve_auto_run_profile(&args, &resolved, &run_requested()).unwrap(),
            Some(gui::AutoRunProfile::C64Load)
        );
    }

    fn resolve_auto_run_profile_picks_c64_load_for_an_attached_d81_disk() {
        let args = args_with(&["--disk", "game.d81"]);
        let resolved = resolved_with_media(None);
        assert_eq!(
            resolve_auto_run_profile(&args, &resolved, &run_requested()).unwrap(),
            Some(gui::AutoRunProfile::C64Load)
        );
    }

    fn resolve_auto_run_profile_picks_c64_run_for_a_prg_program() {
        let args = args_with(&["--program", "game.prg"]);
        let resolved = resolved_with_media(None);
        assert_eq!(
            resolve_auto_run_profile(&args, &resolved, &run_requested()).unwrap(),
            Some(gui::AutoRunProfile::C64Run)
        );
    }

    fn resolve_auto_run_profile_picks_c64_run_for_a_t64_tape() {
        let args = args_with(&["--tape", "game.t64"]);
        let resolved = resolved_with_media(None);
        assert_eq!(
            resolve_auto_run_profile(&args, &resolved, &run_requested()).unwrap(),
            Some(gui::AutoRunProfile::C64Run)
        );
    }
    /// Merged (every check kept): `resolve_auto_run_profile_picks_c64_load_for_an_attached_d64_disk`, `resolve_auto_run_profile_picks_c64_load_for_an_attached_d71_disk`, `resolve_auto_run_profile_picks_c64_load_for_an_attached_d81_disk`, `resolve_auto_run_profile_picks_c64_run_for_a_prg_program`, `resolve_auto_run_profile_picks_c64_run_for_a_t64_tape`.
    #[test]
    fn resolve_auto_run_profile_for_c64_media() {
        resolve_auto_run_profile_picks_c64_load_for_an_attached_d64_disk();
        resolve_auto_run_profile_picks_c64_load_for_an_attached_d71_disk();
        resolve_auto_run_profile_picks_c64_load_for_an_attached_d81_disk();
        resolve_auto_run_profile_picks_c64_run_for_a_prg_program();
        resolve_auto_run_profile_picks_c64_run_for_a_t64_tape();
    }

    #[test]
    fn resolve_auto_run_profile_types_load_then_run_for_a_c64_tap_cassette() {
        let args = args_with(&["--tape", "game.tap"]);
        let resolved = resolved_with_media(None);
        assert_eq!(
            resolve_auto_run_profile(&args, &resolved, &run_requested()).unwrap(),
            Some(gui::AutoRunProfile::C64Tape)
        );
    }

    #[test]
    fn resolve_auto_run_profile_rejects_more_than_one_media_source() {
        let args = args_with(&["--program", "a.prg", "--disk", "b.d64"]);
        let resolved = resolved_with_media(None);
        let err = resolve_auto_run_profile(&args, &resolved, &run_requested()).unwrap_err();
        assert!(err.contains("ambiguous"), "got: {err}");
    }

    fn bool_option(key: &str, flag: &str) -> MachineOptionInfo {
        MachineOptionInfo {
            key: key.to_string(),
            flag: flag.to_string(),
            short: None,
            kind: MachineOptionKind::Bool,
            default: Some(DebugValue::Bool(false)),
            required: false,
            description: String::new(),
        }
    }

    fn enum_option(key: &str, flag: &str, choices: &[&str], default: &str) -> MachineOptionInfo {
        MachineOptionInfo {
            key: key.to_string(),
            flag: flag.to_string(),
            short: None,
            kind: MachineOptionKind::Enum {
                choices: choices.iter().map(|s| s.to_string()).collect(),
            },
            default: Some(DebugValue::Text(default.to_string())),
            required: false,
            description: String::new(),
        }
    }

    fn required_file_option(key: &str, flag: &str) -> MachineOptionInfo {
        MachineOptionInfo {
            key: key.to_string(),
            flag: flag.to_string(),
            short: None,
            kind: MachineOptionKind::File,
            default: None,
            required: true,
            description: String::new(),
        }
    }

    fn file_set_option(key: &str, flag: &str) -> MachineOptionInfo {
        MachineOptionInfo {
            key: key.to_string(),
            flag: flag.to_string(),
            short: None,
            kind: MachineOptionKind::FileSet {
                min_index: 1,
                max_index: 15,
            },
            default: None,
            required: false,
            description: String::new(),
        }
    }

    fn matches_for(declared: &[MachineOptionInfo], argv: &[&str]) -> clap::ArgMatches {
        let cmd = build_command(Some("test-machine"), declared);
        cmd.get_matches_from(argv)
    }

    #[test]
    fn resolve_prefers_cli_flag_over_game_config_over_app_config_over_default() {
        let info = enum_option("tape_mode", "tape-mode", &["caprice", "zesarux"], "caprice");
        let declared = [info.clone()];

        // Nothing given anywhere: falls through to the plugin's own default.
        let matches = matches_for(&declared, &["retroclawd"]);
        let empty = HashMap::new();
        let resolved = resolve_one_machine_option(&info, &matches, &empty, &empty).unwrap();
        assert!(matches!(
            resolved,
            Some(ResolvedOption::Scalar(DebugValue::Text(t))) if t == "caprice"
        ));

        // app-config only: wins over the default.
        let mut app_config_options = HashMap::new();
        app_config_options.insert(
            "tape_mode".to_string(),
            toml::Value::String("zesarux".to_string()),
        );
        let resolved =
            resolve_one_machine_option(&info, &matches, &empty, &app_config_options).unwrap();
        assert!(matches!(
            resolved,
            Some(ResolvedOption::Scalar(DebugValue::Text(t))) if t == "zesarux"
        ));

        // game-config wins over app-config.
        let mut game_config_options = HashMap::new();
        game_config_options.insert(
            "tape_mode".to_string(),
            toml::Value::String("caprice".to_string()),
        );
        let resolved =
            resolve_one_machine_option(&info, &matches, &game_config_options, &app_config_options)
                .unwrap();
        assert!(matches!(
            resolved,
            Some(ResolvedOption::Scalar(DebugValue::Text(t))) if t == "caprice"
        ));

        // An explicit CLI flag wins over everything else.
        let matches = matches_for(&declared, &["retroclawd", "--tape-mode", "zesarux"]);
        let resolved =
            resolve_one_machine_option(&info, &matches, &game_config_options, &app_config_options)
                .unwrap();
        assert!(matches!(
            resolved,
            Some(ResolvedOption::Scalar(DebugValue::Text(t))) if t == "zesarux"
        ));
    }

    #[test]
    fn resolve_bool_flag_only_ever_forces_true_never_false() {
        let info = bool_option("turbosound", "turbosound");
        let declared = [info.clone()];
        let empty = HashMap::new();

        let matches = matches_for(&declared, &["retroclawd", "--turbosound"]);
        let resolved = resolve_one_machine_option(&info, &matches, &empty, &empty).unwrap();
        assert!(matches!(
            resolved,
            Some(ResolvedOption::Scalar(DebugValue::Bool(true)))
        ));

        // Absent flag falls through to the declared default (false).
        let matches = matches_for(&declared, &["retroclawd"]);
        let resolved = resolve_one_machine_option(&info, &matches, &empty, &empty).unwrap();
        assert!(matches!(
            resolved,
            Some(ResolvedOption::Scalar(DebugValue::Bool(false)))
        ));
    }

    #[test]
    fn resolve_file_option_reads_cli_path_into_rom_path() {
        let info = required_file_option("rom", "rom");
        let declared = [info.clone()];
        let empty = HashMap::new();
        let matches = matches_for(&declared, &["retroclawd", "--rom", "game.rom"]);
        let resolved = resolve_one_machine_option(&info, &matches, &empty, &empty).unwrap();
        match resolved {
            Some(ResolvedOption::RomPath(path)) => {
                assert_eq!(path, PathBuf::from("game.rom"));
            }
            other => panic!("expected RomPath, got {other:?}"),
        }
    }

    #[test]
    fn resolve_file_set_option_parses_repeatable_n_equals_path() {
        let info = file_set_option("rom_slot", "rom-slot");
        let declared = [info.clone()];
        let empty = HashMap::new();
        let matches = matches_for(
            &declared,
            &[
                "retroclawd",
                "--rom-slot",
                "4=extra.rom",
                "--rom-slot",
                "7=amsdos.rom",
            ],
        );
        let resolved = resolve_one_machine_option(&info, &matches, &empty, &empty).unwrap();
        match resolved {
            Some(ResolvedOption::RomPathSet(mut entries)) => {
                entries.sort_by_key(|(slot, _)| *slot);
                assert_eq!(
                    entries,
                    vec![
                        (4, PathBuf::from("extra.rom")),
                        (7, PathBuf::from("amsdos.rom")),
                    ]
                );
            }
            other => panic!("expected RomPathSet, got {other:?}"),
        }
    }

    #[test]
    fn resolve_machine_options_errors_on_missing_required_option() {
        let declared = vec![required_file_option("rom", "rom")];
        let matches = matches_for(&declared, &["retroclawd"]);
        let empty = HashMap::new();
        let err = resolve_machine_options("cpc6128", &declared, &matches, &empty, &empty, false)
            .expect_err("missing required rom should error");
        assert!(err.contains("missing required option \"rom\""));
        assert!(err.contains("cpc6128"));
    }

    #[test]
    fn a_missing_required_file_can_be_deferred_to_the_firmware_lookup() {
        let declared = vec![required_file_option("rom", "rom")];
        let matches = matches_for(&declared, &["retroclawd"]);
        let empty = HashMap::new();
        let resolved =
            resolve_machine_options("cpc6128", &declared, &matches, &empty, &empty, true)
                .expect("deferred, not an error");
        assert_eq!(resolved.missing_files.len(), 1);
        assert_eq!(resolved.missing_files[0].key, "rom");
        let err = missing_option_error("cpc6128", &resolved.missing_files[0]);
        assert!(err.contains("--firmware auto"), "{err}");
    }

    #[test]
    fn the_firmware_mode_comes_from_the_flag_then_the_config() {
        let mut romm = app_config::RommConfig::default();
        assert!(resolve_firmware_mode(None, &romm).unwrap() == FirmwareMode::Off);
        romm.firmware = Some("auto".into());
        assert!(resolve_firmware_mode(None, &romm).unwrap() == FirmwareMode::Auto);
        assert!(
            resolve_firmware_mode(Some(FirmwareMode::Off), &romm).unwrap() == FirmwareMode::Off
        );
        romm.firmware = Some("sometimes".into());
        assert!(resolve_firmware_mode(None, &romm).is_err());
    }

    #[test]
    fn resolve_machine_options_rejects_unknown_config_key_with_available_list() {
        let declared = vec![bool_option("turbosound", "turbosound")];
        let matches = matches_for(&declared, &["retroclawd"]);
        let mut game_config_options = HashMap::new();
        game_config_options.insert("typo_key".to_string(), toml::Value::Boolean(true));
        let empty = HashMap::new();
        let err = resolve_machine_options(
            "zx-spectrum-128",
            &declared,
            &matches,
            &game_config_options,
            &empty,
            false,
        )
        .expect_err("unknown option key should error");
        assert!(err.contains("unknown option \"typo_key\""));
        assert!(err.contains("turbosound"));
    }

    #[test]
    fn resolve_machine_options_splits_scalar_values_from_rom_paths() {
        let declared = vec![
            required_file_option("rom", "rom"),
            bool_option("turbosound", "turbosound"),
        ];
        let matches = matches_for(
            &declared,
            &["retroclawd", "--rom", "game.rom", "--turbosound"],
        );
        let empty = HashMap::new();
        let resolved = resolve_machine_options(
            "zx-spectrum-128",
            &declared,
            &matches,
            &empty,
            &empty,
            false,
        )
        .expect("resolution should succeed");
        assert_eq!(
            resolved.rom_paths,
            vec![("rom".to_string(), PathBuf::from("game.rom"))]
        );
        assert_eq!(
            resolved.values.get("turbosound"),
            Some(&DebugValue::Bool(true))
        );
    }

    #[test]
    fn prg_goes_to_the_plugin_when_the_profile_declares_program() {
        let kinds = [MediaKind::Disk, MediaKind::Program];
        assert!(program_goes_to_plugin("hello.prg", &kinds));
        assert!(program_goes_to_plugin("HELLO.PRG", &kinds));
        assert!(program_goes_to_plugin("a/b/tool.ttp", &kinds));
        assert!(program_goes_to_plugin("x.tos", &kinds));
        assert!(
            !program_goes_to_plugin("blob.bin", &kinds),
            "only GEMDOS programs"
        );
        assert!(!program_goes_to_plugin("noextension", &kinds));
    }

    #[test]
    fn prg_keeps_the_c64_path_when_it_does_not() {
        let kinds = [MediaKind::Disk, MediaKind::Tape];
        assert!(!program_goes_to_plugin("hello.prg", &kinds));
        assert!(!program_goes_to_plugin("hello.prg", &[]));
        // And the C64 path still reads the load address from the header.
        let (org, loaded) =
            resolve_program_load("game.prg", vec![0x01, 0x08, 0xAA], 0x4000).unwrap();
        assert_eq!((org, loaded), (0x0801, vec![0xAA]));
    }

    fn resolve_program_load_uses_org_verbatim_for_a_non_prg_extension() {
        let bytes = vec![0xEA, 0xEA, 0xEA];
        let (org, loaded) = resolve_program_load("test.bin", bytes.clone(), 0x4000).unwrap();
        assert_eq!(org, 0x4000);
        assert_eq!(loaded, bytes);
    }

    #[test]
    fn resolve_program_load_reads_the_real_two_byte_header_for_a_prg_file() {
        // Real .prg layout: load address low byte, high byte, then the
        // program bytes -- $0801 (the real, standard BASIC start address)
        // followed by 3 bytes of payload.
        let bytes = vec![0x01, 0x08, 0xAA, 0xBB, 0xCC];
        let (org, loaded) = resolve_program_load("game.prg", bytes, 0x4000).unwrap();
        assert_eq!(
            org, 0x0801,
            "--org must be ignored in favor of the real header"
        );
        assert_eq!(loaded, vec![0xAA, 0xBB, 0xCC]);
    }

    fn resolve_program_load_extension_check_is_case_insensitive() {
        let bytes = vec![0x00, 0xC0, 0x42];
        let (org, loaded) = resolve_program_load("GAME.PRG", bytes, 0x4000).unwrap();
        assert_eq!(org, 0xC000);
        assert_eq!(loaded, vec![0x42]);
    }

    fn resolve_program_load_allows_a_header_only_prg_with_zero_program_bytes() {
        let bytes = vec![0x00, 0x80];
        let (org, loaded) = resolve_program_load("empty.prg", bytes, 0x4000).unwrap();
        assert_eq!(org, 0x8000);
        assert!(loaded.is_empty());
    }
    /// Merged (every check kept): `resolve_program_load_uses_org_verbatim_for_a_non_prg_extension`, `resolve_program_load_extension_check_is_case_insensitive`, `resolve_program_load_allows_a_header_only_prg_with_zero_program_bytes`.
    #[test]
    fn resolve_program_load_accepts_edge_cases() {
        resolve_program_load_uses_org_verbatim_for_a_non_prg_extension();
        resolve_program_load_extension_check_is_case_insensitive();
        resolve_program_load_allows_a_header_only_prg_with_zero_program_bytes();
    }

    fn p00(prg: &[u8]) -> Vec<u8> {
        let mut file = b"C64File\0".to_vec();
        file.extend_from_slice(&[b"HELLO".as_slice(), &[0; 11]].concat()); // 16-byte name
        file.extend_from_slice(&[0, 0]); // zero byte + REL record size
        assert_eq!(file.len(), P00_HEADER_LEN);
        file.extend_from_slice(prg);
        file
    }

    #[test]
    fn resolve_program_load_strips_the_pc64_header_from_a_p00_file() {
        let file = p00(&[0x01, 0x08, 0xAA, 0xBB]);
        let (org, loaded) = resolve_program_load("game.p00", file.clone(), 0x4000).unwrap();
        assert_eq!((org, loaded), (0x0801, vec![0xAA, 0xBB]));
        // p01..p99 are the same container; the check ignores case.
        assert!(resolve_program_load("GAME.P07", file, 0x4000).is_ok());
    }

    fn resolve_program_load_rejects_a_p00_without_the_header_or_a_load_address() {
        let err = resolve_program_load("x.p00", vec![0x01, 0x08, 0xAA], 0x4000).unwrap_err();
        assert!(err.contains("PC64"), "{err}");
        let mut headerless = p00(&[]);
        headerless.truncate(P00_HEADER_LEN + 1);
        assert!(resolve_program_load("x.p00", headerless, 0x4000).is_err());
        // s00/u00/r00 are not programs and pass through untouched.
        let (org, bytes) = resolve_program_load("x.s00", vec![1, 2, 3], 0x4000).unwrap();
        assert_eq!((org, bytes), (0x4000, vec![1, 2, 3]));
    }

    fn resolve_program_load_rejects_a_prg_file_too_short_to_hold_a_load_address() {
        let err = resolve_program_load("bad.prg", vec![0x01], 0x4000).unwrap_err();
        assert!(err.contains("too short"));
    }
    /// Merged (every check kept): `resolve_program_load_rejects_a_p00_without_the_header_or_a_load_address`, `resolve_program_load_rejects_a_prg_file_too_short_to_hold_a_load_address`.
    #[test]
    fn resolve_program_load_rejects_short_files() {
        resolve_program_load_rejects_a_p00_without_the_header_or_a_load_address();
        resolve_program_load_rejects_a_prg_file_too_short_to_hold_a_load_address();
    }

    #[test]
    fn a_game_config_names_the_machine_before_clap_runs() {
        let dir = std::env::temp_dir().join(format!("retroclawd-prescan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("g.toml");
        std::fs::write(&file, "machine = \"c64\"\n").unwrap();
        let path = file.to_string_lossy().to_string();
        let argv = |v: &[&str]| -> Vec<OsString> { v.iter().map(OsString::from).collect() };
        assert_eq!(
            prescan_machine_arg(&argv(&["retroclawd", "--game-config", &path])),
            Some("c64".to_string())
        );
        assert_eq!(
            prescan_machine_arg(&argv(&["retroclawd", &format!("--game-config={path}")])),
            Some("c64".to_string())
        );
        // A positional machine still wins; a missing file is left to the real load.
        assert_eq!(
            prescan_machine_arg(&argv(&["retroclawd", "cpc464", "--game-config", &path])),
            Some("cpc464".to_string())
        );
        assert_eq!(
            prescan_machine_arg(&argv(&["retroclawd", "--game-config", "/nonexistent.toml"])),
            None
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
