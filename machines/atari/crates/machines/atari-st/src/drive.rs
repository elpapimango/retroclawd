//! The floppy drives on the WD1772 (`docs/hardware-facts.md`, "Floppy drives").
//!
//! - [H] *Internals* p.60, PSG port A: bit 0 side select, bit 1 drive 0 select, bit 2 drive 1 select; [H?] the
//!   selects are active low (the `$07` TOS writes deselects both, [O]) and bit 0 clear chooses side 1.
//! - [H?] one revolution is `st_floppy::TRACK_BYTES` byte times (300 rpm at 250 kbit/s); the index pulse is the
//!   first [`INDEX_BYTES`] bytes of the revolution, and only a disk turning in a drive makes one.
//! - [O] a drive without a disk reads write-protected (status `$E4` in Hatari); [H?] in Phase 1 every disk reads
//!   write-protected too, since writing arrives in Phase 2.
//! - [H?] the motor line reaches both drives, so every drive with a disk turns.

use st_floppy::{Image, TRACK_BYTES};
use wd1770::Floppy;

/// [H?] how long the index sensor sees the hole, in byte times.
pub const INDEX_BYTES: usize = 64;
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
}

impl StDrive {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, image: Image) {
        self.image = Some(image);
        self.track = None;
    }

    pub fn eject(&mut self) -> Option<Image> {
        self.track = None;
        self.image.take()
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
        true
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

    fn write(&mut self, _: u16) {
        // Write-protected in Phase 1: the controller never writes.
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
}

impl Drives {
    pub fn new(fitted: u8) -> Self {
        Self {
            drives: [StDrive::new(), StDrive::new()],
            fitted: fitted.clamp(1, 2),
            selected: None,
            side: 0,
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
        Ok(())
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
        for d in &mut self.drives {
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
        for d in &mut self.drives {
            d.write(byte);
        }
    }
}
