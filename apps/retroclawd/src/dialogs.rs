//! File dialogs for media and saved state while the machine runs: load or
//! swap a tape or disk, eject, load/save snapshots and `.state` files, save
//! the cassette recording or the disk image.
//!
//! Each action is a hotkey (`load_tape`, `eject_disk`, ... in `[hotkeys]`).
//! It runs on its own short-lived thread: native dialogs (the `rfd` crate; on
//! Linux the XDG desktop portal) block, and neither the Bevy loop nor the
//! emulation thread may. While the thread works the machine is held still and
//! silent (`EmuCommand::Dialog`). Everything that touches the machine goes
//! over the emu thread's command channel, so the thread that owns it stays
//! the only one that does.
//!
//! The logic (`run`) is written against two small traits -- `Emu` (the
//! machine side) and `Ui` (dialogs and the in-app prompt) -- so the whole
//! sequence, including the "this disk has unsaved changes" question, is
//! unit-tested with fakes. `ChannelEmu` and `RfdUi` are the real ones.
//!
//! Choices made, all from the design agreed for this feature:
//! * dialogs open in the last folder used for that kind of file (tape, disk,
//!   snapshot, state), remembered in `retroclawd.state.toml` next to the
//!   config file -- the config itself is never rewritten; the first time, in
//!   the folder of the file the session was started with, else the working
//!   directory;
//! * the XDG portal has no message boxes, so the unsaved-disk question is an
//!   in-app prompt (the GUI shows it and reads the answer key), not a dialog;
//! * ejecting or swapping a disk with unsaved writes offers Save As /
//!   overwrite the original file / discard / cancel -- never a silent loss;
//!   a swapped-in disk gets the same exit policy as the command-line one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::Resource;
use plugin_api::MediaKind;
use serde::{Deserialize, Serialize};

use crate::emu_thread::{DiskStatus, EmuCommand, Export, MediaOp, Messages, post_message};

/// One of the file hotkeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileAction {
    LoadTape,
    LoadDisk,
    EjectTape,
    EjectDisk,
    LoadSnapshot,
    SaveSnapshot,
    LoadState,
    SaveState,
    SaveTape,
    SaveDisk,
    /// The same three for the second drive (drive B of a CPC664/6128 or +3).
    LoadDiskB,
    EjectDiskB,
    SaveDiskB,
    SaveHardDisk,
    /// Swap to the next / previous disk of an `--disk list.m3u` playlist.
    NextDisk,
    PrevDisk,
}

/// What kind of file a dialog is about; each remembers its own folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileKind {
    Tape,
    Disk,
    Snapshot,
    State,
}

impl FileKind {
    fn key(self) -> &'static str {
        match self {
            FileKind::Tape => "tape",
            FileKind::Disk => "disk",
            FileKind::Snapshot => "snapshot",
            FileKind::State => "state",
        }
    }

    fn media_kind(self) -> Option<MediaKind> {
        match self {
            FileKind::Tape => Some(MediaKind::Tape),
            FileKind::Disk => Some(MediaKind::Disk),
            FileKind::Snapshot => Some(MediaKind::Snapshot),
            FileKind::State => None,
        }
    }

    fn noun(self) -> &'static str {
        match self {
            FileKind::Tape => "a tape",
            FileKind::Disk => "a disk",
            FileKind::Snapshot => "snapshots",
            FileKind::State => "saved states",
        }
    }
}

impl FileAction {
    fn kind(self) -> FileKind {
        match self {
            FileAction::LoadTape | FileAction::EjectTape | FileAction::SaveTape => FileKind::Tape,
            FileAction::LoadDisk
            | FileAction::EjectDisk
            | FileAction::SaveDisk
            | FileAction::LoadDiskB
            | FileAction::EjectDiskB
            | FileAction::SaveDiskB
            | FileAction::SaveHardDisk
            | FileAction::NextDisk
            | FileAction::PrevDisk => FileKind::Disk,
            FileAction::LoadSnapshot | FileAction::SaveSnapshot => FileKind::Snapshot,
            FileAction::LoadState | FileAction::SaveState => FileKind::State,
        }
    }
}

/// A dialog file-type filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Filter {
    pub(crate) name: String,
    pub(crate) extensions: Vec<String>,
}

/// File extensions worth showing for `kind` on this machine. A hint for the
/// dialog only: the machine plugin still decides what it accepts.
pub(crate) fn extensions_for(kind: FileKind, machine_id: &str) -> &'static [&'static str] {
    let spectrum = machine_id.starts_with("zx-spectrum");
    let cpc = machine_id.starts_with("cpc") || machine_id == "gx4000";
    let atari8 = matches!(
        machine_id,
        "atari-800" | "atari-800xl" | "atari-65xe" | "atari-130xe"
    );
    match kind {
        FileKind::Tape if atari8 => &["cas"],
        FileKind::Tape if spectrum => &["tap", "tzx", "pzx", "csw", "voc", "wav"],
        FileKind::Tape if cpc => &["cdt", "tzx", "voc", "wav"],
        FileKind::Tape if machine_id == "c64" => &["tap", "t64"],
        FileKind::Tape if machine_id == "zx81" => {
            &["p", "p81", "81", "tzx", "pzx", "csw", "voc", "wav"]
        }
        FileKind::Disk if spectrum || cpc => &["dsk", "edsk"],
        FileKind::Disk if atari8 => &["atr", "xfd"],
        FileKind::Disk if machine_id == "c64" || machine_id == "c128" => {
            &["d64", "d71", "d81", "g64", "x64"]
        }
        FileKind::Disk if machine_id.starts_with("atari-") => &["st", "msa"],
        FileKind::Snapshot if spectrum => &["z80", "sna", "szx"],
        FileKind::Snapshot if machine_id == "zx81" => &["p", "p81", "81"],
        FileKind::Snapshot if cpc => &["sna"],
        FileKind::State => &["state"],
        _ => &[],
    }
}

/// Which kind of file a path is, by extension (for the starting folder of
/// the media the session was launched with).
pub(crate) fn kind_of_extension(path: &str) -> Option<FileKind> {
    let extension = Path::new(path)
        .extension()?
        .to_string_lossy()
        .to_lowercase();
    match extension.as_str() {
        "tap" | "tzx" | "cdt" | "pzx" | "csw" | "voc" | "wav" | "t64" | "cas" => {
            Some(FileKind::Tape)
        }
        "dsk" | "edsk" | "d64" | "d71" | "d81" | "g64" | "x64" | "nib" | "atr" | "xfd" | "st"
        | "msa" => Some(FileKind::Disk),
        "z80" | "sna" | "szx" | "p" | "p81" | "81" => Some(FileKind::Snapshot),
        "state" => Some(FileKind::State),
        _ => None,
    }
}

fn filters_for(kind: FileKind, machine_id: &str) -> Vec<Filter> {
    let extensions = extensions_for(kind, machine_id);
    if extensions.is_empty() {
        return Vec::new();
    }
    vec![Filter {
        name: "Supported files".to_string(),
        extensions: extensions.iter().map(|e| e.to_string()).collect(),
    }]
}

/// The extension a "save" dialog proposes (the user may type another).
fn default_save_extension(action: FileAction, machine_id: &str) -> &'static str {
    match action {
        FileAction::SaveState => "state",
        FileAction::SaveTape if machine_id == "zx81" => "p",
        FileAction::SaveTape => "cdt",
        FileAction::SaveDisk | FileAction::SaveDiskB => "dsk",
        FileAction::SaveHardDisk => "img",
        _ => extensions_for(FileKind::Snapshot, machine_id)
            .first()
            .copied()
            .unwrap_or("sna"),
    }
}

/// Folders last used per kind of file, kept in a small TOML file next to
/// `retroclawd.toml`. A missing or unreadable file is just "nothing
/// remembered yet".
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct FolderMemory {
    #[serde(default)]
    folders: BTreeMap<String, String>,
}

impl FolderMemory {
    pub(crate) fn load(path: &Path) -> FolderMemory {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The remembered folder, if it still exists.
    pub(crate) fn get(&self, kind: FileKind) -> Option<PathBuf> {
        let dir = PathBuf::from(self.folders.get(kind.key())?);
        dir.is_dir().then_some(dir)
    }

    /// Remembers the folder `file` lives in.
    pub(crate) fn remember(&mut self, kind: FileKind, file: &Path) {
        if let Some(dir) = folder_of(file) {
            self.folders
                .insert(kind.key().to_string(), dir.to_string_lossy().into_owned());
        }
    }

    /// Like `remember`, but only when nothing is stored yet (the folder of a
    /// file the session was started with).
    pub(crate) fn seed(&mut self, kind: FileKind, file: &Path) {
        if !self.folders.contains_key(kind.key()) {
            self.remember(kind, file);
        }
    }
}

fn folder_of(file: &Path) -> Option<PathBuf> {
    let parent = file.parent()?;
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    // Absolute, so the memory means the same folder next time whatever the
    // working directory is then.
    std::fs::canonicalize(parent).ok()
}

/// One answer a prompt accepts: the key to press and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptOption {
    /// Upper-case letter.
    pub(crate) key: char,
    pub(crate) label: String,
}

/// A question for the player, shown by the GUI over the picture.
pub(crate) struct PromptRequest {
    pub(crate) text: String,
    pub(crate) options: Vec<PromptOption>,
    /// The index of the chosen option, `None` for Escape.
    pub(crate) reply: Sender<Option<usize>>,
}

/// The machine side of a dialog action.
pub(crate) trait Emu {
    fn dialog_open(&self, open: bool);
    fn disk_status(&self) -> Result<DiskStatus, String>;
    fn export(&self, what: Export) -> Result<Option<Vec<u8>>, String>;
    fn apply(&self, op: MediaOp);
    fn say(&self, message: String);
}

/// The player side: native dialogs and the in-app question.
pub(crate) trait Ui {
    fn pick_file(&self, title: &str, filters: &[Filter], dir: Option<&Path>) -> Option<PathBuf>;
    fn save_file(
        &self,
        title: &str,
        filters: &[Filter],
        dir: Option<&Path>,
        file_name: &str,
    ) -> Option<PathBuf>;
    fn choose(&self, text: &str, options: &[PromptOption]) -> Option<usize>;
}

/// Everything `run` needs besides the two ports.
pub(crate) struct Session<'a> {
    pub(crate) machine_id: &'a str,
    pub(crate) media_kinds: &'a [MediaKind],
    pub(crate) emu: &'a dyn Emu,
    pub(crate) ui: &'a dyn Ui,
    pub(crate) folders: &'a mut FolderMemory,
    /// Used for the proposed folder of saved states (`states/`).
    pub(crate) work_dir: &'a Path,
    pub(crate) unix_secs: u64,
    /// The disks of an `.m3u` playlist (empty without one) and the index of
    /// the one in the drive.
    pub(crate) playlist: &'a [PathBuf],
    pub(crate) playlist_index: &'a mut usize,
}

/// The outcome of the sequence: a message to show, `None` for a cancel.
type Done = Result<Option<String>, String>;

/// Carries out one file action from start to finish, reporting through
/// `Emu::say`. The machine is held still for its whole duration.
pub(crate) fn run(action: FileAction, session: &mut Session) {
    if let Err(message) = session.check_supported(action) {
        session.emu.say(message);
        return;
    }
    tracing::info!(target: "retroclawd::dialogs", "file action: {action:?}");
    session.emu.dialog_open(true);
    let result = session.perform(action);
    session.emu.dialog_open(false);
    tracing::info!(target: "retroclawd::dialogs", "file action finished: {action:?}");
    match result {
        Ok(Some(message)) => session.emu.say(message),
        Ok(None) => {}
        Err(message) => session.emu.say(message),
    }
}

impl Session<'_> {
    fn check_supported(&self, action: FileAction) -> Result<(), String> {
        let kind = action.kind();
        match kind.media_kind() {
            Some(media) if !self.media_kinds.contains(&media) => Err(format!(
                "{} has no support for {}",
                self.machine_id,
                kind.noun()
            )),
            _ => Ok(()),
        }
    }

    fn start_dir(&self, kind: FileKind) -> Option<PathBuf> {
        self.folders.get(kind)
    }

    fn pick(&mut self, kind: FileKind, title: &str) -> Option<PathBuf> {
        let dir = self.start_dir(kind);
        let filters = filters_for(kind, self.machine_id);
        let path = self.ui.pick_file(title, &filters, dir.as_deref())?;
        self.folders.remember(kind, &path);
        Some(path)
    }

    fn pick_save(
        &mut self,
        kind: FileKind,
        title: &str,
        file_name: &str,
        fallback_dir: Option<PathBuf>,
    ) -> Option<PathBuf> {
        let dir = self.start_dir(kind).or(fallback_dir);
        let filters = filters_for(kind, self.machine_id);
        let path = self
            .ui
            .save_file(title, &filters, dir.as_deref(), file_name)?;
        self.folders.remember(kind, &path);
        Some(path)
    }

    fn timestamped(&self, extension: &str) -> String {
        crate::host_files::timestamped_name(self.machine_id, self.unix_secs, extension)
    }

    fn perform(&mut self, action: FileAction) -> Done {
        match action {
            FileAction::LoadTape => {
                let Some(path) = self.pick(FileKind::Tape, "Load tape") else {
                    return Ok(None);
                };
                let bytes = read_file(&path)?;
                self.emu.apply(MediaOp::AttachTape {
                    extension: input_extension_of(&path),
                    bytes,
                });
                Ok(None)
            }
            FileAction::LoadDisk => {
                if !self.settle_disk("swapping")? {
                    return Ok(None);
                }
                let Some(path) = self.pick(FileKind::Disk, "Load disk") else {
                    return Ok(None);
                };
                let bytes = read_file(&path)?;
                self.emu.apply(MediaOp::AttachDisk {
                    extension: input_extension_of(&path),
                    bytes,
                    path,
                });
                Ok(None)
            }
            FileAction::NextDisk | FileAction::PrevDisk => {
                if self.playlist.len() < 2 {
                    return Err("No disk playlist: start with --disk list.m3u".to_string());
                }
                if !self.settle_disk("swapping")? {
                    return Ok(None);
                }
                let next = crate::playlist::step_index(
                    *self.playlist_index,
                    self.playlist.len(),
                    action == FileAction::NextDisk,
                );
                let path = self.playlist[next].clone();
                let bytes = read_file(&path)?;
                *self.playlist_index = next;
                self.emu.apply(MediaOp::AttachDisk {
                    extension: input_extension_of(&path),
                    bytes,
                    path,
                });
                Ok(None)
            }
            FileAction::LoadDiskB => {
                let Some(path) = self.pick(FileKind::Disk, "Load disk into drive B") else {
                    return Ok(None);
                };
                let bytes = read_file(&path)?;
                self.emu.apply(MediaOp::AttachDiskB {
                    extension: input_extension_of(&path),
                    bytes,
                    path,
                });
                Ok(None)
            }
            FileAction::EjectDiskB => {
                self.emu.apply(MediaOp::EjectDiskB);
                Ok(None)
            }
            FileAction::SaveDiskB => {
                let name = self.timestamped("dsk");
                let Some(path) =
                    self.pick_save(FileKind::Disk, "Save drive B disk image", &name, None)
                else {
                    return Ok(None);
                };
                let Some(bytes) = self.emu.export(Export::DiskB)? else {
                    return Ok(Some("There is no disk in drive B".to_string()));
                };
                write_file(&path, &bytes)?;
                self.emu.apply(MediaOp::DiskBSaved);
                Ok(Some(format!("Drive B disk image saved: {}", label(&path))))
            }
            FileAction::SaveHardDisk => {
                let name = self.timestamped("img");
                let Some(path) =
                    self.pick_save(FileKind::Disk, "Save ACSI hard disk image", &name, None)
                else {
                    return Ok(None);
                };
                let Some(bytes) = self.emu.export(Export::Acsi(0))? else {
                    return Ok(Some("There is no ACSI hard disk 0".to_string()));
                };
                write_file(&path, &bytes)?;
                self.emu.apply(MediaOp::AcsiSaved(0));
                Ok(Some(format!("Hard disk image saved: {}", label(&path))))
            }
            FileAction::EjectTape => {
                self.emu.apply(MediaOp::EjectTape);
                Ok(None)
            }
            FileAction::EjectDisk => {
                if self.settle_disk("ejecting")? {
                    self.emu.apply(MediaOp::EjectDisk);
                }
                Ok(None)
            }
            FileAction::LoadSnapshot => {
                let Some(path) = self.pick(FileKind::Snapshot, "Load snapshot") else {
                    return Ok(None);
                };
                let bytes = read_file(&path)?;
                self.emu.apply(MediaOp::LoadSnapshot {
                    extension: input_extension_of(&path),
                    bytes,
                });
                Ok(None)
            }
            FileAction::LoadState => {
                let Some(path) = self.pick(FileKind::State, "Load saved state") else {
                    return Ok(None);
                };
                let bytes = read_file(&path)?;
                self.emu.apply(MediaOp::LoadState { bytes });
                Ok(None)
            }
            FileAction::SaveSnapshot => {
                let name = self.timestamped(default_save_extension(action, self.machine_id));
                let Some(path) = self.pick_save(FileKind::Snapshot, "Save snapshot", &name, None)
                else {
                    return Ok(None);
                };
                let extension = extension_of(&path);
                let bytes = self
                    .emu
                    .export(Export::Snapshot { extension })?
                    .ok_or("Nothing to save")?;
                write_file(&path, &bytes)?;
                Ok(Some(format!("Snapshot saved: {}", label(&path))))
            }
            FileAction::SaveState => {
                let name = self.timestamped("state");
                let fallback = Some(self.work_dir.join("states")).filter(|d| d.is_dir());
                let Some(path) = self.pick_save(FileKind::State, "Save state", &name, fallback)
                else {
                    return Ok(None);
                };
                let bytes = self.emu.export(Export::State)?.ok_or("Nothing to save")?;
                write_file(&path, &bytes)?;
                Ok(Some(format!("State saved: {}", label(&path))))
            }
            FileAction::SaveTape => {
                let name = self.timestamped(default_save_extension(action, self.machine_id));
                let Some(path) =
                    self.pick_save(FileKind::Tape, "Save cassette recording", &name, None)
                else {
                    return Ok(None);
                };
                let format = extension_of(&path);
                if !matches!(format.as_str(), "cdt" | "tzx" | "voc") {
                    return Err(format!(
                        "Cannot save a recording as .{format} (use .cdt, .tzx or .voc)"
                    ));
                }
                let Some(bytes) = self.emu.export(Export::Tape { format })? else {
                    return Ok(Some(
                        "Nothing has been recorded to the cassette".to_string(),
                    ));
                };
                write_file(&path, &bytes)?;
                Ok(Some(format!("Recording saved: {}", label(&path))))
            }
            FileAction::SaveDisk => {
                let name = self.disk_save_name();
                let Some(path) = self.pick_save(FileKind::Disk, "Save disk image", &name, None)
                else {
                    return Ok(None);
                };
                let Some(bytes) = self.emu.export(Export::Disk)? else {
                    return Ok(Some("There is no disk in the drive".to_string()));
                };
                // A copy: the disk's own unsaved-changes state is untouched,
                // so the exit policy (--disk-save / --disk-writeback / the
                // warning) still covers the disk itself.
                write_file(&path, &bytes)?;
                Ok(Some(format!("Disk image saved: {}", label(&path))))
            }
        }
    }

    fn disk_save_name(&self) -> String {
        let status = self.emu.disk_status().ok();
        status
            .and_then(|s| s.original)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| self.timestamped("dsk"))
    }

    /// Before a disk leaves the drive: if the program wrote to it, asks what
    /// to do. `Ok(true)`: go ahead (nothing to lose, saved, or discarded);
    /// `Ok(false)`: the player cancelled.
    fn settle_disk(&mut self, doing: &str) -> Result<bool, String> {
        let status = self.emu.disk_status()?;
        if !status.modified {
            return Ok(true);
        }
        let mut options = vec![PromptOption {
            key: 'S',
            label: "save as".to_string(),
        }];
        if let Some(original) = &status.original {
            options.push(PromptOption {
                key: 'O',
                label: format!("overwrite {}", label(original)),
            });
        }
        options.push(PromptOption {
            key: 'D',
            label: "discard changes".to_string(),
        });
        let text = format!(
            "The disk has unsaved changes -- before {doing} it:  {}  [Esc] cancel",
            options
                .iter()
                .map(|o| format!("[{}] {}", o.key, o.label))
                .collect::<Vec<_>>()
                .join("  ")
        );
        let Some(chosen) = self.ui.choose(&text, &options) else {
            return Ok(false);
        };
        match options[chosen].key {
            'S' => {
                let name = status
                    .original
                    .as_deref()
                    .and_then(Path::file_name)
                    .map_or_else(
                        || self.timestamped("dsk"),
                        |n| n.to_string_lossy().into_owned(),
                    );
                let Some(path) = self.pick_save(FileKind::Disk, "Save disk image", &name, None)
                else {
                    return Ok(false);
                };
                self.write_disk(&path)?;
                Ok(true)
            }
            'O' => {
                let original = status.original.ok_or("The original file is unknown")?;
                self.write_disk(&original)?;
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    /// Writes the disk's current image to `path` and marks it saved.
    fn write_disk(&mut self, path: &Path) -> Result<(), String> {
        let bytes = self
            .emu
            .export(Export::Disk)?
            .ok_or("There is no disk in the drive")?;
        write_file(path, &bytes)?;
        self.emu.apply(MediaOp::DiskSaved);
        self.emu.say(format!("Disk image saved: {}", label(path)));
        Ok(())
    }
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn label(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Reads a file the user picked; a `.zip` holding one file is unpacked.
fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    crate::inputs::load(&path.to_string_lossy())
        .map(|loaded| loaded.bytes.clone())
        .map_err(|e| format!("Could not read {}: {e}", label(path)))
}

/// The extension of what a picked file really is (the entry inside a zip).
fn input_extension_of(path: &Path) -> String {
    crate::inputs::extension(&path.to_string_lossy())
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    crate::plugin_bridge::write_file_atomically(path, bytes)
        .map_err(|e| format!("Could not write {}: {e}", path.display()))
}

// ---- the real ports -------------------------------------------------------

/// How long the dialog thread waits for the emu thread to answer.
const EMU_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

struct ChannelEmu {
    commands: Sender<EmuCommand>,
    messages: Messages,
}

impl ChannelEmu {
    fn ask<T>(&self, command: impl FnOnce(Sender<T>) -> EmuCommand) -> Result<T, String> {
        let (tx, rx) = mpsc::channel();
        self.commands
            .send(command(tx))
            .map_err(|_| "The emulator has stopped".to_string())?;
        rx.recv_timeout(EMU_REPLY_TIMEOUT)
            .map_err(|_| "The emulator did not answer".to_string())
    }
}

impl Emu for ChannelEmu {
    fn dialog_open(&self, open: bool) {
        let _ = self.commands.send(EmuCommand::Dialog(open));
    }

    fn disk_status(&self) -> Result<DiskStatus, String> {
        self.ask(EmuCommand::DiskStatus)
    }

    fn export(&self, what: Export) -> Result<Option<Vec<u8>>, String> {
        self.ask(|tx| EmuCommand::Export(what, tx))?
    }

    fn apply(&self, op: MediaOp) {
        let _ = self.commands.send(EmuCommand::Media(op));
    }

    fn say(&self, message: String) {
        post_message(&self.messages, message);
    }
}

struct RfdUi {
    prompts: Sender<PromptRequest>,
}

fn dialog(title: &str, filters: &[Filter], dir: Option<&Path>) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    for filter in filters {
        dialog = dialog.add_filter(&filter.name, &filter.extensions);
    }
    if let Some(dir) = dir {
        dialog = dialog.set_directory(dir);
    }
    dialog
}

impl Ui for RfdUi {
    fn pick_file(&self, title: &str, filters: &[Filter], dir: Option<&Path>) -> Option<PathBuf> {
        dialog(title, filters, dir).pick_file()
    }

    fn save_file(
        &self,
        title: &str,
        filters: &[Filter],
        dir: Option<&Path>,
        file_name: &str,
    ) -> Option<PathBuf> {
        dialog(title, filters, dir)
            .set_file_name(file_name)
            .save_file()
    }

    fn choose(&self, text: &str, options: &[PromptOption]) -> Option<usize> {
        let (reply, answer) = mpsc::channel();
        self.prompts
            .send(PromptRequest {
                text: text.to_string(),
                options: options.to_vec(),
                reply,
            })
            .ok()?;
        answer.recv().ok().flatten()
    }
}

/// Where the dialogs get their context, fixed at startup.
#[derive(Debug, Clone, Default)]
pub(crate) struct DialogSettings {
    pub(crate) machine_id: String,
    pub(crate) media_kinds: Vec<MediaKind>,
    /// `retroclawd.state.toml`, next to the config file.
    pub(crate) state_path: PathBuf,
    /// Folders of the files the session was started with.
    pub(crate) seeds: Vec<(FileKind, PathBuf)>,
    pub(crate) work_dir: PathBuf,
    /// Disks of an `--disk list.m3u` playlist, in order (empty otherwise).
    pub(crate) playlist: Vec<PathBuf>,
}

/// Handle the GUI keeps: starts file actions, says whether one is running.
#[derive(Clone, Resource)]
pub(crate) struct FileDialogs {
    busy: Arc<AtomicBool>,
    settings: Arc<DialogSettings>,
    commands: Sender<EmuCommand>,
    messages: Messages,
    prompts: Sender<PromptRequest>,
    /// Serialises access to the state file between dialog threads.
    memory_lock: Arc<Mutex<()>>,
    /// Index of the playlist disk in the drive.
    playlist_index: Arc<Mutex<usize>>,
}

impl FileDialogs {
    pub(crate) fn new(
        settings: DialogSettings,
        commands: Sender<EmuCommand>,
        messages: Messages,
        prompts: Sender<PromptRequest>,
    ) -> Self {
        FileDialogs {
            busy: Arc::new(AtomicBool::new(false)),
            settings: Arc::new(settings),
            commands,
            messages,
            prompts,
            memory_lock: Arc::new(Mutex::new(())),
            playlist_index: Arc::new(Mutex::new(0)),
        }
    }

    /// A dialog or its prompt is open: the GUI keeps keys away from the
    /// machine and from the hotkeys meanwhile.
    pub(crate) fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    /// Starts `action` on its own thread. Returns `false` (and says why) when
    /// another dialog is still open.
    pub(crate) fn start(&self, action: FileAction) -> bool {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            post_message(&self.messages, "A file dialog is already open");
            return false;
        }
        let this = self.clone();
        let spawned = std::thread::Builder::new()
            .name("file-dialog".into())
            .spawn(move || {
                this.run_blocking(action);
                this.busy.store(false, Ordering::Release);
            });
        if spawned.is_err() {
            self.busy.store(false, Ordering::Release);
            post_message(&self.messages, "Could not open a file dialog");
            return false;
        }
        true
    }

    fn run_blocking(&self, action: FileAction) {
        let settings = &self.settings;
        let _guard = self.memory_lock.lock().unwrap();
        let mut folders = FolderMemory::load(&settings.state_path);
        for (kind, file) in &settings.seeds {
            folders.seed(*kind, file);
        }
        let emu = ChannelEmu {
            commands: self.commands.clone(),
            messages: self.messages.clone(),
        };
        let ui = RfdUi {
            prompts: self.prompts.clone(),
        };
        let unix_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        {
            let mut playlist_index = self.playlist_index.lock().unwrap();
            let mut session = Session {
                machine_id: &settings.machine_id,
                media_kinds: &settings.media_kinds,
                emu: &emu,
                ui: &ui,
                folders: &mut folders,
                work_dir: &settings.work_dir,
                unix_secs,
                playlist: &settings.playlist,
                playlist_index: &mut playlist_index,
            };
            run(action, &mut session);
        }
        if let Err(e) = folders.save(&settings.state_path) {
            tracing::warn!("could not remember the dialog folders: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    #[derive(Debug, PartialEq)]
    enum Event {
        Dialog(bool),
        Status,
        Export(Export),
        Apply(String),
        Say(String),
        Pick(String),
        Save(String),
        Choose(String),
    }

    struct FakeEmu<'a> {
        log: &'a RefCell<Vec<Event>>,
        status: RefCell<DiskStatus>,
        exports: RefCell<VecDeque<Result<Option<Vec<u8>>, String>>>,
    }

    impl Emu for FakeEmu<'_> {
        fn dialog_open(&self, open: bool) {
            self.log.borrow_mut().push(Event::Dialog(open));
        }
        fn disk_status(&self) -> Result<DiskStatus, String> {
            self.log.borrow_mut().push(Event::Status);
            Ok(self.status.borrow().clone())
        }
        fn export(&self, what: Export) -> Result<Option<Vec<u8>>, String> {
            self.log.borrow_mut().push(Event::Export(what));
            self.exports.borrow_mut().pop_front().unwrap_or(Ok(None))
        }
        fn apply(&self, op: MediaOp) {
            let text = match op {
                MediaOp::AttachTape { extension, bytes } => {
                    format!("tape .{extension} {} bytes", bytes.len())
                }
                MediaOp::AttachDisk {
                    extension,
                    bytes,
                    path,
                } => format!(
                    "disk .{extension} {} bytes from {}",
                    bytes.len(),
                    label(&path)
                ),
                MediaOp::EjectTape => "eject tape".to_string(),
                MediaOp::EjectDisk => "eject disk".to_string(),
                MediaOp::LoadSnapshot { extension, bytes } => {
                    format!("snapshot .{extension} {} bytes", bytes.len())
                }
                MediaOp::LoadState { bytes } => format!("state {} bytes", bytes.len()),
                MediaOp::DiskSaved => "disk saved".to_string(),
                MediaOp::AttachDiskB { path, .. } => format!("attach B {}", path.display()),
                MediaOp::EjectDiskB => "eject B".to_string(),
                MediaOp::DiskBSaved => "disk B saved".to_string(),
                MediaOp::AcsiSaved(target) => format!("ACSI {target} saved"),
            };
            self.log.borrow_mut().push(Event::Apply(text));
        }
        fn say(&self, message: String) {
            self.log.borrow_mut().push(Event::Say(message));
        }
    }

    struct FakeUi<'a> {
        log: &'a RefCell<Vec<Event>>,
        picks: RefCell<VecDeque<Option<PathBuf>>>,
        saves: RefCell<VecDeque<Option<PathBuf>>>,
        answers: RefCell<VecDeque<Option<usize>>>,
        dirs: RefCell<Vec<Option<PathBuf>>>,
        save_names: RefCell<Vec<String>>,
    }

    impl Ui for FakeUi<'_> {
        fn pick_file(&self, title: &str, _: &[Filter], dir: Option<&Path>) -> Option<PathBuf> {
            self.log.borrow_mut().push(Event::Pick(title.to_string()));
            self.dirs.borrow_mut().push(dir.map(Path::to_path_buf));
            self.picks.borrow_mut().pop_front().flatten()
        }
        fn save_file(
            &self,
            title: &str,
            _: &[Filter],
            dir: Option<&Path>,
            file_name: &str,
        ) -> Option<PathBuf> {
            self.log.borrow_mut().push(Event::Save(title.to_string()));
            self.dirs.borrow_mut().push(dir.map(Path::to_path_buf));
            self.save_names.borrow_mut().push(file_name.to_string());
            self.saves.borrow_mut().pop_front().flatten()
        }
        fn choose(&self, text: &str, options: &[PromptOption]) -> Option<usize> {
            self.log.borrow_mut().push(Event::Choose(format!(
                "{} options: {}",
                options.len(),
                text
            )));
            self.answers.borrow_mut().pop_front().flatten()
        }
    }

    struct Rig {
        dir: PathBuf,
        log: RefCell<Vec<Event>>,
        playlist: Vec<PathBuf>,
        playlist_index: RefCell<usize>,
    }

    impl Rig {
        fn new(name: &str) -> Rig {
            let dir = std::env::temp_dir()
                .join(format!("retroclawd-dialogs-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Rig {
                dir,
                log: RefCell::new(Vec::new()),
                playlist: Vec::new(),
                playlist_index: RefCell::new(0),
            }
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }

        // A test rig: one argument per scripted answer reads clearer than a struct.
        #[allow(clippy::too_many_arguments)]
        fn run(
            &self,
            action: FileAction,
            machine: &str,
            kinds: &[MediaKind],
            status: DiskStatus,
            picks: Vec<Option<PathBuf>>,
            saves: Vec<Option<PathBuf>>,
            answers: Vec<Option<usize>>,
            exports: Vec<Result<Option<Vec<u8>>, String>>,
        ) -> (FolderMemory, Vec<Option<PathBuf>>, Vec<String>) {
            let emu = FakeEmu {
                log: &self.log,
                status: RefCell::new(status),
                exports: RefCell::new(exports.into()),
            };
            let ui = FakeUi {
                log: &self.log,
                picks: RefCell::new(picks.into()),
                saves: RefCell::new(saves.into()),
                answers: RefCell::new(answers.into()),
                dirs: RefCell::new(Vec::new()),
                save_names: RefCell::new(Vec::new()),
            };
            let mut folders = FolderMemory::default();
            let mut session = Session {
                machine_id: machine,
                media_kinds: kinds,
                emu: &emu,
                ui: &ui,
                folders: &mut folders,
                work_dir: &self.dir,
                unix_secs: 1_700_000_000,
                playlist: &self.playlist,
                playlist_index: &mut self.playlist_index.borrow_mut(),
            };
            run(action, &mut session);
            let dirs = ui.dirs.borrow().clone();
            let names = ui.save_names.borrow().clone();
            (folders, dirs, names)
        }

        fn events(&self) -> Vec<Event> {
            std::mem::take(&mut *self.log.borrow_mut())
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn clean() -> DiskStatus {
        DiskStatus {
            modified: false,
            original: None,
        }
    }

    fn dirty(original: Option<PathBuf>) -> DiskStatus {
        DiskStatus {
            modified: true,
            original,
        }
    }

    const ALL_MEDIA: &[MediaKind] = &[MediaKind::Tape, MediaKind::Disk, MediaKind::Snapshot];

    #[test]
    fn st_images_are_disks_for_the_atari() {
        assert_eq!(extensions_for(FileKind::Disk, "atari-st"), &["st", "msa"]);
        assert_eq!(kind_of_extension("game.st"), Some(FileKind::Disk));
        assert_eq!(kind_of_extension("GAME.ST"), Some(FileKind::Disk));
    }

    #[test]
    fn loading_a_tape_attaches_it_and_remembers_the_folder() {
        let rig = Rig::new("tape");
        let tape = rig.file("game.TZX", b"tape-bytes");
        let (folders, dirs, _) = rig.run(
            FileAction::LoadTape,
            "cpc464",
            ALL_MEDIA,
            clean(),
            vec![Some(tape)],
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(
            rig.events(),
            [
                Event::Dialog(true),
                Event::Pick("Load tape".to_string()),
                Event::Apply("tape .tzx 10 bytes".to_string()),
                Event::Dialog(false),
            ],
            "the machine is held still around the dialog, the extension is lower-cased"
        );
        assert_eq!(
            dirs,
            [None],
            "nothing remembered yet: the dialog picks its own folder"
        );
        assert_eq!(
            folders.get(FileKind::Tape),
            Some(std::fs::canonicalize(&rig.dir).unwrap())
        );
        assert_eq!(folders.get(FileKind::Disk), None);
    }

    #[test]
    fn cancelling_a_dialog_does_nothing_and_says_nothing() {
        let rig = Rig::new("cancel");
        let (folders, _, _) = rig.run(
            FileAction::LoadTape,
            "cpc464",
            ALL_MEDIA,
            clean(),
            vec![None],
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(
            rig.events(),
            [
                Event::Dialog(true),
                Event::Pick("Load tape".to_string()),
                Event::Dialog(false)
            ]
        );
        assert_eq!(folders, FolderMemory::default());
    }

    #[test]
    fn a_machine_without_the_media_is_told_so_without_opening_anything() {
        let rig = Rig::new("unsupported");
        rig.run(
            FileAction::LoadDisk,
            "zx81",
            &[MediaKind::Snapshot],
            clean(),
            vec![],
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(
            rig.events(),
            [Event::Say("zx81 has no support for a disk".to_string())]
        );
    }

    #[test]
    fn swapping_a_modified_disk_saves_it_as_chosen_then_attaches_the_new_one() {
        let rig = Rig::new("swap-save");
        let original = rig.file("work.dsk", b"old");
        let new_disk = rig.file("next.dsk", b"next-disk");
        let target = rig.dir.join("copy.dsk");
        rig.run(
            FileAction::LoadDisk,
            "cpc664",
            ALL_MEDIA,
            dirty(Some(original)),
            vec![Some(new_disk.clone())],
            vec![Some(target.clone())],
            vec![Some(0)], // S = save as
            vec![Ok(Some(b"modified-image".to_vec()))],
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"modified-image");
        let events = rig.events();
        assert!(
            matches!(&events[2], Event::Choose(t) if t.starts_with("3 options:") && t.contains("overwrite work.dsk"))
        );
        assert_eq!(events[3], Event::Save("Save disk image".to_string()));
        assert_eq!(events[4], Event::Export(Export::Disk));
        assert_eq!(events[5], Event::Apply("disk saved".to_string()));
        assert_eq!(
            events[6],
            Event::Say("Disk image saved: copy.dsk".to_string())
        );
        assert_eq!(events[7], Event::Pick("Load disk".to_string()));
        assert_eq!(
            events[8],
            Event::Apply("disk .dsk 9 bytes from next.dsk".to_string())
        );
        assert_eq!(events.last(), Some(&Event::Dialog(false)));
    }

    #[test]
    fn overwriting_the_original_writes_that_file_and_keeps_the_swap_going() {
        let rig = Rig::new("swap-overwrite");
        let original = rig.file("work.dsk", b"old");
        let new_disk = rig.file("next.dsk", b"n");
        rig.run(
            FileAction::LoadDisk,
            "cpc664",
            ALL_MEDIA,
            dirty(Some(original.clone())),
            vec![Some(new_disk)],
            vec![],
            vec![Some(1)], // O = overwrite
            vec![Ok(Some(b"modified-image".to_vec()))],
        );
        assert_eq!(std::fs::read(&original).unwrap(), b"modified-image");
        let events = rig.events();
        assert!(events.contains(&Event::Apply("disk saved".to_string())));
        assert!(events.contains(&Event::Apply("disk .dsk 1 bytes from next.dsk".to_string())));
    }

    #[test]
    fn escape_at_the_question_keeps_the_disk_and_the_changes() {
        let rig = Rig::new("swap-cancel");
        rig.run(
            FileAction::LoadDisk,
            "cpc664",
            ALL_MEDIA,
            dirty(None),
            vec![],
            vec![],
            vec![None], // Esc
            vec![],
        );
        let events = rig.events();
        assert!(
            matches!(&events[2], Event::Choose(t) if t.starts_with("2 options:")),
            "no original file known: only Save as and Discard are offered"
        );
        assert_eq!(
            events.len(),
            4,
            "dialog open, status, question, dialog closed: {events:?}"
        );
    }

    #[test]
    fn discarding_ejects_without_writing_anything() {
        let rig = Rig::new("eject-discard");
        rig.run(
            FileAction::EjectDisk,
            "cpc664",
            ALL_MEDIA,
            dirty(None),
            vec![],
            vec![],
            vec![Some(1)], // D = discard (second option when no original)
            vec![],
        );
        let events = rig.events();
        assert!(events.contains(&Event::Apply("eject disk".to_string())));
        assert!(!events.iter().any(|e| matches!(e, Event::Export(_))));
    }

    #[test]
    fn an_unmodified_disk_is_swapped_without_a_question() {
        let rig = Rig::new("swap-clean");
        let new_disk = rig.file("next.dsk", b"abc");
        rig.run(
            FileAction::LoadDisk,
            "c64",
            ALL_MEDIA,
            clean(),
            vec![Some(new_disk)],
            vec![],
            vec![],
            vec![],
        );
        assert!(!rig.events().iter().any(|e| matches!(e, Event::Choose(_))));
    }

    #[test]
    fn next_and_previous_disk_walk_the_playlist_and_wrap() {
        let mut rig = Rig::new("playlist");
        let a = rig.file("a.d64", b"disk-a");
        let b = rig.file("b.d64", b"disk-b");
        rig.playlist = vec![a.clone(), b.clone()];
        let attached = |rig: &Rig| -> Vec<String> {
            rig.events()
                .into_iter()
                .filter_map(|e| match e {
                    Event::Apply(s) => Some(s),
                    _ => None,
                })
                .collect()
        };
        let go = |rig: &Rig, action| {
            rig.run(
                action,
                "c64",
                ALL_MEDIA,
                clean(),
                vec![],
                vec![],
                vec![],
                vec![],
            );
        };
        go(&rig, FileAction::NextDisk);
        let first = attached(&rig);
        assert_eq!(first.len(), 1);
        assert!(first[0].contains("b.d64"), "{first:?}");
        go(&rig, FileAction::NextDisk);
        assert!(
            attached(&rig)[0].contains("a.d64"),
            "wraps to the first disk"
        );
        go(&rig, FileAction::PrevDisk);
        assert!(
            attached(&rig)[0].contains("b.d64"),
            "previous wraps backwards"
        );
    }

    #[test]
    fn saving_a_state_writes_the_exported_bytes_and_proposes_a_dated_name() {
        let rig = Rig::new("save-state");
        let target = rig.dir.join("mine.state");
        let (_, _, names) = rig.run(
            FileAction::SaveState,
            "zx-spectrum",
            ALL_MEDIA,
            clean(),
            vec![],
            vec![Some(target.clone())],
            vec![],
            vec![Ok(Some(b"STATE".to_vec()))],
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"STATE");
        assert_eq!(names, ["zx-spectrum-20231114-221320.state"]);
        assert!(
            rig.events()
                .contains(&Event::Say("State saved: mine.state".to_string()))
        );
    }

    #[test]
    fn a_snapshot_is_saved_in_the_format_of_the_chosen_extension() {
        let rig = Rig::new("save-snapshot");
        let target = rig.dir.join("shot.SNA");
        let (_, _, names) = rig.run(
            FileAction::SaveSnapshot,
            "zx-spectrum",
            ALL_MEDIA,
            clean(),
            vec![],
            vec![Some(target.clone())],
            vec![],
            vec![Ok(Some(b"SNA".to_vec()))],
        );
        assert_eq!(
            names,
            ["zx-spectrum-20231114-221320.z80"],
            "Spectrum proposes .z80"
        );
        assert!(rig.events().contains(&Event::Export(Export::Snapshot {
            extension: "sna".to_string()
        })));
    }

    #[test]
    fn a_recording_needs_a_tape_format_and_an_empty_recording_is_reported() {
        let rig = Rig::new("save-tape");
        rig.run(
            FileAction::SaveTape,
            "cpc464",
            ALL_MEDIA,
            clean(),
            vec![],
            vec![Some(rig.dir.join("out.wav"))],
            vec![],
            vec![],
        );
        assert!(
            rig.events().iter().any(
                |e| matches!(e, Event::Say(m) if m.contains("Cannot save a recording as .wav"))
            )
        );

        rig.run(
            FileAction::SaveTape,
            "cpc464",
            ALL_MEDIA,
            clean(),
            vec![],
            vec![Some(rig.dir.join("out.cdt"))],
            vec![],
            vec![Ok(None)],
        );
        assert!(
            rig.events()
                .iter()
                .any(|e| matches!(e, Event::Say(m) if m.contains("Nothing has been recorded")))
        );
        assert!(!rig.dir.join("out.cdt").exists());
    }

    #[test]
    fn saving_a_disk_copy_does_not_mark_the_disk_saved() {
        let rig = Rig::new("save-disk");
        let target = rig.dir.join("copy.dsk");
        rig.run(
            FileAction::SaveDisk,
            "cpc664",
            ALL_MEDIA,
            dirty(Some(rig.dir.join("work.dsk"))),
            vec![],
            vec![Some(target.clone())],
            vec![],
            vec![Ok(Some(b"IMG".to_vec()))],
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"IMG");
        assert!(
            !rig.events()
                .contains(&Event::Apply("disk saved".to_string()))
        );
    }

    fn the_last_folder_is_used_next_time_and_survives_a_restart() {
        let rig = Rig::new("memory");
        let sub = rig.dir.join("tapes");
        std::fs::create_dir_all(&sub).unwrap();
        let state = rig.dir.join("retroclawd.state.toml");

        let mut folders = FolderMemory::default();
        folders.remember(FileKind::Tape, &sub.join("a.tap"));
        folders.save(&state).unwrap();

        let loaded = FolderMemory::load(&state);
        assert_eq!(
            loaded.get(FileKind::Tape),
            Some(std::fs::canonicalize(&sub).unwrap())
        );
        // A seed never overrides what was remembered...
        let mut loaded = loaded;
        loaded.seed(FileKind::Tape, &rig.dir.join("other.tap"));
        assert_eq!(
            loaded.get(FileKind::Tape),
            Some(std::fs::canonicalize(&sub).unwrap())
        );
        // ...but fills a kind with nothing yet.
        loaded.seed(FileKind::Disk, &rig.dir.join("d.dsk"));
        assert_eq!(
            loaded.get(FileKind::Disk),
            Some(std::fs::canonicalize(&rig.dir).unwrap())
        );
        // A folder that has gone away is not offered.
        std::fs::remove_dir_all(&sub).unwrap();
        assert_eq!(FolderMemory::load(&state).get(FileKind::Tape), None);
        // A garbled file is just "nothing remembered".
        std::fs::write(&state, "not = [toml").unwrap();
        assert_eq!(FolderMemory::load(&state), FolderMemory::default());
    }

    fn dialogs_open_in_the_remembered_folder() {
        let rig = Rig::new("start-dir");
        let tape = rig.file("t.tap", b"x");
        // Remember via a first run, then check the dir handed to the second.
        let emu = FakeEmu {
            log: &rig.log,
            status: RefCell::new(clean()),
            exports: RefCell::new(VecDeque::new()),
        };
        let ui = FakeUi {
            log: &rig.log,
            picks: RefCell::new(vec![Some(tape.clone()), None].into()),
            saves: RefCell::new(VecDeque::new()),
            answers: RefCell::new(VecDeque::new()),
            dirs: RefCell::new(Vec::new()),
            save_names: RefCell::new(Vec::new()),
        };
        let mut folders = FolderMemory::default();
        for _ in 0..2 {
            let mut session = Session {
                machine_id: "c64",
                media_kinds: ALL_MEDIA,
                emu: &emu,
                ui: &ui,
                folders: &mut folders,
                work_dir: &rig.dir,
                unix_secs: 0,
                playlist: &[],
                playlist_index: &mut 0,
            };
            run(FileAction::LoadTape, &mut session);
        }
        let dirs = ui.dirs.borrow().clone();
        assert_eq!(dirs[0], None);
        assert_eq!(dirs[1], Some(std::fs::canonicalize(&rig.dir).unwrap()));
    }
    /// Merged (every check kept): `the_last_folder_is_used_next_time_and_survives_a_restart`, `dialogs_open_in_the_remembered_folder`.
    #[test]
    fn the_last_folder_is_remembered() {
        the_last_folder_is_used_next_time_and_survives_a_restart();
        dialogs_open_in_the_remembered_folder();
    }
}
