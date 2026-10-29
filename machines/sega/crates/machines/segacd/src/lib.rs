//! Sega CD / Mega-CD: the second 68000 (the sub-CPU) with its program RAM, word RAM and backup RAM, the gate array, and the
//! main CPU's view of all of it, as an `Expansion` on the Genesis cartridge port (facts C1-C15 in `docs/hardware-facts.md`).
//!
//! The CD drive, the CD controller, the PCM chip and the graphics unit are later slices: the sub-CPU's accesses to their
//! addresses stop the machine with a report, as the project rule says for an address no fact covers.
#![forbid(unsafe_code)]

use genesis::mc68000::{BusFault, Cpu, FunctionCode, IackResponse, M68kBus, Size};
use genesis::{Config, Expansion, Genesis, Region};
use mcd_gate_array::{GateArray, WordRamOwner};
use std::any::Any;

/// The sub-CPU's clock (fact C10).
pub const SUB_CLOCK_HZ: u64 = 12_500_000;
pub const BIOS_SIZE: usize = 0x2_0000;
pub const PRG_RAM_SIZE: usize = 0x8_0000;
pub const WORD_RAM_SIZE: usize = 0x4_0000;
pub const BACKUP_RAM_SIZE: usize = 0x2000;

pub struct SegaCdConfig {
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
            bios,
            region,
            pads: [genesis::PadKind::Six; 2],
            exact_timing: true,
        }
    }
}

/// A Genesis with a Mega-CD on its cartridge port.
pub fn new_machine(config: SegaCdConfig) -> Result<Genesis, String> {
    let exp = SegaCdExp::new(config.bios, config.region.master_hz())?;
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
    /// The sub-CPU touched word RAM it does not own (fact C7): it waits forever, until a forced reset.
    hung: bool,
    stop: Option<String>,
    pc: u32,
    /// Debugging aid: the sub-CPU's writes to the gate array registers, `(pc, address, size, value)`.
    io_log: Option<Vec<(u32, u32, Size, u32)>>,
}

impl SubSide {
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
            return None;
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
            0x08_0000..=0x0B_FFFF => {
                if self.gate.mode_1m() {
                    return false;
                }
                if self.gate.word_ram_owner() != WordRamOwner::Sub {
                    self.hung = true;
                    return true;
                }
                self.word[(addr - 0x08_0000) as usize] = v;
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
        let addr = addr & 0x00FF_FFFF;
        match self.sub_read(addr, size) {
            Some(v) => Ok(v),
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
        let addr = addr & 0x00FF_FFFF;
        if (0xFF_8000..=0xFF_81FF).contains(&addr)
            && let Some(log) = self.io_log.as_mut()
            && log.len() < 4000
        {
            log.push((self.pc, addr, size, value));
        }
        if !self.sub_write(addr, size, value) {
            self.fail("sub-CPU", true, addr, size, value);
        }
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
    /// Debugging aid: the last sub-CPU instruction addresses, frozen when the sub-CPU reaches an address in `trace_stop`.
    trace: Option<SubTrace>,
}

/// One traced sub-CPU instruction: address, data registers, address registers, status register.
pub type TraceEntry = (u32, [u32; 8], [u32; 7], u16);

struct SubTrace {
    ring: std::collections::VecDeque<TraceEntry>,
    stop_at: Vec<u32>,
    frozen: bool,
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
                hung: false,
                stop: None,
                pc: 0,
                io_log: None,
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
            trace: None,
        })
    }

    /// Sub-CPU clocks that have come due since power on.
    pub fn sub_clocks_due(&self) -> u64 {
        self.due
    }

    /// Sub-CPU clocks it actually ran instructions for (the rest it spent halted or waiting).
    pub fn sub_clocks_run(&self) -> u64 {
        self.run_clocks
    }

    /// Starts logging the sub-CPU's register writes (a debugging aid).
    pub fn log_sub_io(&mut self) {
        self.side.io_log = Some(Vec::new());
    }

    pub fn sub_io_log(&self) -> &[(u32, u32, Size, u32)] {
        self.side.io_log.as_deref().unwrap_or(&[])
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
        let addr = addr & 0x00FF_FFFF;
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
            if let Some(t) = self.trace.as_mut()
                && !t.frozen
            {
                if t.stop_at.contains(&self.sub.pc) {
                    t.frozen = true;
                } else {
                    if t.ring.len() >= 200 {
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
        }
    }
}

/// The cartridge-boot (mode 1) windows of fact C16 show the same memory as the CD-boot map: the BIOS at `$400000`, the program
/// RAM window at `$420000`, word RAM at `$600000`.
fn alias(addr: u32) -> u32 {
    match addr {
        0x40_0000..=0x43_FFFF | 0x60_0000..=0x63_FFFF => addr - 0x40_0000,
        _ => addr,
    }
}

impl Expansion for SegaCdExp {
    fn claims(&self, addr: u32) -> bool {
        addr < 0x80_0000 || (0xA1_2000..=0xA1_20FF).contains(&addr)
    }

    fn read(&mut self, addr: u32, size: Size, now: u64) -> Option<u32> {
        self.advance(now);
        if (0xA1_2000..=0xA1_20FF).contains(&addr) && size == Size::Word {
            return self
                .side
                .gate
                .main_read_word(addr - 0xA1_2000)
                .map(u32::from);
        }
        match size {
            Size::Byte => self.main_byte(addr, true).map(u32::from),
            Size::Word => {
                let hi = self.main_byte(addr, true)?;
                let lo = self.main_byte(addr | 1, true)?;
                Some(u32::from(hi) << 8 | u32::from(lo))
            }
        }
    }

    fn write(&mut self, addr: u32, size: Size, value: u32, now: u64) -> bool {
        self.advance(now);
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
    }

    fn stop_report(&self) -> Option<String> {
        self.side.stop.clone()
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
        start_sub(&mut e, &[0x3039, 0x000F, 0x0000, 0x60FE]);
        run_master(&mut e, 10_000);
        let report = e.stop_report().unwrap();
        assert!(
            report.contains("sub-CPU") && report.contains("$0F0000"),
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
        for (i, w) in [0x3039u16, 0x0030, 0x0000, 0x60FE].iter().enumerate() {
            stop[0x200 + 2 * i..0x202 + 2 * i].copy_from_slice(&w.to_be_bytes());
        }
        let mut m = new_machine(SegaCdConfig::new(stop, Region::Usa)).unwrap();
        m.step();
        assert!(m.halted());
        assert!(m.take_stop_report().unwrap().contains("$300000"));
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
