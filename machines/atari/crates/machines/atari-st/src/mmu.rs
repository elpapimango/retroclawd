//! The memory controller (MMU): the configuration register at `$FF8001` and the mapping of CPU addresses to the RAM
//! chips. Facts: `docs/hardware-facts.md`, "MMU".
//!
//! [H] *Atari ST Internals* pp.55-56: one byte register, bits 3-2 bank 0 and bits 1-0 bank 1, each `00` = 128K,
//! `01` = 512K, `10` = 2M, `11` reserved; it can be read and written. Bank 0 starts at 0, bank 1 directly after the
//! size configured for bank 0.

use serde::{Deserialize, Serialize};

use crate::config::RamSize;

/// The size of one bank: the chips fitted, or what the register says they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BankSize {
    None,
    K128,
    K512,
    M2,
}

impl BankSize {
    pub fn bytes(self) -> u32 {
        match self {
            BankSize::None => 0,
            BankSize::K128 => 128 << 10,
            BankSize::K512 => 512 << 10,
            BankSize::M2 => 2 << 20,
        }
    }

    /// The size a two-bit register field selects; `11` is reserved, treated as no RAM ([H?]).
    fn from_field(bits: u8) -> Self {
        match bits & 3 {
            0 => BankSize::K128,
            1 => BankSize::K512,
            2 => BankSize::M2,
            _ => BankSize::None,
        }
    }
}

/// The banks fitted for each `--ram` size. [O] the configuration EmuTOS 1.4 settles on in Hatari 2.6.1 for the same
/// RAM size (`$00`, `$04`, `$05`, `$08`, `$09`, `$0A`), decoded with the register layout above.
pub fn installed_banks(ram: RamSize) -> [BankSize; 2] {
    use BankSize::*;
    match ram {
        RamSize::K256 => [K128, K128],
        RamSize::K512 => [K512, None],
        RamSize::M1 => [K512, K512],
        RamSize::M2 => [M2, None],
        RamSize::M2_5 => [M2, K512],
        RamSize::M4 => [M2, M2],
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mmu {
    config: u8,
    installed: [BankSize; 2],
}

impl Mmu {
    /// Power-on: [O] the register reads `$00` (Hatari's first access, before TOS writes it).
    pub fn new(ram: RamSize) -> Self {
        Self {
            config: 0,
            installed: installed_banks(ram),
        }
    }

    /// [H?] bits 7-4 read as 0.
    pub fn read_config(&self) -> u8 {
        self.config
    }

    pub fn write_config(&mut self, value: u8) {
        self.config = value & 0x0F;
    }

    /// Bytes of RAM the chips hold.
    pub fn ram_bytes(&self) -> usize {
        (self.installed[0].bytes() + self.installed[1].bytes()) as usize
    }

    /// The RAM index a CPU address below `$400000` reaches, or `None` when no chip answers there ([H?]: reads `$FF`,
    /// writes vanish, no bus error).
    pub fn translate(&self, addr: u32) -> Option<usize> {
        let configured = [
            BankSize::from_field(self.config >> 2),
            BankSize::from_field(self.config),
        ];
        let (bank, offset) = if addr < configured[0].bytes() {
            (0, addr)
        } else if addr - configured[0].bytes() < configured[1].bytes() {
            (1, addr - configured[0].bytes())
        } else {
            return None;
        };
        let fitted = self.installed[bank];
        if fitted == BankSize::None {
            return None;
        }
        let cell = cell(configured[bank], fitted, offset)?;
        let base = if bank == 1 {
            self.installed[0].bytes()
        } else {
            0
        };
        Some((base + cell) as usize)
    }
}

/// Address lines per row (and per column) of the DRAM chips a bank of this size is built from: 16 chips of 64K,
/// 256K or 1M bits (*Internals* p.55), square arrays of 8, 9 or 10 lines.
fn lines(size: BankSize) -> u32 {
    match size {
        BankSize::K128 => 8,
        BankSize::K512 => 9,
        BankSize::M2 => 10,
        BankSize::None => 0,
    }
}

/// Where `offset` (within a bank configured as `configured`) lands in chips of size `fitted`.
///
/// [H?] The MMU splits the word address into a column (the low `n` bits) and a row (the next `n` bits), `n` from
/// the configured size, and drives both over the same lines; a chip with fewer lines ignores the top ones. So in a
/// bank configured for 2M, 256K-bit chips ignore A10 and A20 and 64K-bit chips also A9 and A19: a pattern written
/// at offset `$8` reads back at `$408` (512K fitted) or `$208` (128K fitted), two of the offsets EmuTOS 1.4's
/// sizing checks (`$FC01BC-$FC0202`), and the result Hatari 2.6.1 gives for every RAM size ([O] final `$FF8001`
/// values). No primary source for the multiplexing was found; see `docs/hardware-facts.md`.
fn cell(configured: BankSize, fitted: BankSize, offset: u32) -> Option<u32> {
    let n = lines(configured);
    let m = lines(fitted);
    let word = offset >> 1;
    let column = word & ((1 << n) - 1);
    let row = (word >> n) & ((1 << n) - 1);
    let mask = (1 << m) - 1;
    let cell = ((row & mask) << m) | (column & mask);
    Some(cell << 1 | (offset & 1))
}
