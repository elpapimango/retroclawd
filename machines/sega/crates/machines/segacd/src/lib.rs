//! Sega CD / Mega-CD: the second 68000 (the sub-CPU) with its program RAM, word RAM and backup RAM, the gate array, and the
//! main CPU's view of all of it, as an `Expansion` on the Genesis cartridge port (facts C1-C15 in `docs/hardware-facts.md`).
//!
//! The CD drive, the CD controller, the PCM chip and the graphics unit are later slices: the sub-CPU's accesses to their
//! addresses stop the machine with a report, as the project rule says for an address no fact covers.
#![forbid(unsafe_code)]

pub mod audio;
pub mod drive;

use genesis::mc68000::{BusFault, Cpu, FunctionCode, IackResponse, M68kBus, Size};
use genesis::{Config, Expansion, Genesis, Region};
use mcd_gate_array::{GateArray, WordRamOwner};
use serde::{Deserialize, Serialize};
use std::any::Any;

/// The sub-CPU's clock (fact C10).
pub const SUB_CLOCK_HZ: u64 = 12_500_000;
pub const BIOS_SIZE: usize = 0x2_0000;
pub const PRG_RAM_SIZE: usize = 0x8_0000;
pub const WORD_RAM_SIZE: usize = 0x4_0000;
pub const BACKUP_RAM_SIZE: usize = 0x2000;

pub struct SegaCdConfig {
    /// A disc to put in the drive (a cue sheet or an ISO).
    pub disc: Option<std::path::PathBuf>,
    /// The BIOS ROM (128 KB).
    pub bios: Vec<u8>,
    pub region: Region,
    pub pads: [genesis::PadKind; 2],
    /// The Genesis side's bus timing effects (see the `execution_mode` option).
    pub exact_timing: bool,
}

impl SegaCdConfig {
    pub fn new(bios: Vec<u8>, region: Region) -> Self {
        Self {
            disc: None,
            bios,
            region,
            pads: [genesis::PadKind::Six; 2],
            exact_timing: true,
        }
    }
}

/// A Genesis with a Mega-CD on its cartridge port.
pub fn new_machine(config: SegaCdConfig) -> Result<Genesis, String> {
    let mut exp = SegaCdExp::new(config.bios, config.region.master_hz())?;
    if let Some(path) = &config.disc {
        let disc = disc::Disc::open(path).map_err(|e| format!("disc: {e}"))?;
        exp.insert_disc(disc);
    }
    // The cartridge slot is empty in CD-boot mode (the BIOS answers the whole cartridge area); the Genesis needs some ROM to start.
    let rom = vec![0u8; 0x400];
    let mut genesis_config = Config::new(rom, config.region);
    genesis_config.pads = config.pads;
    genesis_config.exact_timing = config.exact_timing;
    Genesis::with_expansion(genesis_config, Some(Box::new(exp)))
}

/// What the sub-CPU's bus reaches: everything but the CPU itself, so the CPU can borrow it while it runs.
struct SubSide {
    bios: Vec<u8>,
    prg: Box<[u8]>,
    word: Box<[u8]>,
    backup: Box<[u8]>,
    gate: GateArray,
    pcm: rf5c164::Pcm,
    asic: mcd_asic::Asic,
    /// The sub-CPU clock at the instruction being executed (for the graphics unit's busy time).
    clock: u64,
    /// The sub-CPU touched word RAM it does not own (fact C7): it waits forever, until a forced reset.
    hung: bool,
    stop: Option<String>,
    pc: u32,
    /// Debugging aid: the sub-CPU's writes to the gate array registers, `(pc, address, size, value)`.
    io_log: Option<Vec<(u32, u32, Size, u32)>>,
    /// Debugging aid: the addresses `io_log` records (reads and writes), `(first, last)`.
    io_filter: (u32, u32),
    /// Debugging aid: an address range besides the registers whose writes are logged.
    watch: (u32, u32),
}

/// The sub-CPU's address lines A1-A19 only are connected: the map repeats every 1 MB. The top 128 KB of the megabyte (backup RAM,
/// PCM chip, registers) is shown at `$FE0000-$FFFFFF` in the manual and used here at that place (fact C20).
fn sub_address(addr: u32) -> u32 {
    let a = addr & 0x000F_FFFF;
    if a >= 0x0E_0000 { 0x00F0_0000 | a } else { a }
}

/// The cell image the main CPU sees at `$220000-$23FFFF` in 1M mode (fact C19): five stretches of its bank, each a bitmap of
/// 512 dot wide lines (256 bytes a line) that reads as 8x8 dot cells of 32 bytes stacked in columns of `cells` cells. Returns
/// the offset in the bank's 128 KB of the byte that the cell image byte at `off` (from `$220000`) shows.
fn cell_image_source(off: u32) -> usize {
    // (cell image start, bitmap start in the bank, cells in a column)
    const AREAS: [(u32, u32, u32); 5] = [
        (0x0_0000, 0x0_0000, 32),
        (0x1_0000, 0x1_0000, 16),
        (0x1_8000, 0x1_8000, 8),
        (0x1_C000, 0x1_C000, 4),
        (0x1_E000, 0x1_E000, 4),
    ];
    let &(start, src, cells) = AREAS.iter().rev().find(|a| off >= a.0).unwrap_or(&AREAS[0]);
    let o = off - start;
    let (cell, within) = (o / 32, o % 32);
    let (col, row) = (cell / cells, cell % cells);
    let (line, byte) = (row * 8 + within / 4, within % 4);
    (src + line * 256 + col * 4 + byte) as usize
}

impl SubSide {
    /// Carries out the CD controller's DMA transfers into program RAM, word RAM or the PCM chip's wave RAM (fact E13): the
    /// DMA address register counts in units of 8 bytes and moves on by what was transferred.
    fn apply_cdc_dma(&mut self) {
        while let Some(dma) = self.gate.take_cdc_dma() {
            let start = usize::from(self.gate.dma_address()) * 8;
            self.put_dma_bytes(dma.destination, start, &dma.bytes);
            self.gate.advance_dma_address(dma.bytes.len());
        }
    }

    /// Writes the bytes of one transfer from the DMA address `start` on.
    fn put_dma_bytes(
        &mut self,
        destination: mcd_gate_array::cdc::Destination,
        start: usize,
        bytes: &[u8],
    ) {
        use mcd_gate_array::cdc::Destination;
        for (i, b) in bytes.iter().enumerate() {
            let at = start + i;
            match destination {
                Destination::ProgramRam => self.prg[at & (PRG_RAM_SIZE - 1)] = *b,
                Destination::WordRam if !self.gate.mode_1m() => {
                    self.word[at & (WORD_RAM_SIZE - 1)] = *b;
                }
                // in 1M mode the transfer fills the bank the sub-CPU has (fact E18)
                Destination::WordRam => {
                    let bank = 1 - self.gate.main_bank_1m();
                    self.word[bank * 0x2_0000 + (at & 0x1_FFFF)] = *b;
                }
                // the chip's address space: the wave RAM window starts at $1000 (fact P2)
                Destination::Pcm if (0x1000..0x2000).contains(&at) => {
                    self.pcm.bank_mut()[at - 0x1000] = *b;
                }
                _ => {}
            }
        }
    }

    fn fail(&mut self, who: &str, write: bool, addr: u32, size: Size, value: u32) {
        if self.stop.is_some() {
            return;
        }
        let width = if size == Size::Byte { "byte" } else { "word" };
        self.stop = Some(if write {
            format!(
                "stop: {who} write {width} ${addr:06X} = ${value:X} (pc ${:06X}): no fact covers this address",
                self.pc
            )
        } else {
            format!(
                "stop: {who} read {width} ${addr:06X} (pc ${:06X}): no fact covers this address",
                self.pc
            )
        });
    }

    fn sub_byte(&mut self, addr: u32) -> Option<u8> {
        match addr {
            0..=0x07_FFFF => Some(self.prg[addr as usize]),
            0x08_0000..=0x0B_FFFF => self.word_ram_byte(addr - 0x08_0000),
            // in 2M mode the range of the 1M mode bank repeats the first 128 KB of word RAM (fact C18)
            0x0C_0000..=0x0D_FFFF if self.gate.mode_1m() => {
                let bank = 1 - self.gate.main_bank_1m();
                Some(self.word[bank * 0x2_0000 + (addr - 0x0C_0000) as usize])
            }
            0x0C_0000..=0x0D_FFFF => self.word_ram_byte(addr - 0x0C_0000),
            0xFE_0000..=0xFE_3FFF => {
                // the battery RAM answers on odd addresses only (fact C2); even ones are not decoded (open bus, [H?])
                Some(if addr & 1 == 1 {
                    self.backup[((addr - 0xFE_0000) >> 1) as usize]
                } else {
                    0xFF
                })
            }
            // the PCM chip answers on odd addresses (fact C2), its 13-bit address is A12-A1
            0xFF_0000..=0xFF_3FFF => {
                if addr & 1 == 0 {
                    Some(0xFF)
                } else {
                    self.pcm.read(((addr - 0xFF_0000) >> 1) as u16)
                }
            }
            0xFF_8000..=0xFF_81FF => self.gate.sub_read_byte(addr - 0xFF_8000),
            _ => None,
        }
    }

    /// A byte of word RAM for the sub-CPU in 2M mode; `None` in 1M mode (not modelled, fact C8).
    fn word_ram_byte(&mut self, off: u32) -> Option<u8> {
        if self.gate.mode_1m() {
            // the dot image of the sub-CPU's bank: one dot a byte, the left dot of each stored byte first (fact C19)
            let bank = 1 - self.gate.main_bank_1m();
            let byte = self.word[bank * 0x2_0000 + (off >> 1) as usize];
            return Some(if off & 1 == 0 { byte >> 4 } else { byte & 0xF });
        }
        if self.gate.word_ram_owner() != WordRamOwner::Sub {
            self.hung = true;
            return Some(0);
        }
        Some(self.word[off as usize])
    }

    fn sub_write_byte(&mut self, addr: u32, v: u8) -> bool {
        match addr {
            0..=0x07_FFFF => {
                self.prg[addr as usize] = v;
                true
            }
            0x0C_0000..=0x0D_FFFF if self.gate.mode_1m() => {
                let bank = 1 - self.gate.main_bank_1m();
                self.word[bank * 0x2_0000 + (addr - 0x0C_0000) as usize] = v;
                true
            }
            0x08_0000..=0x0B_FFFF if self.gate.mode_1m() => {
                // a dot written into the dot image goes into a nibble, under the write priority mode (fact G8)
                let bank = 1 - self.gate.main_bank_1m();
                let at = bank * 0x2_0000 + ((addr - 0x08_0000) >> 1) as usize;
                let (old, shift) = if addr & 1 == 0 {
                    (self.word[at] >> 4, 4)
                } else {
                    (self.word[at] & 0xF, 0)
                };
                let new = v & 0xF;
                let write = match self.gate.priority_mode() {
                    1 => old == 0,
                    2 => new != 0,
                    _ => true,
                };
                if write {
                    self.word[at] = (self.word[at] & !(0xF << shift)) | (new << shift);
                }
                true
            }
            0x08_0000..=0x0D_FFFF => {
                if self.gate.mode_1m() {
                    return false;
                }
                if self.gate.word_ram_owner() != WordRamOwner::Sub {
                    self.hung = true;
                    return true;
                }
                let base = if addr >= 0x0C_0000 {
                    0x0C_0000
                } else {
                    0x08_0000
                };
                self.word[(addr - base) as usize] = v;
                true
            }
            0xFE_0000..=0xFE_3FFF => {
                if addr & 1 == 1 {
                    self.backup[((addr - 0xFE_0000) >> 1) as usize] = v;
                }
                true
            }
            0xFF_0000..=0xFF_3FFF => {
                addr & 1 == 0 || self.pcm.write(((addr - 0xFF_0000) >> 1) as u16, v)
            }
            0xFF_8000..=0xFF_81FF => self.gate.sub_write_byte(addr - 0xFF_8000, v),
            _ => false,
        }
    }

    /// The registers take whole words, so a word access to them is not two byte accesses.
    fn sub_read(&mut self, addr: u32, size: Size) -> Option<u32> {
        if (0xFF_8000..=0xFF_81FF).contains(&addr) && mcd_asic::Asic::owns(addr - 0xFF_8000) {
            let word = self.asic.read_word(addr - 0xFF_8000, self.clock)?;
            return Some(match size {
                Size::Word => u32::from(word),
                Size::Byte if addr & 1 == 0 => u32::from(word >> 8),
                Size::Byte => u32::from(word & 0xFF),
            });
        }
        if (0xFF_8000..=0xFF_81FF).contains(&addr) && size == Size::Word {
            return self.gate.sub_read_word(addr - 0xFF_8000).map(u32::from);
        }
        match size {
            Size::Byte => self.sub_byte(addr).map(u32::from),
            Size::Word => {
                let hi = self.sub_byte(addr)?;
                let lo = self.sub_byte(addr | 1)?;
                Some(u32::from(hi) << 8 | u32::from(lo))
            }
        }
    }

    fn sub_write(&mut self, addr: u32, size: Size, value: u32) -> bool {
        if (0xFF_8000..=0xFF_81FF).contains(&addr) && mcd_asic::Asic::owns(addr - 0xFF_8000) {
            let off = addr - 0xFF_8000;
            let word = match size {
                Size::Word => value as u16,
                // the trace vector register takes words only; the others merge a byte into their current value
                Size::Byte => {
                    let Some(old) = self.asic.read_word(off, self.clock) else {
                        return false;
                    };
                    if off & 1 == 0 {
                        (old & 0x00FF) | ((value as u16) << 8)
                    } else {
                        (old & 0xFF00) | (value as u16 & 0xFF)
                    }
                }
            };
            if self.gate.mode_1m() {
                return false; // the graphics unit works in 2M mode only; 1M mode is not modelled (fact C8)
            }
            let pm = self.gate.priority_mode();
            return self
                .asic
                .write_word(off, word, &mut self.word, pm, self.clock);
        }
        if (0xFF_8000..=0xFF_81FF).contains(&addr) && size == Size::Word {
            return self.gate.sub_write_word(addr - 0xFF_8000, value as u16);
        }
        match size {
            Size::Byte => self.sub_write_byte(addr, value as u8),
            Size::Word => {
                self.sub_write_byte(addr, (value >> 8) as u8)
                    && self.sub_write_byte(addr | 1, value as u8)
            }
        }
    }
}

impl M68kBus for SubSide {
    fn read(&mut self, addr: u32, size: Size, _fc: FunctionCode) -> Result<u32, BusFault> {
        let addr = sub_address(addr);
        match self.sub_read(addr, size) {
            Some(v) => {
                if (self.io_filter.0..=self.io_filter.1).contains(&addr)
                    && (0xFF_8004..=0xFF_800B).contains(&addr)
                    && let Some(log) = self.io_log.as_mut()
                    && log.len() < 200_000
                {
                    log.push((self.pc, addr | 0x8000_0000, size, v));
                }
                Ok(v)
            }
            None => {
                self.fail("sub-CPU", false, addr, size, 0);
                Ok(0)
            }
        }
    }

    fn write(
        &mut self,
        addr: u32,
        size: Size,
        value: u32,
        _fc: FunctionCode,
    ) -> Result<(), BusFault> {
        let addr = sub_address(addr);
        let watched = (0xFF_8000..=0xFF_81FF).contains(&addr)
            || (self.watch.0..=self.watch.1).contains(&addr);
        if watched
            && (self.io_filter.0..=self.io_filter.1).contains(&addr)
            && let Some(log) = self.io_log.as_mut()
            && log.len() < 200_000
        {
            log.push((self.pc, addr, size, value));
        }
        if !self.sub_write(addr, size, value) {
            self.fail("sub-CPU", true, addr, size, value);
        }
        self.apply_cdc_dma();
        Ok(())
    }

    fn iack(&mut self, level: u8) -> IackResponse {
        self.gate.ack_sub_irq(level);
        IackResponse::Autovector
    }
}

/// The Mega-CD as the Genesis bus sees it.
pub struct SegaCdExp {
    side: SubSide,
    sub: Cpu,
    master_hz: u64,
    last_master: u64,
    /// Master clocks times the sub clock not yet turned into whole sub clocks (kept exact, no floats).
    remainder: u64,
    /// Sub clocks that have come due, and those the sub-CPU has executed.
    due: u64,
    spent: u64,
    was_running: bool,
    /// How many times the main CPU started the sub-CPU from reset.
    starts: u32,
    run_clocks: u64,
    /// Debugging aid: the main CPU's writes to the gate array, `(sub clocks due, address, size, value)`.
    main_log: Option<Vec<(u64, u32, Size, u32)>>,
    /// Debugging aid: the last sub-CPU instruction addresses, frozen when the sub-CPU reaches an address in `trace_stop`.
    trace: Option<SubTrace>,
    /// The PCM chip's and the CD audio's samples waiting for the output (fact C22).
    pcm_audio: audio::Resampler,
    cd_audio: audio::Resampler,
    /// Scratch for the PCM chip's samples.
    pcm_scratch: Vec<(i16, i16)>,
    /// Debugging aid: PCM samples that were not silent and CD audio frames produced so far.
    activity: (u64, u64),
}

/// One traced sub-CPU instruction: address, data registers, address registers, status register.
pub type TraceEntry = (u32, [u32; 8], [u32; 7], u16);

struct SubTrace {
    ring: std::collections::VecDeque<TraceEntry>,
    stop_at: Vec<u32>,
    frozen: bool,
}

/// Everything of the Mega-CD that changes while it runs, for a save state. Not in it: the BIOS and the disc (the state names the
/// BIOS by CRC and the disc by its track layout and refuses another), the debugging aids, and a stop that was already reported.
#[derive(Serialize, Deserialize)]
struct SavedExp {
    bios_crc: u32,
    master_hz: u64,
    prg: Vec<u8>,
    word: Vec<u8>,
    backup: Vec<u8>,
    gate: GateArray,
    pcm: rf5c164::Pcm,
    asic: mcd_asic::Asic,
    clock: u64,
    hung: bool,
    pc: u32,
    sub: Cpu,
    last_master: u64,
    remainder: u64,
    due: u64,
    spent: u64,
    was_running: bool,
    starts: u32,
    run_clocks: u64,
    pcm_audio: audio::Resampler,
    cd_audio: audio::Resampler,
    activity: (u64, u64),
}

impl SegaCdExp {
    pub fn new(bios: Vec<u8>, master_hz: u64) -> Result<Self, String> {
        if bios.len() != BIOS_SIZE {
            return Err(format!(
                "the Sega CD BIOS must be {BIOS_SIZE} bytes, this one is {}",
                bios.len()
            ));
        }
        Ok(Self {
            side: SubSide {
                bios,
                prg: vec![0; PRG_RAM_SIZE].into_boxed_slice(),
                word: vec![0; WORD_RAM_SIZE].into_boxed_slice(),
                backup: vec![0; BACKUP_RAM_SIZE].into_boxed_slice(),
                gate: GateArray::new(),
                pcm: rf5c164::Pcm::new(),
                asic: mcd_asic::Asic::new(),
                clock: 0,
                hung: false,
                stop: None,
                pc: 0,
                io_log: None,
                io_filter: (0, 0xFF_FFFF),
                watch: (1, 0),
            },
            sub: Cpu::new(),
            master_hz,
            last_master: 0,
            remainder: 0,
            due: 0,
            spent: 0,
            was_running: false,
            starts: 0,
            run_clocks: 0,
            main_log: None,
            trace: None,
            pcm_audio: audio::Resampler::new(SUB_CLOCK_HZ / rf5c164::CLOCKS_PER_SAMPLE),
            cd_audio: audio::Resampler::new(44_100),
            pcm_scratch: Vec::new(),
            activity: (0, 0),
        })
    }

    /// Debugging aid: how many PCM samples were not silent and how many CD audio frames were played so far.
    pub fn audio_activity(&self) -> (u64, u64) {
        self.activity
    }

    /// The 8 KB of internal backup RAM (the sub-CPU sees it on odd addresses only, fact C2).
    pub fn backup_ram(&self) -> &[u8] {
        &self.side.backup
    }

    /// Loads the backup RAM from a saved image (shorter images fill the start, longer ones are cut).
    pub fn load_backup_ram(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(BACKUP_RAM_SIZE);
        self.side.backup[..n].copy_from_slice(&bytes[..n]);
    }

    /// Puts a disc into the drive (fact E14).
    pub fn insert_disc(&mut self, disc: disc::Disc) {
        self.side
            .gate
            .set_drive(Box::new(drive::DiscDrive::new(disc)));
    }

    /// Sub-CPU clocks that have come due since power on.
    pub fn sub_clocks_due(&self) -> u64 {
        self.due
    }

    /// Sub-CPU clocks it actually ran instructions for (the rest it spent halted or waiting).
    pub fn sub_clocks_run(&self) -> u64 {
        self.run_clocks
    }

    /// Starts logging the main CPU's gate array writes (a debugging aid).
    pub fn log_main_io(&mut self) {
        self.main_log = Some(Vec::new());
    }

    pub fn main_io_log(&self) -> &[(u64, u32, Size, u32)] {
        self.main_log.as_deref().unwrap_or(&[])
    }

    /// Starts logging the sub-CPU's register writes (a debugging aid).
    /// Also logs the sub-CPU's writes to this address range (a debugging aid).
    pub fn watch_sub_writes(&mut self, from: u32, to: u32) {
        self.side.watch = (from, to);
    }

    pub fn log_sub_io(&mut self) {
        self.side.io_log = Some(Vec::new());
    }

    /// Debugging aid: only the sub-CPU accesses to `from..=to` go into the log.
    pub fn filter_sub_io(&mut self, from: u32, to: u32) {
        self.side.io_filter = (from, to);
    }

    pub fn sub_io_log(&self) -> &[(u32, u32, Size, u32)] {
        self.side.io_log.as_deref().unwrap_or(&[])
    }

    /// The first trace vector lines of the graphics unit's last operation (a debugging aid).
    pub fn asic_trace(&self) -> &[[u16; 4]] {
        self.side.asic.last_trace()
    }

    pub fn sub_starts(&self) -> u32 {
        self.starts
    }

    pub fn sub_cpu(&self) -> &Cpu {
        &self.sub
    }

    pub fn gate_array(&self) -> &GateArray {
        &self.side.gate
    }

    pub fn prg_ram(&self) -> &[u8] {
        &self.side.prg
    }

    pub fn word_ram(&self) -> &[u8] {
        &self.side.word
    }

    /// A byte of the sub-CPU's address space as the debugger sees it: no side effects, `$FF` where nothing is mapped or the
    /// RAM belongs to the other CPU.
    pub fn sub_peek(&self, addr: u32) -> u8 {
        let addr = sub_address(addr);
        match addr {
            0..=0x07_FFFF => self.side.prg[addr as usize],
            0x08_0000..=0x0B_FFFF
                if !self.side.gate.mode_1m()
                    && self.side.gate.word_ram_owner() == WordRamOwner::Sub =>
            {
                self.side.word[(addr - 0x08_0000) as usize]
            }
            0xFE_0000..=0xFE_3FFF if addr & 1 == 1 => {
                self.side.backup[((addr - 0xFE_0000) >> 1) as usize]
            }
            _ => 0xFF,
        }
    }

    /// The sub-CPU waits forever on word RAM it does not own (fact C7).
    pub fn sub_hung(&self) -> bool {
        self.side.hung
    }

    fn main_byte(&mut self, addr: u32, side_effects: bool) -> Option<u8> {
        let addr = alias(addr);
        let gate = &self.side.gate;
        match addr {
            0..=0x01_FFFF => Some(self.side.bios[addr as usize]),
            0x02_0000..=0x03_FFFF => {
                if !gate.sub_bus_granted() {
                    return Some(0); // the sub-CPU owns program RAM: nothing answers usefully (fact C1)
                }
                let at = usize::from(gate.prg_bank()) * 0x2_0000 + (addr & 0x1_FFFF) as usize;
                Some(self.side.prg[at])
            }
            // 1M mode: the main CPU's bank (128 KB); the cell image view above it is not modelled
            0x20_0000..=0x21_FFFF if gate.mode_1m() => {
                Some(self.side.word[gate.main_bank_1m() * 0x2_0000 + (addr - 0x20_0000) as usize])
            }
            0x22_0000..=0x23_FFFF if gate.mode_1m() => {
                let at = cell_image_source(addr - 0x22_0000);
                Some(self.side.word[gate.main_bank_1m() * 0x2_0000 + at])
            }
            0x20_0000..=0x23_FFFF => {
                if gate.mode_1m() {
                    return None;
                }
                if gate.word_ram_owner() == WordRamOwner::Main {
                    Some(self.side.word[(addr - 0x20_0000) as usize])
                } else {
                    Some(0xFF) // the sub-CPU has it: the main CPU reads nothing sensible ([H?])
                }
            }
            0xA1_2000..=0xA1_20FF => {
                if side_effects {
                    self.side.gate.main_read_byte(addr - 0xA1_2000)
                } else {
                    self.side.gate.clone().main_read_byte(addr - 0xA1_2000)
                }
            }
            _ => None,
        }
    }

    fn main_write_byte(&mut self, addr: u32, v: u8) -> bool {
        let addr = alias(addr);
        match addr {
            0..=0x01_FFFF => true, // the BIOS is ROM
            0x02_0000..=0x03_FFFF => {
                let gate = &self.side.gate;
                if gate.sub_bus_granted() {
                    let sub_addr = (u32::from(gate.prg_bank()) * 0x2_0000) | (addr & 0x1_FFFF);
                    if !gate.prg_write_protected(sub_addr) {
                        self.side.prg[sub_addr as usize] = v;
                    }
                }
                true
            }
            0x20_0000..=0x21_FFFF if self.side.gate.mode_1m() => {
                let bank = self.side.gate.main_bank_1m();
                self.side.word[bank * 0x2_0000 + (addr - 0x20_0000) as usize] = v;
                true
            }
            0x22_0000..=0x23_FFFF if self.side.gate.mode_1m() => {
                let bank = self.side.gate.main_bank_1m();
                let at = cell_image_source(addr - 0x22_0000);
                self.side.word[bank * 0x2_0000 + at] = v;
                true
            }
            0x20_0000..=0x23_FFFF => {
                let gate = &self.side.gate;
                if gate.mode_1m() {
                    return false;
                }
                if gate.word_ram_owner() == WordRamOwner::Main {
                    self.side.word[(addr - 0x20_0000) as usize] = v;
                }
                true
            }
            0xA1_2000..=0xA1_20FF => {
                let ok = self.side.gate.main_write_byte(addr - 0xA1_2000, v);
                self.after_main_register_write();
                ok
            }
            _ => false,
        }
    }

    /// The sub-CPU starts from its reset vectors when the main CPU lets it run (fact C4).
    fn after_main_register_write(&mut self) {
        let running = !self.side.gate.sub_in_reset();
        if running && !self.was_running {
            self.side.hung = false;
            self.side.pc = 0;
            self.starts += 1;
            self.sub.reset(&mut self.side);
        }
        if !running {
            self.side.hung = false;
        }
        self.was_running = running;
    }

    /// Starts recording the sub-CPU's instruction addresses (with registers); recording freezes when it reaches one of `stop_at`.
    pub fn trace_sub(&mut self, stop_at: Vec<u32>) {
        self.trace = Some(SubTrace {
            ring: std::collections::VecDeque::new(),
            stop_at,
            frozen: false,
        });
    }

    /// The recorded trace: address, data registers, address registers and SR before each instruction.
    pub fn sub_trace(&self) -> Option<(bool, Vec<TraceEntry>)> {
        self.trace
            .as_ref()
            .map(|t| (t.frozen, t.ring.iter().copied().collect()))
    }

    fn run_sub(&mut self) {
        while self.spent < self.due {
            if self.side.stop.is_some() || self.side.hung || self.side.gate.sub_halted() {
                // time passes without instructions
                self.spent = self.due;
                return;
            }
            self.side.pc = self.sub.pc;
            self.side.clock = self.spent;
            if let Some(t) = self.trace.as_mut()
                && !t.frozen
            {
                if t.stop_at.contains(&self.sub.pc) {
                    t.frozen = true;
                } else {
                    if t.ring.len() >= 20_000 {
                        t.ring.pop_front();
                    }
                    t.ring
                        .push_back((self.sub.pc, self.sub.d, self.sub.a, self.sub.sr()));
                }
            }
            self.sub
                .set_ipl(self.side.gate.sub_irq_level().unwrap_or(0));
            let clocks = self.sub.step(&mut self.side);
            self.spent += u64::from(clocks.max(4));
            self.run_clocks += u64::from(clocks.max(4));
            if self.side.asic.take_finished(self.spent) {
                self.side.gate.raise_level(1);
            }
        }
    }
}

/// The cartridge-boot (mode 1) windows of fact C16 show the same memory as the CD-boot map: the BIOS at `$400000`, the program
/// RAM window at `$420000`, word RAM at `$600000`.
fn alias(addr: u32) -> u32 {
    match addr {
        0x40_0000..=0x43_FFFF => addr - 0x40_0000,
        // word RAM repeats every 256 KB up to the end of its megabyte, in both places (fact C21)
        0x24_0000..=0x3F_FFFF | 0x60_0000..=0x7F_FFFF => 0x20_0000 | (addr & 0x3_FFFF),
        _ => addr,
    }
}

impl Expansion for SegaCdExp {
    fn claims(&self, addr: u32) -> bool {
        addr < 0x80_0000 || (0xA1_2000..=0xA1_20FF).contains(&addr)
    }

    fn read(&mut self, addr: u32, size: Size, now: u64) -> Option<u32> {
        self.advance(now);
        let value = if (0xA1_2000..=0xA1_20FF).contains(&addr) && size == Size::Word {
            self.side
                .gate
                .main_read_word(addr - 0xA1_2000)
                .map(u32::from)
        } else {
            match size {
                Size::Byte => self.main_byte(addr, true).map(u32::from),
                Size::Word => {
                    let hi = self.main_byte(addr, true)?;
                    let lo = self.main_byte(addr | 1, true)?;
                    Some(u32::from(hi) << 8 | u32::from(lo))
                }
            }
        };
        // debugging aid: the main CPU's reads of the flag and status words are logged too, marked by bit 31 of the address
        if let Some(v) = value
            && (0xA1_200E..=0xA1_202F).contains(&addr)
            && let Some(log) = self.main_log.as_mut()
            && log.len() < 400_000
        {
            log.push((self.due, addr | 0x8000_0000, size, v));
        }
        value
    }

    fn write(&mut self, addr: u32, size: Size, value: u32, now: u64) -> bool {
        self.advance(now);
        if (0xA1_2000..=0xA1_20FF).contains(&addr)
            && let Some(log) = self.main_log.as_mut()
            && log.len() < 400_000
        {
            log.push((self.due, addr, size, value));
        }
        if (0xA1_2000..=0xA1_20FF).contains(&addr) && size == Size::Word {
            let ok = self
                .side
                .gate
                .main_write_word(addr - 0xA1_2000, value as u16);
            self.after_main_register_write();
            return ok;
        }
        match size {
            Size::Byte => self.main_write_byte(addr, value as u8),
            Size::Word => {
                self.main_write_byte(addr, (value >> 8) as u8)
                    && self.main_write_byte(addr | 1, value as u8)
            }
        }
    }

    fn dma_read_word(&mut self, addr: u32) -> Option<u16> {
        // A VDP DMA from word RAM gets each word one cycle late: the word it delivers is the one before the address it asked
        // for, which is why software starts such a transfer one word further on (fact C17).
        let mut addr = addr;
        if (0x20_0000..=0x23_FFFF).contains(&alias(addr)) && alias(addr) > 0x20_0000 {
            addr -= 2;
        }
        let hi = self.main_byte(addr, false)?;
        let lo = self.main_byte(addr | 1, false)?;
        Some(u16::from(hi) << 8 | u16::from(lo))
    }

    fn peek(&self, addr: u32) -> u8 {
        let copy_free = |a: u32| -> u8 {
            let a = alias(a);
            match a {
                0..=0x01_FFFF => self.side.bios[a as usize],
                0x02_0000..=0x03_FFFF => {
                    let at =
                        usize::from(self.side.gate.prg_bank()) * 0x2_0000 + (a & 0x1_FFFF) as usize;
                    self.side.prg[at]
                }
                0x20_0000..=0x23_FFFF => self.side.word[(a - 0x20_0000) as usize],
                _ => 0xFF,
            }
        };
        copy_free(addr)
    }

    fn advance(&mut self, now: u64) {
        if now <= self.last_master {
            return;
        }
        let delta = now - self.last_master;
        self.last_master = now;
        // sub clocks = master clocks x 12.5 MHz / master Hz, with the fraction carried (exact integers)
        let scaled = self.remainder + delta * SUB_CLOCK_HZ;
        let clocks = scaled / self.master_hz;
        self.remainder = scaled % self.master_hz;
        self.due += clocks;
        self.side.gate.advance_sub_clocks(clocks);
        self.run_sub();
        self.side.pcm.advance(clocks, &mut self.pcm_scratch);
        for (l, r) in self.pcm_scratch.drain(..) {
            self.activity.0 += u64::from((l, r) != (0, 0));
            self.pcm_audio
                .push(f32::from(l) / 32768.0, f32::from(r) / 32768.0);
        }
        let cd = self.side.gate.take_cd_audio();
        self.activity.1 += (cd.len() / 2) as u64;
        for lr in cd.as_chunks::<2>().0 {
            self.cd_audio
                .push(f32::from(lr[0]) / 32768.0, f32::from(lr[1]) / 32768.0);
        }
    }

    fn mix_audio(&mut self, out: &mut [f32], frame_clocks: u64) {
        for frame in out.as_chunks_mut::<2>().0 {
            let (pl, pr) = self.pcm_audio.next(frame_clocks, self.master_hz);
            let (cl, cr) = self.cd_audio.next(frame_clocks, self.master_hz);
            frame[0] += pl + cl;
            frame[1] += pr + cr;
        }
    }

    fn stop_report(&self) -> Option<String> {
        self.side.stop.clone()
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        let saved = SavedExp {
            bios_crc: crc32fast::hash(&self.side.bios),
            master_hz: self.master_hz,
            prg: self.side.prg.to_vec(),
            word: self.side.word.to_vec(),
            backup: self.side.backup.to_vec(),
            gate: self.side.gate.snapshot(),
            pcm: self.side.pcm.clone(),
            asic: self.side.asic.clone(),
            clock: self.side.clock,
            hung: self.side.hung,
            pc: self.side.pc,
            sub: self.sub.clone(),
            last_master: self.last_master,
            remainder: self.remainder,
            due: self.due,
            spent: self.spent,
            was_running: self.was_running,
            starts: self.starts,
            run_clocks: self.run_clocks,
            pcm_audio: self.pcm_audio.clone(),
            cd_audio: self.cd_audio.clone(),
            activity: self.activity,
        };
        Some(
            bincode::serde::encode_to_vec(&saved, bincode::config::standard())
                .expect("a state always encodes"),
        )
    }

    fn load_state(&mut self, bytes: &[u8], now: u64) -> Result<(), String> {
        let (saved, used): (SavedExp, usize) =
            bincode::serde::decode_from_slice(bytes, bincode::config::standard())
                .map_err(|e| format!("the Mega-CD state is damaged: {e}"))?;
        if used != bytes.len() {
            return Err("the Mega-CD state is damaged: trailing bytes".to_string());
        }
        if saved.bios_crc != crc32fast::hash(&self.side.bios) {
            return Err("the state was saved with another Sega CD BIOS".to_string());
        }
        if saved.master_hz != self.master_hz {
            return Err("the state was saved for another region".to_string());
        }
        if saved.prg.len() != PRG_RAM_SIZE
            || saved.word.len() != WORD_RAM_SIZE
            || saved.backup.len() != BACKUP_RAM_SIZE
        {
            return Err("the Mega-CD state has a RAM of the wrong size".to_string());
        }
        saved.pcm.validate()?;
        saved.asic.validate()?;
        saved
            .pcm_audio
            .validate(SUB_CLOCK_HZ / rf5c164::CLOCKS_PER_SAMPLE)?;
        saved.cd_audio.validate(44_100)?;
        // the sub-CPU finishes the instruction it started, so it can be a few clocks ahead of what is due
        if saved.last_master > now
            || saved.spent > saved.due.saturating_add(4096)
            || saved.remainder >= self.master_hz
        {
            return Err("the Mega-CD state has its clocks out of order".to_string());
        }
        // the gate array is the one part that can still refuse (the drive): it goes first, so nothing changes on an error
        self.side.gate.restore(saved.gate)?;
        self.side.prg.copy_from_slice(&saved.prg);
        self.side.word.copy_from_slice(&saved.word);
        self.side.backup.copy_from_slice(&saved.backup);
        self.side.pcm = saved.pcm;
        self.side.asic = saved.asic;
        self.side.clock = saved.clock;
        self.side.hung = saved.hung;
        self.side.stop = None;
        self.side.pc = saved.pc;
        self.sub = saved.sub;
        self.last_master = saved.last_master;
        self.remainder = saved.remainder;
        self.due = saved.due;
        self.spent = saved.spent;
        self.was_running = saved.was_running;
        self.starts = saved.starts;
        self.run_clocks = saved.run_clocks;
        self.pcm_audio = saved.pcm_audio;
        self.cd_audio = saved.cd_audio;
        self.activity = saved.activity;
        self.pcm_scratch.clear();
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine::Machine;

    const NTSC: u64 = 53_693_175;

    fn exp() -> SegaCdExp {
        SegaCdExp::new(vec![0; BIOS_SIZE], NTSC).unwrap()
    }

    fn write_words(e: &mut SegaCdExp, base: u32, words: &[u16]) {
        for (i, w) in words.iter().enumerate() {
            assert!(e.write(
                base + 2 * i as u32,
                Size::Word,
                u32::from(*w),
                e.last_master
            ));
        }
    }

    /// Starts the sub-CPU with vectors SSP = `$7F000`, PC = `$100` and `code` at `$100`.
    fn start_sub(e: &mut SegaCdExp, code: &[u16]) {
        write_words(e, 0x02_0000, &[0x0007, 0xF000, 0x0000, 0x0100]);
        write_words(e, 0x02_0100, code);
        assert!(e.write(0xA1_2001, Size::Byte, 0x01, e.last_master));
    }

    fn run_master(e: &mut SegaCdExp, master_clocks: u64) {
        let t = e.last_master + master_clocks;
        e.advance(t);
    }

    /// A machine whose sub-CPU runs a counting loop: `$100: ADDQ.W #1,$2000.L ; BRA.S $100`-ish, enough to change RAM and PC.
    fn running() -> SegaCdExp {
        let mut e = exp();
        // 52 79 00 00 20 00: ADDQ.W #1,($2000).L  then 60 F6: BRA.S back
        start_sub(&mut e, &[0x5279, 0x0000, 0x2000, 0x60F6]);
        run_master(&mut e, 200_000);
        e
    }

    #[test]
    fn a_saved_expansion_resumes_exactly_in_a_fresh_one() {
        let mut a = running();
        let blob = a.save_state().unwrap();
        let mut b = exp();
        b.load_state(&blob, a.last_master).unwrap();
        assert_eq!(
            b.save_state().unwrap(),
            blob,
            "loading then saving gives the same bytes"
        );
        run_master(&mut a, 400_000);
        run_master(&mut b, 400_000);
        assert_eq!(a.save_state().unwrap(), b.save_state().unwrap());
        assert_ne!(a.prg_ram()[0x2000..0x2002], [0, 0], "the loop ran");
    }

    #[test]
    fn a_state_of_another_bios_region_or_time_is_refused_and_changes_nothing() {
        let a = running();
        let blob = a.save_state().unwrap();
        let mut other_bios = vec![0u8; BIOS_SIZE];
        other_bios[7] = 1;
        let mut b = SegaCdExp::new(other_bios, NTSC).unwrap();
        let before = b.save_state().unwrap();
        assert!(
            b.load_state(&blob, a.last_master)
                .unwrap_err()
                .contains("BIOS")
        );
        let mut pal = SegaCdExp::new(vec![0; BIOS_SIZE], 53_203_424).unwrap();
        assert!(
            pal.load_state(&blob, a.last_master)
                .unwrap_err()
                .contains("region")
        );
        let mut c = exp();
        assert!(
            c.load_state(&blob, 0).unwrap_err().contains("clocks"),
            "a state from the future"
        );
        assert_eq!(b.save_state().unwrap(), before);
    }

    #[test]
    fn damaged_expansion_states_never_panic_and_never_half_load() {
        let a = running();
        let blob = a.save_state().unwrap();
        let mut target = exp();
        let before = target.save_state().unwrap();
        for len in (0..blob.len()).step_by(997) {
            assert!(
                target.load_state(&blob[..len], a.last_master).is_err(),
                "cut at {len}"
            );
        }
        assert_eq!(
            target.save_state().unwrap(),
            before,
            "refusals change nothing"
        );
        // flipped bytes either load (and the machine keeps running) or are refused: never a panic
        let mut seed = 0x1234_5678u32;
        for _ in 0..300 {
            let mut bad = blob.clone();
            for _ in 0..3 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let at = (seed >> 8) as usize % bad.len();
                bad[at] ^= 1 << (seed & 7);
            }
            let mut t = exp();
            if t.load_state(&bad, a.last_master).is_ok() {
                run_master(&mut t, 100_000);
            }
        }
    }

    fn cd_machine() -> Genesis {
        new_machine(SegaCdConfig::new(vec![0; BIOS_SIZE], Region::Usa)).unwrap()
    }

    #[test]
    fn a_whole_mega_cd_machine_saves_and_resumes_and_refuses_plain_genesis_states() {
        let advance = |m: &mut Genesis, clocks: u64| {
            let mut spent = 0u64;
            while spent < clocks {
                spent += u64::from(m.step());
            }
        };
        let mut a = cd_machine();
        advance(&mut a, 3_000_000);
        let state = a.save_state();
        let mut b = cd_machine();
        b.load_state(&state).unwrap();
        advance(&mut a, 3_000_000);
        advance(&mut b, 3_000_000);
        assert_eq!(a.save_state(), b.save_state());

        // a Genesis without the Mega-CD neither accepts this state nor makes one this machine accepts
        let mut plain = Genesis::new(Config::new(vec![0u8; 0x400], Region::Usa)).unwrap();
        assert!(
            plain
                .load_state(&state)
                .unwrap_err()
                .contains("cartridge port")
        );
        let plain_state = plain.save_state();
        assert!(
            b.load_state(&plain_state)
                .unwrap_err()
                .contains("cartridge port")
        );
    }

    #[test]
    fn the_cartridge_boot_windows_show_the_same_memory() {
        let mut bios = vec![0u8; BIOS_SIZE];
        bios[0x100..0x104].copy_from_slice(b"SEGA");
        let mut e = SegaCdExp::new(bios, NTSC).unwrap();
        assert_eq!(e.read(0x40_0100, Size::Word, 0), Some(0x5345));
        assert_eq!(e.read(0x00_0100, Size::Word, 0), Some(0x5345));
        assert!(e.write(0x42_0010, Size::Word, 0xABCD, 0));
        assert_eq!(e.prg_ram()[0x10], 0xAB);
        assert!(e.write(0x60_0000, Size::Word, 0x1234, 0));
        assert_eq!(e.word_ram()[1], 0x34);
        assert_eq!(e.read(0x20_0000, Size::Word, 0), Some(0x1234));
        // the rest of the cartridge space is not answered
        assert_eq!(e.read(0x50_0000, Size::Word, 0), None);
        assert_eq!(e.peek(0x40_0100), b'S');
    }

    #[test]
    fn a_dma_from_word_ram_delivers_each_word_one_cycle_late() {
        let mut e = exp();
        for (i, w) in [0x1111u32, 0x2222, 0x3333].iter().enumerate() {
            assert!(e.write(0x20_0000 + 2 * i as u32, Size::Word, *w, 0));
        }
        // asking for word 2 gives word 1; the cartridge-boot window behaves the same
        assert_eq!(e.dma_read_word(0x20_0004), Some(0x2222));
        assert_eq!(e.dma_read_word(0x60_0004), Some(0x2222));
        // the BIOS area is not delayed
        assert_eq!(e.dma_read_word(0x00_0004), Some(0));
    }

    #[test]
    fn a_cd_controller_transfer_to_word_ram_in_one_megabit_mode_fills_the_sub_cpus_bank() {
        use mcd_gate_array::cdc::Destination;
        let mut e = exp();
        // 1M mode, RET = 0: the main CPU has bank 0, so the sub-CPU's is bank 1
        assert!(e.side.gate.sub_write_word(0x02, 0x0004));
        e.side.put_dma_bytes(Destination::WordRam, 0x10, &[1, 2, 3]);
        assert_eq!(&e.word_ram()[0x2_0010..0x2_0013], &[1, 2, 3]);
        assert_eq!(&e.word_ram()[0x10..0x13], &[0, 0, 0]);
        // RET = 1 hands the main CPU bank 1: the next transfer fills bank 0, and the address wraps inside a bank
        assert!(e.side.gate.sub_write_word(0x02, 0x0005));
        e.side
            .put_dma_bytes(Destination::WordRam, 0x2_0000 + 4, &[9]);
        assert_eq!(e.word_ram()[4], 9);
        // in 2M mode it is one memory
        assert!(e.side.gate.sub_write_word(0x02, 0x0000));
        e.side.put_dma_bytes(Destination::WordRam, 0x3_0000, &[7]);
        assert_eq!(e.word_ram()[0x3_0000], 7);
    }

    #[test]
    fn one_megabit_mode_gives_each_cpu_a_bank_and_the_dot_and_cell_views_show_it() {
        let mut e = exp();
        // the sub-CPU switches to 1M mode (MODE = bit 2 of the memory mode register): bank 0 is the main CPU's
        assert!(e.side.gate.sub_write_word(0x02, 0x0004));
        assert!(e.write(0x20_0000, Size::Word, 0xABCD, 0));
        assert_eq!(e.read(0x20_0000, Size::Word, 0), Some(0xABCD));
        // the cell image: bitmap line 1 (256 bytes on) is the second row of 4 bytes of cell 0 of column 0
        assert!(e.write(0x20_0100, Size::Word, 0x1122, 0));
        assert_eq!(e.read(0x22_0004, Size::Word, 0), Some(0x1122));
        // the second column of 8 dots (bytes 4-7 of a line) starts the next cell column, 32 cells further on
        assert!(e.write(0x20_0004, Size::Word, 0x5566, 0));
        assert_eq!(e.read(0x22_0400, Size::Word, 0), Some(0x5566));
        // the sub-CPU's bank is the other one: a write through its dot image lands in bank 1
        let mut side = &mut e.side;
        assert!(side.sub_write_byte(0x08_0000, 0x7));
        assert!(side.sub_write_byte(0x08_0001, 0x3));
        assert_eq!(side.word[0x2_0000], 0x73);
        assert_eq!(side.sub_byte(0x08_0000), Some(7));
        assert_eq!(side.sub_byte(0x0C_0000), Some(0x73));
        // a swap: the main CPU asks, the sub-CPU answers by writing RET
        side = &mut e.side;
        assert!(side.gate.main_write_byte(3, 0x02));
        assert_eq!(
            side.gate.main_read_byte(3).unwrap() & 3,
            0b10,
            "request pending"
        );
        assert!(side.gate.sub_write_word(0x02, 0x0005));
        assert_eq!(
            side.gate.main_read_byte(3).unwrap() & 3,
            0b01,
            "done, bank 1 is the main CPU's"
        );
        assert_eq!(
            side.sub_byte(0x0C_0000),
            Some(0xAB),
            "and the sub-CPU has what the main CPU wrote"
        );
    }

    /// Sets up channel 1 of the PCM chip to play a constant `+100` byte at full level and starts the chip.
    fn start_pcm_tone(e: &mut SegaCdExp) {
        let pcm = &mut e.side.pcm;
        pcm.write(7, 0);
        for i in 0..16 {
            pcm.write(0x1000 + i, 0x80 | 100);
        }
        pcm.write(0x1010 - 1, 0xFF);
        pcm.write(7, 0x40);
        for (a, v) in [(0, 0xFF), (1, 0xFF), (2, 0), (3, 0), (6, 0)] {
            pcm.write(a, v);
        }
        pcm.write(8, 0xFE);
        pcm.write(7, 0x80);
    }

    #[test]
    fn the_pcm_chip_and_the_cd_audio_are_added_to_the_genesis_sound() {
        let mut e = exp();
        start_pcm_tone(&mut e);
        // a tenth of a second of the master clock, taken as output samples of 1008 clocks
        e.advance(NTSC / 10);
        let frames = (NTSC / 10 / 1008) as usize;
        let mut out = vec![0.25f32; frames * 2];
        e.mix_audio(&mut out, 1008);
        let expect = 0.25 + ((100 * 255 * 15) >> 5) as f32 / 32768.0;
        assert!(
            (out[out.len() - 1] - expect).abs() < 1e-4,
            "{}",
            out[out.len() - 1]
        );
        assert!((out[out.len() - 2] - expect).abs() < 1e-4);
        // the CD audio, 44.1 kHz stereo: left 0.5 and right -0.5
        let mut e = exp();
        for _ in 0..4410 {
            e.cd_audio.push(0.5, -0.5);
        }
        let mut out = vec![0.0f32; 2 * 100];
        e.mix_audio(&mut out, 1008);
        assert_eq!((out[198], out[199]), (0.5, -0.5));
    }

    #[test]
    fn a_pcm_chip_that_is_off_adds_nothing() {
        let mut e = exp();
        e.advance(NTSC / 10);
        let mut out = vec![0.0f32; 2 * 1000];
        e.mix_audio(&mut out, 1008);
        assert!(out.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn a_bios_of_the_wrong_size_is_refused() {
        assert!(SegaCdExp::new(vec![0; 100], NTSC).is_err());
        assert!(SegaCdExp::new(vec![0; BIOS_SIZE], NTSC).is_ok());
    }

    #[test]
    fn the_sub_clock_ratio_is_exact() {
        let mut e = exp();
        e.advance(NTSC);
        assert_eq!(e.sub_clocks_due(), 12_500_000);
        // in small steps the total is the same to the clock
        let mut e = exp();
        let mut t = 0;
        while t < NTSC {
            t = (t + 28).min(NTSC);
            e.advance(t);
        }
        assert_eq!(e.sub_clocks_due(), 12_500_000);
        // PAL
        let mut p = SegaCdExp::new(vec![0; BIOS_SIZE], 53_203_424).unwrap();
        p.advance(53_203_424);
        assert_eq!(p.sub_clocks_due(), 12_500_000);
    }

    #[test]
    fn the_sub_cpu_runs_a_program_the_main_cpu_copied() {
        let mut e = exp();
        // ADDQ.W #1,$000010 ; BRA.S back
        start_sub(&mut e, &[0x5279, 0x0000, 0x0010, 0x60F8]);
        run_master(&mut e, 200_000);
        let counted = u16::from_be_bytes([e.prg_ram()[0x10], e.prg_ram()[0x11]]);
        assert!(counted > 50, "{counted}");
        assert!(e.stop_report().is_none());
        // the stopwatch ran with it
        assert!(e.gate_array().clone().main_read_word(0x0C).unwrap() > 0);
    }

    #[test]
    fn the_main_cpu_cannot_touch_program_ram_while_the_sub_cpu_runs() {
        let mut e = exp();
        start_sub(&mut e, &[0x60FE]);
        run_master(&mut e, 1000);
        assert_eq!(e.read(0x02_0100, Size::Word, e.last_master), Some(0));
        assert!(e.write(0x02_0100, Size::Word, 0x1234, e.last_master));
        assert_eq!(e.prg_ram()[0x100], 0x60, "unchanged");
        // with the bus requested it is readable again
        assert!(e.write(0xA1_2001, Size::Byte, 0x03, e.last_master));
        assert_eq!(e.read(0x02_0100, Size::Word, e.last_master), Some(0x60FE));
    }

    #[test]
    fn write_protect_and_banks_apply_to_main_writes() {
        let mut e = exp();
        assert!(e.write(0xA1_2002, Size::Word, 0x0100 | 0x0040, e.last_master)); // WP = 1 ($200 bytes), bank 1
        assert!(e.write(0x02_0000, Size::Word, 0xAAAA, e.last_master));
        assert!(e.write(0x02_0200, Size::Word, 0xBBBB, e.last_master));
        // bank 1 is sub address $20000.., protected below $200 more than the first $200: sub $20000 is above $200
        assert_eq!(e.prg_ram()[0x2_0000], 0xAA);
        assert!(e.write(0xA1_2002, Size::Word, 0xFF00, e.last_master)); // WP = 255, bank 0
        assert!(e.write(0x02_0100, Size::Word, 0xCCCC, e.last_master));
        assert_eq!(e.prg_ram()[0x100], 0, "protected");
    }

    #[test]
    fn an_unmapped_sub_address_stops_with_a_report_naming_the_sub_cpu() {
        let mut e = exp();
        // MOVE.W $0F0000,D0
        start_sub(&mut e, &[0x3039, 0x000E, 0x4000, 0x60FE]);
        run_master(&mut e, 10_000);
        let report = e.stop_report().unwrap();
        assert!(
            report.contains("sub-CPU") && report.contains("$FE4000"),
            "{report}"
        );
        assert!(report.contains("pc $000100"), "{report}");
    }

    #[test]
    fn the_sub_cpu_hangs_on_word_ram_it_does_not_own_and_nothing_else_stops() {
        let mut e = exp();
        // MOVE.W $080000,D0 ; ADDQ.W #1,$10 ; BRA.S
        start_sub(
            &mut e,
            &[0x3039, 0x0008, 0x0000, 0x5279, 0x0000, 0x0010, 0x60F8],
        );
        run_master(&mut e, 100_000);
        assert!(e.sub_hung());
        assert!(e.stop_report().is_none());
        assert_eq!(e.prg_ram()[0x11], 0, "never got past the access");
        // a forced reset frees it
        assert!(e.write(0xA1_2001, Size::Byte, 0x00, e.last_master));
        assert!(!e.sub_hung());
    }

    #[test]
    fn the_sub_cpu_uses_word_ram_once_the_main_cpu_hands_it_over() {
        let mut e = exp();
        assert!(e.write(0x20_0000, Size::Word, 0x1234, e.last_master)); // main owns it at power on
        assert_eq!(e.word_ram()[1], 0x34);
        assert!(e.write(0xA1_2003, Size::Byte, 0x02, e.last_master)); // DMNA: give it to the sub-CPU
        // MOVE.W $080000,D0 ; MOVE.W D0,$000010 ; BRA.S *
        start_sub(
            &mut e,
            &[0x3039, 0x0008, 0x0000, 0x33C0, 0x0000, 0x0010, 0x60FE],
        );
        run_master(&mut e, 10_000);
        assert!(!e.sub_hung());
        assert_eq!((e.prg_ram()[0x10], e.prg_ram()[0x11]), (0x12, 0x34));
    }

    #[test]
    fn level_2_from_the_main_cpu_runs_the_sub_handler() {
        let mut e = exp();
        // vector 26 (level 2 autovector) at $68 -> $200
        write_words(&mut e, 0x02_0068, &[0x0000, 0x0200]);
        // handler: ADDQ.W #1,$20 ; RTE
        write_words(&mut e, 0x02_0200, &[0x5279, 0x0000, 0x0020, 0x4E73]);
        // main: MOVE.W #4,$FF8032 ; ANDI #$F8FF,SR ; BRA.S *  (the sub's own code)
        start_sub(
            &mut e,
            &[0x33FC, 0x0004, 0x00FF, 0x8032, 0x027C, 0xF8FF, 0x60FE],
        );
        run_master(&mut e, 20_000);
        assert_eq!(e.prg_ram()[0x21], 0, "not before the main CPU asks");
        assert!(e.write(0xA1_2000, Size::Byte, 0x01, e.last_master)); // IFL2
        run_master(&mut e, 20_000);
        assert_eq!(e.prg_ram()[0x21], 1, "ran once");
        run_master(&mut e, 20_000);
        assert_eq!(e.prg_ram()[0x21], 1, "and not again");
    }

    #[test]
    fn the_bios_answers_the_main_cpu_at_zero_and_a_machine_boots_from_it() {
        // a BIOS whose reset vectors start `MOVE.W #$BEEF,$FF0000 ; BRA.S *` at $200
        let mut bios = vec![0u8; BIOS_SIZE];
        bios[0..4].copy_from_slice(&0x00FF_FE00u32.to_be_bytes());
        bios[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        for (i, w) in [0x33FCu16, 0xBEEF, 0x00FF, 0x0000, 0x60FE]
            .iter()
            .enumerate()
        {
            bios[0x200 + 2 * i..0x202 + 2 * i].copy_from_slice(&w.to_be_bytes());
        }
        let mut m = new_machine(SegaCdConfig::new(bios, Region::Usa)).unwrap();
        for _ in 0..10 {
            m.step();
        }
        assert_eq!(
            (m.read_memory(0xFF_0000), m.read_memory(0xFF_0001)),
            (0xBE, 0xEF)
        );
        assert!(!m.halted());
        // an address the Mega-CD does not answer stops the machine through the usual report
        let mut stop = vec![0u8; BIOS_SIZE];
        stop[0..4].copy_from_slice(&0x00FF_FE00u32.to_be_bytes());
        stop[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        for (i, w) in [0x3039u16, 0x0050, 0x0000, 0x60FE].iter().enumerate() {
            stop[0x200 + 2 * i..0x202 + 2 * i].copy_from_slice(&w.to_be_bytes());
        }
        let mut m = new_machine(SegaCdConfig::new(stop, Region::Usa)).unwrap();
        m.step();
        assert!(m.halted());
        assert!(m.take_stop_report().unwrap().contains("$500000"));
    }

    #[test]
    fn the_sub_cpu_is_driven_by_main_clock_through_the_genesis_bus() {
        // the main CPU: copy a sub program through the window, run it, then spin
        let mut bios = vec![0u8; BIOS_SIZE];
        bios[0..4].copy_from_slice(&0x00FF_FE00u32.to_be_bytes());
        bios[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        let main: [u16; 28] = [
            0x23FC, 0x0007, 0xF000, 0x0002, 0x0000, // MOVE.L #$7F000,$020000
            0x23FC, 0x0000, 0x0100, 0x0002, 0x0004, // MOVE.L #$100,$020004
            0x33FC, 0x5279, 0x0002, 0x0100, // MOVE.W #$5279,$020100
            0x33FC, 0x0000, 0x0002, 0x0102, // MOVE.W #0,$020102
            0x33FC, 0x0010, 0x0002, 0x0104, // MOVE.W #$10,$020104
            0x33FC, 0x60F8, 0x0002, 0x0106, // MOVE.W #$60F8,$020106
            0x13FC, 0x0001, // MOVE.B #1,$A12001 ... address follows
        ];
        let mut words = main.to_vec();
        words.extend([0x00A1, 0x2001, 0x60FE]);
        for (i, w) in words.iter().enumerate() {
            bios[0x200 + 2 * i..0x202 + 2 * i].copy_from_slice(&w.to_be_bytes());
        }
        let mut m = new_machine(SegaCdConfig::new(bios, Region::Usa)).unwrap();
        let mut spent = 0u64;
        while spent < 400_000 {
            spent += u64::from(m.step());
        }
        assert!(!m.halted(), "{:?}", m.take_stop_report());
        // the sub counter is in program RAM; read it the way the debugger does (bank 0 window, bus not granted: peek ignores that)
        let hi = m.bus().peek(0x02_0010);
        let lo = m.bus().peek(0x02_0011);
        assert!(u16::from_be_bytes([hi, lo]) > 10, "{hi:#x}{lo:#x}");
    }
}
