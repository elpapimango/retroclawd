//! The floppy drives on the WD1772 (`docs/hardware-facts.md`, "Floppy drives").
//!
//! - [H] *Internals* p.60, PSG port A: bit 0 side select, bit 1 drive 0 select, bit 2 drive 1 select; [H?] the
//!   selects are active low (the `$07` TOS writes deselects both, [O]) and bit 0 clear chooses side 1.
//! - [H?] one revolution is `st_floppy::TRACK_BYTES` byte times (300 rpm at 250 kbit/s); the index pulse is the
//!   first [`INDEX_BYTES`] bytes of the revolution, and only a disk turning in a drive makes one.
//! - [O] a drive without a disk reads write-protected (status `$E4` in Hatari), one with a disk does not (`$24`,
//!   `$A4`).
//! - Writing: a byte the controller writes goes into the track under the head (the bytes the drive keeps for the
//!   current cylinder and side); when the head leaves the track, the disk is ejected or the controller is idle, the
//!   track is decoded back into sectors (`st_floppy::decode`) and the image takes them. The disk counts as modified
//!   from the first byte written until it is marked saved. A write-protected disk (the sensor, set by the user)
//!   takes nothing; the controller sees the write-protect line and stops the command itself.
//! - [H?] the motor line reaches both drives, so every drive with a disk turns.

use st_floppy::{Image, TRACK_BYTES};
use wd1770::Floppy;

/// [H?] how long the index sensor sees the hole, in byte times.
pub const INDEX_BYTES: usize = 64;
/// [H?] how long the write-protect sensor stays blocked while a disk goes in or out, in byte times (0.5 s). TOS
/// notices a disk change by this transition.
pub const MEDIA_CHANGE_BYTES: u64 = 15_625;
/// [H?] the last cylinder the head can reach.
const MAX_CYLINDER: u8 = 85;

/// One drive: a head, a spindle and maybe a disk.
#[derive(Debug, Clone, Default)]
pub struct StDrive {
    image: Option<Image>,
    cylinder: u8,
    /// Byte position under the head within the revolution.
    pos: usize,
    index_count: u32,
    /// The track under the head, synthesized from the image (cylinder, side, bytes).
    track: Option<(u8, u8, Vec<u16>)>,
    /// The write-protect tab, set by the user.
    write_protected: bool,
    /// The track under the head holds bytes the controller wrote that the image has not taken yet.
    track_dirty: bool,
    /// The image holds sectors the controller wrote since it was inserted or last marked saved.
    modified: bool,
    /// Byte times the write-protect sensor stays blocked by a disk going in or out.
    swap_hold: u64,
}

impl StDrive {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, image: Image) {
        self.commit_track();
        self.image = Some(image);
        self.track = None;
        self.track_dirty = false;
        self.modified = false;
    }

    pub fn eject(&mut self) -> Option<Image> {
        self.commit_track();
        self.track = None;
        self.modified = false;
        self.image.take()
    }

    /// Sets the write-protect tab.
    pub fn set_write_protected(&mut self, on: bool) {
        self.write_protected = on;
    }

    /// Whether the controller has written to the disk since it was inserted or last marked saved.
    pub fn is_modified(&self) -> bool {
        self.modified || self.track_dirty
    }

    /// The image has been saved: it is no longer modified.
    pub fn mark_saved(&mut self) {
        self.commit_track();
        self.modified = false;
    }

    /// Decodes the written track back into the image's sectors. A track that does not decode (the controller wrote
    /// garbage, or a half-formatted track) changes nothing and is dropped, so the next read sees the image's own.
    pub fn commit_track(&mut self) {
        if !std::mem::take(&mut self.track_dirty) {
            return;
        }
        let Some((cyl, side, bytes)) = self.track.take() else {
            return;
        };
        let Some(image) = self.image.as_mut() else {
            return;
        };
        if let Ok(n) = image.apply_track(cyl, side, &bytes) {
            self.modified |= n > 0;
            self.track = Some((cyl, side, bytes));
        }
    }

    /// Whether there is a written track waiting to be taken into the image.
    pub fn has_pending_track(&self) -> bool {
        self.track_dirty
    }

    pub fn image(&self) -> Option<&Image> {
        self.image.as_ref()
    }

    pub fn has_disk(&self) -> bool {
        self.image.is_some()
    }

    pub fn cylinder(&self) -> u8 {
        self.cylinder
    }

    fn track_bytes(&mut self, side: u8) -> &[u16] {
        let cyl = self.cylinder;
        let stale = !matches!(&self.track, Some((c, s, _)) if *c == cyl && *s == side);
        if stale {
            self.commit_track();
            let bytes = match &self.image {
                Some(img) => img.track(cyl, side),
                None => vec![0; TRACK_BYTES],
            };
            self.track = Some((cyl, side, bytes));
        }
        &self.track.as_ref().expect("just made").2
    }

    fn read_side(&mut self, side: u8) -> u16 {
        if self.image.is_none() {
            return 0;
        }
        let pos = self.pos;
        let byte = self.track_bytes(side)[pos];
        self.rotate(1);
        byte
    }
}

impl Floppy for StDrive {
    fn index(&self) -> bool {
        self.image.is_some() && self.pos < INDEX_BYTES
    }

    fn index_count(&self) -> u32 {
        self.index_count
    }

    fn index_count_reset(&mut self) {
        self.index_count = 0;
    }

    fn rotate(&mut self, bytes: u64) {
        if self.image.is_none() {
            return;
        }
        let total = self.pos as u64 + bytes;
        self.index_count = self
            .index_count
            .saturating_add((total / TRACK_BYTES as u64) as u32);
        self.pos = (total % TRACK_BYTES as u64) as usize;
    }

    fn track0(&self) -> bool {
        self.cylinder == 0
    }

    fn write_protect(&self) -> bool {
        self.image.is_none() || self.swap_hold > 0 || self.write_protected
    }

    fn seek_pulse(&mut self, inward: bool) {
        self.cylinder = if inward {
            (self.cylinder + 1).min(MAX_CYLINDER)
        } else {
            self.cylinder.saturating_sub(1)
        };
    }

    fn read(&mut self) -> u16 {
        self.read_side(0)
    }

    fn write(&mut self, byte: u16) {
        self.write_side(0, byte);
    }
}

impl StDrive {
    /// The controller writes `byte` (bit 8 = sync mark) under the head on `side`; the disk turns by one byte.
    fn write_side(&mut self, side: u8, byte: u16) {
        if self.image.is_some() && !self.write_protected {
            let pos = self.pos;
            self.track_bytes(side);
            if let Some((_, _, t)) = self.track.as_mut()
                && let Some(slot) = t.get_mut(pos)
            {
                *slot = byte;
                self.track_dirty = true;
            }
        }
        self.rotate(1);
    }
}

/// Both drive connectors as the controller sees them: the selected drive and side from PSG port A.
#[derive(Debug, Clone)]
pub struct Drives {
    drives: [StDrive; 2],
    /// Drives fitted (1 or 2).
    fitted: u8,
    selected: Option<usize>,
    side: u8,
    /// Byte times since power-on (the controller's clock as the drives see it).
    elapsed: u64,
}

impl Drives {
    pub fn new(fitted: u8) -> Self {
        Self {
            drives: [StDrive::new(), StDrive::new()],
            fitted: fitted.clamp(1, 2),
            selected: None,
            side: 0,
            elapsed: 0,
        }
    }

    pub fn fitted(&self) -> u8 {
        self.fitted
    }

    pub fn drive_mut(&mut self, n: usize) -> &mut StDrive {
        &mut self.drives[n & 1]
    }

    pub fn drive(&self, n: usize) -> &StDrive {
        &self.drives[n & 1]
    }

    pub fn insert(&mut self, n: usize, image: Image) -> Result<(), String> {
        if n >= self.fitted as usize {
            return Err(format!(
                "this ST has {} floppy drive{}: no drive {}",
                self.fitted,
                if self.fitted == 1 { "" } else { "s" },
                if n == 0 { "A" } else { "B" }
            ));
        }
        self.drives[n].insert(image);
        self.hold_sensor(n);
        Ok(())
    }

    /// Takes the disk out of drive `n`.
    pub fn eject(&mut self, n: usize) -> Option<Image> {
        let image = self.drives[n & 1].eject();
        if image.is_some() {
            self.hold_sensor(n & 1);
        }
        image
    }

    /// A disk going in or out while the machine runs blocks the write-protect sensor; at power-on it is simply there.
    fn hold_sensor(&mut self, n: usize) {
        if self.elapsed > 0 {
            self.drives[n].swap_hold = MEDIA_CHANGE_BYTES;
        }
    }

    /// PSG port A's output: drive selects (active low) and side.
    pub fn set_port_a(&mut self, value: u8) {
        let fitted = |n: usize| n < self.fitted as usize;
        self.selected = match (value & 0x02 == 0, value & 0x04 == 0) {
            (true, _) => Some(0),
            (false, true) if fitted(1) => Some(1),
            _ => None,
        };
        self.side = if value & 0x01 == 0 { 1 } else { 0 };
    }

    /// Takes every written track into its image (see [`StDrive::commit_track`]).
    pub fn commit_pending(&mut self) {
        for d in &mut self.drives {
            d.commit_track();
        }
    }

    /// Whether any drive holds a written track not yet in its image.
    pub fn has_pending_track(&self) -> bool {
        self.drives.iter().any(StDrive::has_pending_track)
    }

    fn sel(&self) -> Option<&StDrive> {
        self.selected.map(|n| &self.drives[n])
    }
}

impl Floppy for Drives {
    fn index(&self) -> bool {
        self.sel().is_some_and(Floppy::index)
    }

    fn index_count(&self) -> u32 {
        self.sel().map_or(0, Floppy::index_count)
    }

    fn index_count_reset(&mut self) {
        for d in &mut self.drives {
            d.index_count_reset();
        }
    }

    fn rotate(&mut self, bytes: u64) {
        self.elapsed += bytes;
        for d in &mut self.drives {
            d.swap_hold = d.swap_hold.saturating_sub(bytes);
            d.rotate(bytes);
        }
    }

    fn track0(&self) -> bool {
        self.sel().is_some_and(Floppy::track0)
    }

    fn write_protect(&self) -> bool {
        self.sel().is_none_or(Floppy::write_protect)
    }

    fn seek_pulse(&mut self, inward: bool) {
        if let Some(n) = self.selected {
            self.drives[n].seek_pulse(inward);
        }
    }

    fn read(&mut self) -> u16 {
        let side = self.side;
        let byte = match self.selected {
            Some(n) => self.drives[n].read_side(side),
            None => 0,
        };
        // The other drive's spindle turns with the shared motor line.
        for (i, d) in self.drives.iter_mut().enumerate() {
            if Some(i) != self.selected {
                d.rotate(1);
            }
        }
        if self.selected.is_some_and(|n| !self.drives[n].has_disk()) {
            return 0;
        }
        byte
    }

    fn write(&mut self, byte: u16) {
        let side = self.side;
        if let Some(n) = self.selected {
            self.drives[n].write_side(side, byte);
        }
        // The other drives' spindles turn with the shared motor line.
        for (i, d) in self.drives.iter_mut().enumerate() {
            if Some(i) != self.selected {
                d.rotate(1);
            }
        }
    }
}
