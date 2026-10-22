//! The DMA chip and the WD1772 behind it (`docs/hardware-facts.md`, "DMA and floppy controller").
//!
//! - [H] *Internals* p.59: `$FF8604` reaches the FDC (A1/A0 from the mode register) or the sector count, read and
//!   write; its upper 8 bits read as 1. `$FF8606` written is the mode register (bit 1 A0, bit 2 A1, bit 3 HDC, bit 4
//!   sector count, bit 6 DMA off, bit 8 write); read, its low three bits are the status (bit 0 error, bit 1 sector
//!   count not zero, bit 2 FDC DRQ). [O] bit 0 is set when there was no error (the book prints the opposite; see
//!   `status`). [H?] its other bits read 0; bit 7 is not decoded.
//! - [H] `$FF8609/0B/0D` are the DMA address high, mid, low, read and write.
//! - [H?] the DMA moves FDC bytes through a 16-byte FIFO into memory a burst at a time (and, writing, takes the
//!   FDC's bytes from the FIFO, refilled from memory 16 bytes at a time as it empties), with no bus stall;
//!   the sector count is 8 bits and counts 512-byte sectors; changing the mode register's write bit clears the FIFO
//!   and the error bit.
//! - [H] the FDC's INTRQ drives GPIP I5 (*Internals* p.61), active low.
//! - [H?] the WD1772 runs from the CPU clock (8 MHz): its timings are the data sheet's at 8 MHz.
//! - [O] an HDC (ACSI) access with nothing connected: writes vanish and I5 stays high (EmuTOS's probe in Hatari).
//!   [H?] an HDC read has no fact and stops the machine.

use serde::{Deserialize, Serialize};
use wd1770::{Wd1770, Wd1770State};

use crate::drive::Drives;

/// One byte time of the controller, in CPU cycles (32 µs at 8 MHz).
pub const FDC_BYTE_CYCLES: u64 = 256;
const FIFO_BYTES: usize = 16;
const SECTOR_BYTES: u16 = 512;

pub struct Dma {
    pub(crate) fdc: Wd1770<Drives>,
    /// The CPU cycle the controller has been advanced to.
    pub(crate) fdc_at: u64,
    pub(crate) mode: u16,
    sector_count: u16,
    pub(crate) address: u32,
    pub(crate) fifo: Vec<u8>,
    bytes_in_sector: u16,
    error: bool,
    /// The generation of the one current controller event.
    pub(crate) generation: u64,
}

impl std::fmt::Debug for Dma {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dma")
            .field("mode", &self.mode)
            .field("sector_count", &self.sector_count)
            .field("address", &self.address)
            .finish()
    }
}

/// The DMA chip, the controller and the drives as data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DmaState {
    fdc: Wd1770State,
    drives: Drives,
    fdc_at: u64,
    mode: u16,
    sector_count: u16,
    address: u32,
    fifo: Vec<u8>,
    bytes_in_sector: u16,
    error: bool,
    generation: u64,
}

impl Dma {
    pub(crate) fn snapshot(&self) -> DmaState {
        DmaState {
            fdc: self.fdc.dump_state(),
            drives: self.fdc.floppy.stripped_for_state(),
            fdc_at: self.fdc_at,
            mode: self.mode,
            sector_count: self.sector_count,
            address: self.address,
            fifo: self.fifo.clone(),
            bytes_in_sector: self.bytes_in_sector,
            error: self.error,
            generation: self.generation,
        }
    }

    /// The state's FIFO never holds more than the chip's 16 bytes and its sector byte count stays inside a sector.
    pub(crate) fn restore(&mut self, s: DmaState) -> Result<(), String> {
        if s.fifo.len() > FIFO_BYTES || s.bytes_in_sector >= SECTOR_BYTES {
            return Err("the DMA state is not one this machine can be in".into());
        }
        let drives = self.fdc.floppy.merged_with_state(s.drives)?;
        self.fdc.restore_state(s.fdc);
        self.fdc.floppy = drives;
        self.fdc_at = s.fdc_at;
        self.mode = s.mode;
        self.sector_count = s.sector_count;
        self.address = s.address & 0xFF_FFFF;
        self.fifo = s.fifo;
        self.bytes_in_sector = s.bytes_in_sector;
        self.error = s.error;
        self.generation = s.generation;
        Ok(())
    }

    pub fn new(drives: u8) -> Self {
        Self {
            fdc: Wd1770::new(Drives::new(drives), 8, true),
            fdc_at: 0,
            mode: 0,
            sector_count: 0,
            address: 0,
            fifo: Vec::with_capacity(FIFO_BYTES),
            bytes_in_sector: 0,
            error: false,
            generation: 0,
        }
    }

    /// The mode register as last written.
    pub fn mode(&self) -> u16 {
        self.mode
    }

    /// The FDC's register `reg` (0 status, 1 track, 2 sector, 3 data), without side effects.
    pub fn fdc_register_value(&self, reg: u8) -> u8 {
        self.fdc.peek(reg)
    }

    pub(crate) fn hdc_selected(&self) -> bool {
        self.mode & 0x08 != 0
    }

    fn fdc_register(&self) -> u8 {
        ((self.mode >> 1) & 3) as u8
    }

    /// The status the CPU reads at `$FF8606`. [O] bit 0 is set when there was no DMA error: Hatari reads `$..b1`
    /// after a good transfer and EmuTOS retries a read that leaves it clear; *Internals* p.58 prints the opposite
    /// ("0=no error"), a conflict recorded in `docs/hardware-facts.md`.
    pub fn status(&self) -> u16 {
        (!self.error as u16) | ((self.sector_count != 0) as u16) << 1 | (self.fdc.drq() as u16) << 2
    }

    /// `$FF8604` read, with the FDC up to date.
    pub(crate) fn read_data(&mut self) -> u16 {
        let low = if self.mode & 0x10 != 0 {
            self.sector_count as u8
        } else {
            self.fdc.read(self.fdc_register())
        };
        0xFF00 | low as u16
    }

    pub(crate) fn write_data(&mut self, value: u16) {
        if self.hdc_selected() {
            return;
        }
        if self.mode & 0x10 != 0 {
            self.sector_count = value & 0xFF;
            self.bytes_in_sector = 0;
        } else {
            self.fdc.write(self.fdc_register(), value as u8);
        }
    }

    pub(crate) fn write_mode(&mut self, value: u16) {
        if (value ^ self.mode) & 0x100 != 0 {
            self.fifo.clear();
            self.error = false;
        }
        self.mode = value;
    }

    /// Address register byte `n` (0 high, 1 mid, 2 low).
    pub(crate) fn address_byte(&self, n: u32) -> u8 {
        (self.address >> (16 - 8 * n)) as u8
    }

    pub(crate) fn set_address_byte(&mut self, n: u32, value: u8) {
        let shift = 16 - 8 * n;
        self.address = (self.address & !(0xFF << shift)) | (value as u32) << shift;
    }

    /// Whether a command is running (the controller's BUSY bit), whatever the data request line says.
    pub(crate) fn command_running(&self) -> bool {
        self.fdc.peek(0) & 0x01 != 0
    }

    /// Whether the controller has anything to do (a command running or a byte waiting).
    pub(crate) fn busy(&self) -> bool {
        self.fdc.peek(0) & 0x01 != 0 || self.fdc.drq()
    }

    fn writing(&self) -> bool {
        self.mode & 0x100 != 0 && self.mode & 0x40 == 0 && !self.hdc_selected()
    }

    /// Whether the controller wants a byte to write and the FIFO is empty: the DMA reads the next burst from memory.
    pub(crate) fn needs_burst(&self) -> bool {
        self.writing() && self.fdc.drq() && self.fifo.is_empty() && self.sector_count != 0
    }

    pub(crate) fn load_burst(&mut self, bytes: Vec<u8>) {
        self.fifo = bytes;
    }

    /// Hands the controller its next byte from the FIFO when it asks for one and the DMA is writing.
    pub(crate) fn service_write_drq(&mut self) {
        if !self.writing() || !self.fdc.drq() || self.fifo.is_empty() {
            return;
        }
        let byte = self.fifo.remove(0);
        self.fdc.write(3, byte);
        self.bytes_in_sector += 1;
        if self.bytes_in_sector == SECTOR_BYTES {
            self.bytes_in_sector = 0;
            self.sector_count -= 1;
        }
    }

    pub(crate) const BURST_BYTES: usize = FIFO_BYTES;

    /// Takes the controller's byte into the FIFO when the DMA is reading and has sectors left; returns a full FIFO
    /// to be written to memory.
    pub(crate) fn service_drq(&mut self) -> Option<Vec<u8>> {
        let reading = self.mode & 0x100 == 0 && self.mode & 0x40 == 0 && !self.hdc_selected();
        if !self.fdc.drq() || !reading || self.sector_count == 0 {
            return None;
        }
        let byte = self.fdc.read(3);
        self.fifo.push(byte);
        self.bytes_in_sector += 1;
        if self.bytes_in_sector == SECTOR_BYTES {
            self.bytes_in_sector = 0;
            self.sector_count -= 1;
        }
        if self.fifo.len() == FIFO_BYTES {
            return Some(std::mem::take(&mut self.fifo));
        }
        None
    }
}
