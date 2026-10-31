//! The Mega-CD gate array: the registers both CPUs see (main side at `$A12000`, sub side at `$FF8000`), the ownership of word
//! RAM, program RAM banking and write protection, the mailboxes, the stopwatch, the timer and the sub-CPU's interrupt levels.
//!
//! Facts C1-C15 in `docs/hardware-facts.md` (source: Mega-CD Hardware Manual v1.0). Registers that belong to the CD
//! controller, the drive, the graphics unit and the sub-code are not here: they return `None` ("no such register") until
//! their slices, and the machine stops with a report, as the project rule says for an unknown address.
//!
//! Word RAM in 1M/1M mode is not modelled (fact C8 is unresolved): the mode bit can be set and `mode_1m()` tells the machine,
//! which stops with a report when the RAM is touched.
#![forbid(unsafe_code)]

pub mod cdc;
pub mod cdd;

use serde::{Deserialize, Serialize};

/// Sub-CPU clocks per tick of the stopwatch and the timer (30.72 us at 12.5 MHz, fact C10).
pub const CLOCKS_PER_TICK: u64 = 384;

/// Who has the 256 KB of word RAM in 2M mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WordRamOwner {
    Main,
    Sub,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GateArray {
    run: bool,
    sbrq: bool,
    ifl2: bool,
    wp: u8,
    bank: u8,
    mode_1m: bool,
    owner: WordRamOwner,
    /// 1M mode: which bank the main CPU has (RET: 0 gives it bank 0, 1 bank 1; the sub-CPU has the other, fact C8).
    ret_1m: bool,
    /// 1M mode: the main CPU asked for the banks to be swapped and the sub-CPU has not done it yet.
    swap_request: bool,
    pm: u8,
    hint_vector: u16,
    flag_main: u8,
    flag_sub: u8,
    commands: [u16; 8],
    status: [u16; 8],
    stopwatch: u16,
    tick_acc: u64,
    timer_reload: u8,
    timer_count: u8,
    mask: u8,
    pending: u8,
    /// Progress through the forced reset sequence of fact C5 (0 = not armed).
    reset_step: u8,
    cdd: cdd::Cdd,
    cdc: cdc::Cdc,
    dma_address: u16,
    /// Fader register of the audio output (fact E1): stored, not used until the audio slice.
    fader: u16,
}

impl Default for GateArray {
    fn default() -> Self {
        Self::new()
    }
}

impl GateArray {
    /// A copy for a save state (the drive's position included, not the drive).
    pub fn snapshot(&self) -> GateArray {
        let mut copy = self.clone();
        copy.cdd = self.cdd.snapshot();
        copy
    }

    /// Whether a state's values are ones the gate array can have (a state is untrusted input).
    pub fn validate(&self) -> Result<(), String> {
        if self.bank > 3 || self.pm > 3 || self.reset_step > 3 {
            return Err("a Mega-CD register holds a value it cannot".to_string());
        }
        self.cdc.validate()?;
        self.cdd.validate()
    }

    /// Takes a snapshot, keeping the plugged-in drive. Nothing changes on an error.
    pub fn restore(&mut self, mut saved: GateArray) -> Result<(), String> {
        saved.validate()?;
        let cdd = std::mem::take(&mut saved.cdd);
        let mut this_cdd = std::mem::take(&mut self.cdd);
        if let Err(e) = this_cdd.adopt(cdd) {
            self.cdd = this_cdd;
            return Err(e);
        }
        *self = GateArray {
            cdd: this_cdd,
            ..saved
        };
        Ok(())
    }

    /// Power-on state (fact C6, C4, C9, C12): sub-CPU in reset, 2M mode, word RAM with the main CPU.
    pub fn new() -> GateArray {
        GateArray {
            run: false,
            sbrq: false,
            ifl2: false,
            wp: 0,
            bank: 0,
            mode_1m: false,
            owner: WordRamOwner::Main,
            ret_1m: false,
            swap_request: false,
            pm: 0,
            hint_vector: 0xFFFF,
            flag_main: 0,
            flag_sub: 0,
            commands: [0; 8],
            status: [0; 8],
            stopwatch: 0,
            tick_acc: 0,
            timer_reload: 0,
            timer_count: 0,
            mask: 0,
            pending: 0,
            reset_step: 0,
            cdd: cdd::Cdd::new(),
            cdc: cdc::Cdc::new(),
            dma_address: 0xFFFF, // fact E21
            fader: 0,
        }
    }

    /// The forced reset (fact C5): everything but the main side's own registers goes back to its power-on value.
    fn reset_forced(&mut self) {
        let (wp, bank, hint, cmds) = (self.wp, self.bank, self.hint_vector, self.commands);
        *self = GateArray::new();
        self.wp = wp;
        self.bank = bank;
        self.hint_vector = hint;
        self.commands = cmds;
    }

    // ------------------------------------------------------------------ state for the buses

    /// The sub-CPU does not run (reset, or its bus given to the main CPU).
    /// The DMA address register as written (fact C13), for the DMA paths of slice 3b.
    pub fn dma_address(&self) -> u16 {
        self.dma_address
    }

    /// Raises level 5 on a rising edge of the CD controller's interrupt output.
    fn poll_cdc_int(&mut self) {
        if self.cdc.int_edge() {
            self.pending |= 1 << 5;
        }
    }

    /// A sector the drive hands over, for the owner of the drive (fact E14).
    pub fn cdc_mut(&mut self) -> &mut cdc::Cdc {
        &mut self.cdc
    }

    /// A DMA transfer the CD controller has ready, to be written into memory the gate array does not own.
    pub fn take_cdc_dma(&mut self) -> Option<cdc::Dma> {
        let dma = self.cdc.take_dma();
        self.poll_cdc_int();
        dma
    }

    /// The DMA address register moves on by the bytes a transfer wrote (it counts in units of 8 bytes).
    pub fn advance_dma_address(&mut self, bytes: usize) {
        self.dma_address = self.dma_address.wrapping_add((bytes / 8) as u16);
    }

    /// The commands and answers of the drive port so far (a debugging aid).
    /// The CD audio the drive played since the last call (interleaved left and right samples at 44.1 kHz).
    pub fn take_cd_audio(&mut self) -> Vec<i16> {
        self.cdd.take_audio()
    }

    pub fn cdd_log(&self) -> &[([u8; 10], [u8; 10])] {
        self.cdd.log()
    }

    /// Plugs a drive into the CDD port.
    pub fn set_drive(&mut self, drive: Box<dyn cdd::Drive>) {
        self.cdd.set_drive(drive);
    }

    pub fn sub_halted(&self) -> bool {
        !self.run || self.sbrq
    }

    pub fn sub_in_reset(&self) -> bool {
        !self.run
    }

    /// The main CPU may use program RAM (fact C1): the sub-CPU is in reset or has granted its bus.
    pub fn sub_bus_granted(&self) -> bool {
        !self.run || self.sbrq
    }

    /// Program RAM bank (0-3) the main CPU sees at `$020000`.
    pub fn prg_bank(&self) -> u8 {
        self.bank
    }

    /// Program RAM `addr` (sub-CPU address) cannot be written by the main CPU (fact C2: the first `WP x $200` bytes).
    pub fn prg_write_protected(&self, addr: u32) -> bool {
        addr < u32::from(self.wp) * 0x200
    }

    /// The write priority mode bits of the memory mode register: 0 off, 1 underwrite, 2 overwrite (fact C6).
    pub fn priority_mode(&self) -> u8 {
        self.pm
    }

    /// 1M mode: the bank (0 or 1) the main CPU has; the sub-CPU has the other.
    pub fn main_bank_1m(&self) -> usize {
        usize::from(self.ret_1m)
    }

    pub fn mode_1m(&self) -> bool {
        self.mode_1m
    }

    /// Who has word RAM in 2M mode.
    pub fn word_ram_owner(&self) -> WordRamOwner {
        self.owner
    }

    pub fn hint_vector(&self) -> u16 {
        self.hint_vector
    }

    // ------------------------------------------------------------------ main side

    fn read_a12000(&self) -> u16 {
        let ien2 = u16::from(self.mask & 0x04 != 0);
        let ifl2 = u16::from(self.ifl2);
        let granted = u16::from(self.sub_bus_granted());
        (ien2 << 15) | (ifl2 << 8) | (granted << 1) | u16::from(self.run)
    }

    fn memory_mode_low(&self) -> u8 {
        let (dmna, ret) = if self.mode_1m {
            (u8::from(self.swap_request), u8::from(self.ret_1m))
        } else {
            match self.owner {
                WordRamOwner::Main => (0, 1),
                WordRamOwner::Sub => (1, 0),
            }
        };
        (self.bank << 6) | (u8::from(self.mode_1m) << 2) | (dmna << 1) | ret
    }

    /// Reads a main-side word; `off` is the address minus `$A12000` (even). `None`: no such register.
    pub fn main_read_word(&mut self, off: u32) -> Option<u16> {
        let off = off & !1;
        Some(match off {
            0x00 => self.read_a12000(),
            0x02 => (u16::from(self.wp) << 8) | u16::from(self.memory_mode_low()),
            0x04 => self.cdc.mode_word(),
            0x08 => {
                let w = self.cdc.host_word(cdc::Destination::MainRead).unwrap_or(0);
                self.poll_cdc_int();
                w
            }
            0x06 => self.hint_vector,
            0x0C => self.stopwatch,
            0x0E => (u16::from(self.flag_main) << 8) | u16::from(self.flag_sub),
            0x10..=0x1E => self.commands[((off - 0x10) / 2) as usize],
            0x20..=0x2E => self.status[((off - 0x20) / 2) as usize],
            _ => return None,
        })
    }

    pub fn main_read_byte(&mut self, off: u32) -> Option<u8> {
        let w = self.main_read_word(off)?;
        Some(if off & 1 == 0 {
            (w >> 8) as u8
        } else {
            w as u8
        })
    }

    /// Writes a main-side byte. Returns `false` for an address with no register.
    pub fn main_write_byte(&mut self, off: u32, v: u8) -> bool {
        // the forced reset is three byte writes to $A12001 right after the word $FF00 to $A12002 (fact C5)
        if self.reset_step != 0 {
            let expect = [0x03, 0x02, 0x00][usize::from(self.reset_step - 1)];
            if off == 1 && v == expect {
                self.reset_step += 1;
                if self.reset_step == 4 {
                    self.reset_forced();
                }
                return true;
            }
            self.reset_step = 0;
        }
        match off {
            0x00 => {
                // bit 0 of the high byte is IFL2: write 1 raises level 2 on the sub side
                if v & 1 != 0 {
                    self.ifl2 = true;
                    self.pending |= 1 << 2;
                }
                true
            }
            0x01 => {
                self.run = v & 1 != 0;
                self.sbrq = v & 2 != 0;
                true
            }
            0x02 => {
                self.wp = v;
                true
            }
            0x03 => {
                self.bank = v >> 6;
                // DMNA (bit 1): writing 1 gives the 2M RAM to the sub-CPU (fact C7)
                if v & 2 != 0 {
                    if self.mode_1m {
                        self.swap_request = true; // 1M mode: a request for the sub-CPU to swap the banks (fact C8)
                    } else {
                        self.owner = WordRamOwner::Sub;
                    }
                }
                true
            }
            0x06 => {
                self.hint_vector = (self.hint_vector & 0x00FF) | (u16::from(v) << 8);
                true
            }
            0x07 => {
                self.hint_vector = (self.hint_vector & 0xFF00) | u16::from(v);
                true
            }
            // the high byte of the flag word is the main CPU's (CFM); the low byte is the sub-CPU's and is ignored here
            // either byte address writes the main CPU's flags: the strobes of the two halves are not decoded (fact C9)
            0x0E | 0x0F => {
                self.flag_main = v;
                true
            }
            0x10..=0x1F => {
                let i = ((off - 0x10) / 2) as usize;
                let w = self.commands[i];
                self.commands[i] = if off & 1 == 0 {
                    (w & 0x00FF) | (u16::from(v) << 8)
                } else {
                    (w & 0xFF00) | u16::from(v)
                };
                true
            }
            // read-only here, or not a register: stopwatch, status words are not writable by the main CPU
            0x0C..=0x0D | 0x20..=0x2F => true,
            _ => false,
        }
    }

    /// Writes a main-side word as the two byte writes it is, except for the reset word `$FF00` to `$A12002`.
    pub fn main_write_word(&mut self, off: u32, v: u16) -> bool {
        if off == 0x02 && v == 0xFF00 {
            // an exact word write arms the forced reset; it is also an ordinary write of the register
            let ok = self.main_write_byte(0x02, 0xFF) && self.main_write_byte(0x03, 0x00);
            self.reset_step = 1;
            return ok;
        }
        if off & !1 == 0x0E {
            self.flag_main = (v >> 8) as u8; // a word write sets the main flags from its high byte
            return true;
        }
        let ok_hi = self.main_write_byte(off & !1, (v >> 8) as u8);
        let ok_lo = self.main_write_byte(off | 1, v as u8);
        ok_hi && ok_lo
    }

    // ------------------------------------------------------------------ sub side

    /// Reads a sub-side word; `off` is the address minus `$FF8000` (even). `None`: no such register (yet).
    pub fn sub_read_word(&mut self, off: u32) -> Option<u16> {
        let off = off & !1;
        Some(match off {
            // reset register: peripherals operational at once, version nibble 0, LEDs off (fact C15, simplified)
            0x00 => 0x0001,
            0x02 => {
                (u16::from(self.wp) << 8)
                    | (u16::from(self.pm) << 3)
                    | u16::from(self.memory_mode_low() & 0x07)
            }
            0x0C => self.stopwatch,
            0x0E => (u16::from(self.flag_main) << 8) | u16::from(self.flag_sub),
            0x10..=0x1E => self.commands[((off - 0x10) / 2) as usize],
            0x20..=0x2E => self.status[((off - 0x20) / 2) as usize],
            0x30 => u16::from(self.timer_reload),
            0x32 => u16::from(self.mask),
            0x04 => self.cdc.mode_word(),
            // the DMA address reads back, advanced by the transfers (fact E21)
            0x0A => self.dma_address,
            // the data port: reading it steps the address register, so only a word access (or the low byte) counts
            0x06 => {
                let v = u16::from(self.cdc.read()?);
                self.poll_cdc_int();
                v
            }
            // the host data port: the next word of a transfer to the sub-CPU (zero when nothing waits)
            0x08 => {
                let w = self.cdc.host_word(cdc::Destination::SubRead).unwrap_or(0);
                self.poll_cdc_int();
                w
            }
            0x34 => self.fader & 0x7FFF,
            0x36 => u16::from(self.cdd.data_mode()) << 8 | u16::from(self.cdd.hock()) << 2,
            0x38..=0x40 => {
                let i = ((off - 0x38) / 2 * 2) as usize;
                u16::from(self.cdd.rs(i)) << 8 | u16::from(self.cdd.rs(i + 1))
            }
            0x42..=0x4A => {
                let i = ((off - 0x42) / 2 * 2) as usize;
                u16::from(self.cdd.tc(i)) << 8 | u16::from(self.cdd.tc(i + 1))
            }
            _ => return None,
        })
    }

    pub fn sub_read_byte(&mut self, off: u32) -> Option<u8> {
        if off == 0x06 {
            return Some(0); // the high byte of the data port is not connected
        }
        let w = self.sub_read_word(off)?;
        Some(if off & 1 == 0 {
            (w >> 8) as u8
        } else {
            w as u8
        })
    }

    /// Writes a sub-side word. Returns `false` for a register that is not there (yet).
    pub fn sub_write_word(&mut self, off: u32, v: u16) -> bool {
        let off = off & !1;
        match off {
            0x00 => true,
            0x02 => {
                self.pm = ((v >> 3) & 3) as u8;
                let mode_1m = v & 0x04 != 0;
                if mode_1m != self.mode_1m {
                    // switching the mode starts with bank 0 at the main CPU and no swap pending; the contents are lost
                    self.ret_1m = false;
                    self.swap_request = false;
                }
                self.mode_1m = mode_1m;
                if self.mode_1m {
                    // 1M mode: RET chooses which bank the main CPU has, and answers a swap request (fact C8)
                    self.ret_1m = v & 1 != 0;
                    self.swap_request = false;
                } else if v & 1 != 0 {
                    // 2M mode: writing 1 gives the RAM back to the main CPU (fact C7)
                    self.owner = WordRamOwner::Main;
                }
                true
            }
            0x0C => {
                self.stopwatch = 0;
                self.tick_acc = 0;
                true
            }
            0x0E => {
                self.flag_sub = v as u8;
                true
            }
            0x10..=0x1E => true, // commands are read only here
            0x20..=0x2E => {
                self.status[((off - 0x20) / 2) as usize] = v;
                true
            }
            0x30 => {
                self.timer_reload = v as u8;
                self.timer_count = v as u8;
                true
            }
            0x32 => {
                self.mask = (v as u8) & 0x7E;
                true
            }
            0x04 => {
                self.cdc.write_mode(v);
                self.dma_address = 0; // fact E21
                true
            }
            0x06 => {
                self.cdc.write(v as u8);
                self.poll_cdc_int();
                true
            }
            0x0A => {
                self.dma_address = v;
                true
            }
            0x34 => {
                self.fader = v;
                true
            }
            0x36 => {
                // HOCK (bit 2) starts the communication: the first status packet and its level 4 interrupt follow at once
                if self.cdd.set_hock(v & 0x04 != 0) {
                    self.cdd.deliver();
                    self.pending |= 1 << 4;
                }
                true
            }
            0x38..=0x40 => true, // received status is read only
            0x42..=0x4A => {
                let i = ((off - 0x42) / 2 * 2) as usize;
                self.cdd.write_tc(i, (v >> 8) as u8);
                self.cdd.write_tc(i + 1, v as u8);
                true
            }
            _ => false,
        }
    }

    pub fn sub_write_byte(&mut self, off: u32, v: u8) -> bool {
        match off {
            0x04 => {
                self.cdc
                    .write_mode(u16::from(v) << 8 | u16::from(self.cdc.address()));
                self.dma_address = 0; // fact E21
                return true;
            }
            0x05 => {
                self.cdc.write_address(v);
                return true;
            }
            0x06 => return true,
            0x07 => {
                self.cdc.write(v);
                self.poll_cdc_int();
                return true;
            }
            0x0A => {
                self.dma_address = (self.dma_address & 0x00FF) | (u16::from(v) << 8);
                return true;
            }
            0x0B => {
                self.dma_address = (self.dma_address & 0xFF00) | u16::from(v);
                return true;
            }
            _ => {}
        }
        match self.sub_read_word(off) {
            None => false,
            Some(w) => {
                let w = if off & 1 == 0 {
                    (w & 0x00FF) | (u16::from(v) << 8)
                } else {
                    (w & 0xFF00) | u16::from(v)
                };
                if (0x42..=0x4B).contains(&off) {
                    self.cdd.write_tc((off - 0x42) as usize, v);
                    return true;
                }
                if (0x38..=0x41).contains(&off) {
                    return true;
                }
                // the flag word is split by owner: the sub-CPU can only change its own (low) byte
                if off & !1 == 0x0E {
                    self.flag_sub = v;
                    return true;
                }
                self.sub_write_word(off, w)
            }
        }
    }

    // ------------------------------------------------------------------ time and interrupts

    /// Advances the stopwatch and the timer by `clocks` sub-CPU clocks.
    pub fn advance_sub_clocks(&mut self, clocks: u64) {
        for _ in 0..self.cdd.advance(clocks) {
            self.cdd.drive_period(&mut self.cdc);
            if self.cdd.hock() {
                self.cdd.deliver();
                self.pending |= 1 << 4;
            }
        }
        self.poll_cdc_int();
        self.tick_acc += clocks;
        let mut ticks = self.tick_acc / CLOCKS_PER_TICK;
        self.tick_acc %= CLOCKS_PER_TICK;
        self.stopwatch = ((u64::from(self.stopwatch) + ticks) & 0x0FFF) as u16;
        if self.timer_reload == 0 {
            return;
        }
        // the count goes reload .. 0 and the tick after 0 raises level 3: a period of reload + 1 ticks (fact C11)
        while ticks > 0 {
            let c = u64::from(self.timer_count);
            if ticks > c {
                ticks -= c + 1;
                self.pending |= 1 << 3;
                self.timer_count = self.timer_reload;
            } else {
                self.timer_count -= ticks as u8;
                ticks = 0;
            }
        }
    }

    /// Raises a sub-CPU interrupt level (1, 4, 5 or 6 from the units that own them; 2 and 3 are raised inside).
    pub fn raise_level(&mut self, level: u8) {
        if (1..=6).contains(&level) {
            self.pending |= 1 << level;
        }
    }

    /// The highest pending level the mask enables, if any (fact C3).
    pub fn sub_irq_level(&self) -> Option<u8> {
        let live = self.pending & self.mask;
        (1..=6).rev().find(|l| live & (1 << l) != 0)
    }

    /// The CPU took `level`: clear it.
    pub fn ack_sub_irq(&mut self, level: u8) {
        if (1..=6).contains(&level) {
            self.pending &= !(1 << level);
            if level == 2 {
                self.ifl2 = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_sub(g: &mut GateArray) {
        g.main_write_byte(1, 0x01);
    }

    #[test]
    fn power_on_values() {
        let mut g = GateArray::new();
        assert_eq!(g.main_read_word(0x00), Some(0x0002)); // in reset, bus granted
        assert_eq!(g.main_read_word(0x02), Some(0x0001)); // 2M mode, word RAM with main
        assert_eq!(g.sub_read_word(0x02), Some(0x0001));
        assert_eq!(g.main_read_word(0x06), Some(0xFFFF));
        assert_eq!(g.main_read_word(0x0E), Some(0));
        assert_eq!(g.sub_read_word(0x30), Some(0));
        assert!(g.sub_halted() && g.sub_bus_granted());
    }

    #[test]
    fn main_a12000_bits() {
        let mut g = GateArray::new();
        run_sub(&mut g);
        assert_eq!(g.main_read_word(0x00).unwrap() & 3, 0b01); // running, bus not granted
        assert!(!g.sub_halted());
        g.main_write_byte(1, 0x03); // run + request the bus
        assert_eq!(g.main_read_word(0x00).unwrap() & 3, 0b11);
        assert!(g.sub_halted() && g.sub_bus_granted());
        g.main_write_byte(1, 0x00);
        assert!(g.sub_in_reset());
    }

    #[test]
    fn level_2_needs_ifl2_and_the_mask_and_clears_when_taken() {
        let mut g = GateArray::new();
        g.main_write_byte(0, 0x01); // IFL2
        assert_eq!(g.sub_irq_level(), None, "masked");
        assert_eq!(g.main_read_word(0x00).unwrap() & 0x8100, 0x0100);
        g.sub_write_word(0x32, 0x0004);
        assert_eq!(
            g.main_read_word(0x00).unwrap() & 0x8000,
            0x8000,
            "IEN2 shows the mask"
        );
        assert_eq!(g.sub_irq_level(), Some(2));
        g.ack_sub_irq(2);
        assert_eq!(g.sub_irq_level(), None);
        assert_eq!(g.main_read_word(0x00).unwrap() & 0x0100, 0);
    }

    #[test]
    fn interrupt_mask_and_priority() {
        let mut g = GateArray::new();
        g.sub_write_word(0x32, 0x007E);
        g.raise_level(4);
        g.raise_level(5);
        g.raise_level(1);
        assert_eq!(g.sub_irq_level(), Some(5));
        g.ack_sub_irq(5);
        assert_eq!(g.sub_irq_level(), Some(4));
        g.sub_write_word(0x32, 0x0002); // only level 1 enabled
        assert_eq!(g.sub_irq_level(), Some(1));
        assert_eq!(g.sub_read_word(0x32), Some(0x0002));
        // writing bit 0 or bit 7 does not enable anything
        g.sub_write_word(0x32, 0x0081);
        assert_eq!(g.sub_read_word(0x32), Some(0));
    }

    #[test]
    fn reset_sequence_must_be_exact() {
        let mut g = GateArray::new();
        g.sub_write_word(0x32, 0x007E);
        g.sub_write_word(0x20, 0x1234);
        // the right sequence
        assert!(g.main_write_word(0x02, 0xFF00));
        g.main_write_byte(1, 0x03);
        g.main_write_byte(1, 0x02);
        g.main_write_byte(1, 0x00);
        assert_eq!(g.sub_read_word(0x32), Some(0), "sub side was reset");
        assert_eq!(g.sub_read_word(0x20), Some(0));
        // wrong value, wrong order, and a missing arming word do not reset
        for seq in [[0x03u8, 0x00, 0x02], [0x02, 0x03, 0x00], [0x03, 0x02, 0x01]] {
            let mut g = GateArray::new();
            g.sub_write_word(0x32, 0x007E);
            g.main_write_word(0x02, 0xFF00);
            for b in seq {
                g.main_write_byte(1, b);
            }
            assert_eq!(g.sub_read_word(0x32), Some(0x7E), "{seq:?}");
        }
        let mut g = GateArray::new();
        g.sub_write_word(0x32, 0x007E);
        for b in [0x03, 0x02, 0x00] {
            g.main_write_byte(1, b);
        }
        assert_eq!(g.sub_read_word(0x32), Some(0x7E), "not armed");
        // a byte write of $FF00 (two bytes) does not arm it
        let mut g = GateArray::new();
        g.sub_write_word(0x32, 0x007E);
        g.main_write_byte(2, 0xFF);
        g.main_write_byte(3, 0x00);
        for b in [0x03, 0x02, 0x00] {
            g.main_write_byte(1, b);
        }
        assert_eq!(g.sub_read_word(0x32), Some(0x7E), "bytes do not arm");
    }

    #[test]
    fn flag_word_halves() {
        let mut g = GateArray::new();
        g.main_write_word(0x0E, 0xAB12); // main owns the high byte only
        assert_eq!(g.main_read_word(0x0E), Some(0xAB00));
        g.sub_write_word(0x0E, 0xCD34); // the sub-CPU owns the low byte only
        assert_eq!(g.sub_read_word(0x0E), Some(0xAB34));
        g.sub_write_byte(0x0E, 0x55); // a write to either byte address sets the sub-CPU's flags
        assert_eq!(g.main_read_word(0x0E), Some(0xAB55));
        g.main_write_byte(0x0F, 0x66); // and the same for the main CPU's
        assert_eq!(g.main_read_word(0x0E), Some(0x6655));
        g.main_write_byte(0x0E, 0xAB);
        g.sub_write_byte(0x0F, 0x01);
        assert_eq!(g.main_read_byte(0x0F), Some(0x01));
    }

    #[test]
    fn command_and_status_words_go_one_way_each() {
        let mut g = GateArray::new();
        g.main_write_word(0x10, 0x1111);
        g.main_write_word(0x1E, 0x7777);
        g.main_write_word(0x20, 0x9999); // not writable by main
        g.sub_write_word(0x10, 0x5555); // not writable by sub
        g.sub_write_word(0x2E, 0x3333);
        assert_eq!(g.sub_read_word(0x10), Some(0x1111));
        assert_eq!(g.sub_read_word(0x1E), Some(0x7777));
        assert_eq!(g.main_read_word(0x20), Some(0));
        assert_eq!(g.main_read_word(0x2E), Some(0x3333));
        g.main_write_byte(0x11, 0x22);
        assert_eq!(g.main_read_word(0x10), Some(0x1122));
    }

    #[test]
    fn stopwatch_counts_and_wraps() {
        let mut g = GateArray::new();
        g.advance_sub_clocks(383);
        assert_eq!(g.main_read_word(0x0C), Some(0));
        g.advance_sub_clocks(1);
        assert_eq!(g.main_read_word(0x0C), Some(1));
        g.advance_sub_clocks(384 * 4094);
        assert_eq!(g.sub_read_word(0x0C), Some(4095));
        g.advance_sub_clocks(384);
        assert_eq!(g.sub_read_word(0x0C), Some(0), "wraps");
        g.advance_sub_clocks(384 * 5);
        g.sub_write_word(0x0C, 0);
        assert_eq!(g.main_read_word(0x0C), Some(0));
    }

    #[test]
    fn timer_counts_down_and_interrupts_level_3() {
        let mut g = GateArray::new();
        g.sub_write_word(0x32, 0x0008);
        g.advance_sub_clocks(384 * 100);
        assert_eq!(g.sub_irq_level(), None, "n = 0: no interrupt");
        g.sub_write_word(0x30, 3); // period 4 ticks
        g.advance_sub_clocks(384 * 3);
        assert_eq!(g.sub_irq_level(), None);
        g.advance_sub_clocks(384);
        assert_eq!(g.sub_irq_level(), Some(3));
        g.ack_sub_irq(3);
        g.advance_sub_clocks(384 * 4);
        assert_eq!(
            g.sub_irq_level(),
            Some(3),
            "periodic (to be confirmed by a probe)"
        );
        g.ack_sub_irq(3);
        g.sub_write_word(0x30, 255);
        g.advance_sub_clocks(384 * 255);
        assert_eq!(g.sub_irq_level(), None);
        g.advance_sub_clocks(384);
        assert_eq!(g.sub_irq_level(), Some(3));
        assert_eq!(
            g.sub_read_word(0x30),
            Some(255),
            "a read gives the value written"
        );
    }

    #[test]
    fn word_ram_2m_handover() {
        let mut g = GateArray::new();
        assert_eq!(g.word_ram_owner(), WordRamOwner::Main);
        g.main_write_byte(3, 0x02); // DMNA
        assert_eq!(g.word_ram_owner(), WordRamOwner::Sub);
        assert_eq!(g.main_read_byte(3).unwrap() & 3, 0b10);
        assert_eq!(g.sub_read_word(0x02).unwrap() & 3, 0b10);
        g.sub_write_word(0x02, 0x0001); // RET
        assert_eq!(g.word_ram_owner(), WordRamOwner::Main);
        assert_eq!(g.main_read_byte(3).unwrap() & 3, 0b01);
        // the main CPU cannot take it back by itself, nor the sub give it by writing DMNA
        g.main_write_byte(3, 0x02);
        g.main_write_byte(3, 0x00);
        assert_eq!(g.word_ram_owner(), WordRamOwner::Sub);
        g.sub_write_word(0x02, 0x0002);
        assert_eq!(g.word_ram_owner(), WordRamOwner::Sub);
    }

    #[test]
    fn mode_bit_is_set_by_the_sub_cpu_and_read_by_both() {
        let mut g = GateArray::new();
        assert!(!g.mode_1m());
        g.sub_write_word(0x02, 0x0004);
        assert!(g.mode_1m());
        assert_eq!(g.main_read_byte(3).unwrap() & 4, 4);
    }

    #[test]
    fn prg_bank_and_write_protect() {
        let mut g = GateArray::new();
        g.main_write_word(0x02, 0x00C0); // bank bits are bits 7-6 of the low byte
        assert_eq!(g.prg_bank(), 3);
        assert_eq!(g.main_read_byte(3).unwrap() >> 6, 3);
        g.main_write_byte(2, 4); // 4 x $200 = $800 protected
        assert!(g.prg_write_protected(0x7FF));
        assert!(!g.prg_write_protected(0x800));
        g.main_write_byte(2, 0xFF);
        assert!(g.prg_write_protected(0x1FDFF));
        assert!(!g.prg_write_protected(0x1FE00));
        assert_eq!(g.sub_read_word(0x02).unwrap() >> 8, 0xFF);
    }

    #[test]
    fn unknown_registers_are_not_invented() {
        let mut g = GateArray::new();
        for off in [0x4C, 0x58, 0x100, 0x1FE] {
            assert_eq!(g.sub_read_word(off), None, "{off:#x}");
            assert!(!g.sub_write_word(off, 0), "{off:#x}");
        }
        assert_eq!(g.main_read_word(0x40), None);
        assert!(!g.main_write_byte(0x40, 0));
    }

    fn cdd_status(g: &mut GateArray) -> [u8; 10] {
        let mut rs = [0u8; 10];
        for (i, n) in rs.iter_mut().enumerate() {
            *n = g.sub_read_byte(0x38 + i as u32).unwrap();
        }
        rs
    }

    #[test]
    fn the_cdc_dma_address_reads_back_and_a_write_of_the_mode_register_clears_it() {
        let mut g = GateArray::new();
        assert_eq!(g.sub_read_word(0x0A), Some(0xFFFF), "power-on value");
        assert!(g.sub_write_word(0x0A, 0x1234));
        assert_eq!(g.sub_read_word(0x0A), Some(0x1234));
        assert_eq!(g.sub_read_byte(0x0B), Some(0x34));
        g.advance_dma_address(2048);
        assert_eq!(g.sub_read_word(0x0A), Some(0x1234 + 256));
        assert!(g.sub_write_word(0x04, 0x0300));
        assert_eq!(g.sub_read_word(0x0A), Some(0));
        assert!(g.sub_write_word(0x0A, 0x0042));
        assert!(g.sub_write_byte(0x04, 0x03));
        assert_eq!(g.sub_read_word(0x0A), Some(0));
    }

    #[test]
    fn the_cdd_checksum_is_the_inverted_sum_of_nine_nibbles() {
        assert_eq!(cdd::checksum(&[0; 10]), 0xF);
        assert_eq!(cdd::checksum(&[0xB, 0, 0, 0, 0, 0, 0, 0, 0, 0]), 0x4);
        assert_eq!(
            cdd::checksum(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 0]),
            (!45u32 & 0xF) as u8
        );
        // the tenth nibble is not part of the sum
        assert_eq!(cdd::checksum(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 7]), 0xE);
    }

    #[test]
    fn the_cdd_starts_with_an_all_zero_packet_and_reports_no_disc_after_the_first_command() {
        let mut g = GateArray::new();
        let rs = cdd_status(&mut g);
        assert_eq!(rs, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0xF], "power-on packet");
        assert_eq!(g.sub_read_word(0x38), Some(0));
        g.sub_write_word(0x32, 0x0010);
        g.sub_write_byte(0x37, 0x04); // HOCK: the first packet is delivered as it is
        assert_eq!(cdd_status(&mut g), rs);
        // a NOP command (all zero, checksum F): the drive answers with its state
        g.sub_write_byte(0x4B, 0x0F);
        g.advance_sub_clocks(166_667);
        let rs = cdd_status(&mut g);
        assert_eq!(rs[0], cdd::status::NO_DISC);
        assert_eq!(rs[9], cdd::checksum(&rs));
        assert_eq!(g.sub_read_word(0x38), Some(0x0B00));
        // with no new command the same packet comes again
        g.advance_sub_clocks(166_667);
        assert_eq!(cdd_status(&mut g), rs);
    }

    #[test]
    fn host_clock_starts_the_communication_and_raises_level_4_once_per_period() {
        let mut g = GateArray::new();
        g.sub_write_word(0x32, 0x0010); // level 4 enabled
        g.advance_sub_clocks(12_500_000);
        assert_eq!(
            g.sub_irq_level(),
            None,
            "nothing before the host clock runs"
        );
        g.sub_write_byte(0x37, 0x04); // HOCK
        assert_eq!(
            g.sub_irq_level(),
            Some(4),
            "the first packet arrives at once"
        );
        assert_eq!(
            g.sub_read_word(0x36),
            Some(0x0104),
            "HOCK, and not playing music"
        );
        g.ack_sub_irq(4);
        // 75 periods in a second, exactly, whatever the step size
        let mut count = 0;
        for _ in 0..12_500 {
            g.advance_sub_clocks(1000);
            if let Some(l) = g.sub_irq_level() {
                assert_eq!(l, 4);
                g.ack_sub_irq(4);
                count += 1;
            }
        }
        assert_eq!(count, 75);
        // setting HOCK again does not restart it; clearing it stops the interrupts
        g.sub_write_byte(0x37, 0x00);
        g.advance_sub_clocks(12_500_000);
        assert_eq!(g.sub_irq_level(), None);
    }

    #[test]
    fn the_last_command_nibble_makes_the_drive_answer_at_the_next_period() {
        let mut g = GateArray::new();
        g.sub_write_word(0x32, 0x0010);
        g.sub_write_byte(0x37, 0x04);
        g.ack_sub_irq(4);
        g.sub_write_byte(0x4B, 0x0F); // NOP: the drive reports no disc
        g.advance_sub_clocks(166_667);
        g.ack_sub_irq(4);
        assert_eq!(cdd_status(&mut g)[0], cdd::status::NO_DISC);
        // open the door: TC1 = $D, TC9 = checksum
        g.sub_write_byte(0x42, 0x0D);
        g.sub_write_byte(0x4B, 0x02);
        assert_eq!(
            cdd_status(&mut g)[0],
            cdd::status::NO_DISC,
            "not before the period ends"
        );
        g.advance_sub_clocks(166_667);
        assert_eq!(g.sub_irq_level(), Some(4));
        let rs = cdd_status(&mut g);
        assert_eq!(rs[0], cdd::status::DOOR_OPEN);
        assert_eq!(rs[9], cdd::checksum(&rs));
        g.ack_sub_irq(4);
        // close it again
        g.sub_write_byte(0x42, 0x0C);
        g.sub_write_byte(0x4B, 0x03);
        g.advance_sub_clocks(166_667);
        assert_eq!(cdd_status(&mut g)[0], cdd::status::NO_DISC);
        // command nibbles read back, status is not writable
        assert_eq!(g.sub_read_byte(0x42), Some(0x0C));
        g.sub_write_byte(0x38, 0x07);
        assert_eq!(cdd_status(&mut g)[0], cdd::status::NO_DISC);
    }

    #[test]
    fn h_int_vector_bytes() {
        let mut g = GateArray::new();
        g.main_write_word(0x06, 0x1234);
        assert_eq!(g.hint_vector(), 0x1234);
        g.main_write_byte(0x07, 0xFF);
        assert_eq!(g.main_read_word(0x06), Some(0x12FF));
    }
}
