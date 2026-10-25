//! The Z80's side of the machine: its memory map (fact rows Z1-Z6) as `machine::Bus`, and running it up to a master clock.
//!
//! Map: `$0000-$3FFF` the 8 KB RAM twice, `$4000-$5FFF` the YM2612 (not emulated yet: reads 0, writes dropped),
//! `$6000-$60FF` the bank register, `$7F00-$7F1F` the VDP ports and the PSG, `$8000-$FFFF` a window of 32 KB onto the 68000's
//! address space chosen by the bank register. Probe P05: the cartridge shows through the window, the 68000's work RAM does not
//! (reads `$FF`), and the bank register cannot be read.

use crate::bus::Bus;
use crate::config::{M68K_DIVIDER, Z80_DIVIDER};

/// Z80 clocks the Z80 waits for the 68000's bus on average (fact Z13).
const Z80_BUS_WAIT: u64 = 3;
/// Master clocks the 68000 loses for each access of the Z80 to its bus (fact Z13).
const Z80_WINDOW_68K_DELAY: u64 = 10 * M68K_DIVIDER;

impl Bus {
    /// What the Z80 reads at a 68000 address through its window. Unmapped places read `$FF` (the data bus floats high).
    fn z80_window_read(&mut self, addr: u32) -> u8 {
        match addr {
            0..=0x3F_FFFF => self.rom_byte_for_z80(addr),
            0xC0_0000..=0xC0_000F => {
                let word = self.vdp_word_for_z80(addr & !1);
                if addr & 1 == 0 {
                    (word >> 8) as u8
                } else {
                    word as u8
                }
            }
            _ => 0xFF,
        }
    }

    fn z80_window_write(&mut self, addr: u32, value: u8) {
        match addr {
            0xC0_0000..=0xC0_0007 => self.vdp_byte_write_for_z80(addr, value),
            0xC0_0011 | 0xC0_0013 | 0xC0_0015 | 0xC0_0017 => {
                let t = self.z80_access_clock();
                self.sound.psg_write(t, value);
            }
            _ => {}
        }
    }

    /// The Z80's view of its own address space; `t` is the master clock of the access.
    fn z80_read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x3FFF => self.z80.ram[usize::from(addr & 0x1FFF)],
            0x4000..=0x5FFF => {
                let t = self.z80_access_clock();
                self.sound.ym_read(t, u32::from(addr & 3))
            }
            0x6000..=0x7EFF => 0xFF,
            0x7F00..=0x7FFF => {
                let a = 0xC0_0000 | u32::from(addr & 0x1F);
                if a < 0xC0_0010 {
                    self.z80_window_read(a)
                } else {
                    0xFF
                }
            }
            0x8000..=0xFFFF => {
                self.z80_takes_68k_bus();
                let a = u32::from(self.z80.bank) << 15 | u32::from(addr & 0x7FFF);
                self.z80_window_read(a)
            }
        }
    }

    fn z80_write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x3FFF => self.z80.ram[usize::from(addr & 0x1FFF)] = value,
            0x4000..=0x5FFF => {
                let t = self.z80_access_clock();
                self.sound.ym_write(t, u32::from(addr & 3), value);
            }
            0x6000..=0x60FF => self.z80.write_bank(value),
            0x7F00..=0x7FFF => {
                let a = 0xC0_0000 | u32::from(addr & 0x1F);
                self.z80_window_write(a, value);
            }
            0x8000..=0xFFFF => {
                self.z80_takes_68k_bus();
                let a = u32::from(self.z80.bank) << 15 | u32::from(addr & 0x7FFF);
                self.z80_window_write(a, value);
            }
            _ => {}
        }
    }

    /// The Z80 reaches into the 68000's bus (fact Z13): it waits until a VDP DMA from that bus is over and then for the
    /// arbitration, and the 68000 loses cycles while the Z80 holds the bus.
    fn z80_takes_68k_bus(&mut self) {
        if !self.exact {
            return;
        }
        let t = self.z80_access_clock();
        let mut wait = Z80_BUS_WAIT;
        if t < self.dma_hold_until {
            wait += (self.dma_hold_until - t).div_ceil(Z80_DIVIDER);
        }
        self.z80.offset += wait;
        self.z80.wait += wait;
        self.z80_delay += Z80_WINDOW_68K_DELAY;
    }

    /// The master clock of the Z80 access in progress.
    pub(crate) fn z80_access_clock(&self) -> u64 {
        self.z80.time + self.z80.offset * Z80_DIVIDER
    }

    /// Runs the Z80 until its clock reaches `until` (it may overshoot by the instruction in progress). A Z80 that is in
    /// reset or whose bus the 68000 holds loses the time.
    pub fn run_z80(&mut self, until: u64) {
        // A Z80 far behind (a damaged state) loses the time instead of running through all of it.
        if until > self.z80.time.saturating_add(4 * 1_070_460) {
            self.z80.time = until - 4 * 1_070_460;
        }
        while self.z80.time < until {
            if !self.z80.running(self.z80.time) {
                self.z80.time = until;
                break;
            }
            let t = self.z80.time;
            self.vdp.advance(t);
            let mut cpu = self.z80.cpu;
            if self.vdp.z80_int() {
                cpu.request_interrupt();
            }
            self.z80.offset = 0;
            self.z80.wait = 0;
            let tstates = cpu.step(self);
            self.z80.cpu = cpu;
            self.z80.time += (u64::from(tstates) + self.z80.wait) * Z80_DIVIDER;
            self.z80.offset = 0;
            self.z80.wait = 0;
        }
    }
}

impl machine::Bus for Bus {
    fn read(&mut self, addr: u16) -> u8 {
        self.z80_read(addr)
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.z80_write(addr, value);
    }

    fn advance_tstates(&mut self, n: u8) {
        self.z80.offset += u64::from(n);
    }

    fn read_io(&mut self, _port: u16) -> u8 {
        0xFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Region};
    use mc68000::{FunctionCode, M68kBus, Size};

    fn bus() -> Bus {
        Bus::new(&Config::new(vec![0x53, 0x45, 0x47, 0x41], Region::Usa))
    }

    fn wr(b: &mut Bus, addr: u32, size: Size, v: u32) {
        b.write(addr, size, v, FunctionCode::SUPERVISOR_DATA)
            .unwrap();
    }

    fn rd(b: &mut Bus, addr: u32, size: Size) -> u32 {
        b.read(addr, size, FunctionCode::SUPERVISOR_DATA).unwrap()
    }

    /// The documented start of Z80 work: reset, request the bus, release the reset.
    fn open(b: &mut Bus) {
        wr(b, 0xA1_1200, Size::Word, 0x0000);
        wr(b, 0xA1_1100, Size::Word, 0x0100);
        wr(b, 0xA1_1200, Size::Word, 0x0100);
        b.now += 1000;
    }

    #[test]
    fn z80_ram_is_visible_to_the_68000_only_when_the_bus_is_granted() {
        let mut b = bus();
        wr(&mut b, 0xA0_0000, Size::Byte, 0x12);
        assert_eq!(
            rd(&mut b, 0xA0_0000, Size::Byte),
            0xFF,
            "no bus: nothing there"
        );
        open(&mut b);
        wr(&mut b, 0xA0_0000, Size::Byte, 0x12);
        assert_eq!(rd(&mut b, 0xA0_0000, Size::Byte), 0x12);
        assert_eq!(rd(&mut b, 0xA0_2000, Size::Byte), 0x12, "the RAM repeats");
        assert!(b.stop().is_none());
    }

    #[test]
    fn the_z80_area_takes_bytes_a_word_write_stores_its_high_byte_and_a_word_read_repeats_the_byte()
    {
        let mut b = bus();
        open(&mut b);
        wr(&mut b, 0xA0_0002, Size::Word, 0x3456);
        assert_eq!(rd(&mut b, 0xA0_0002, Size::Byte), 0x34);
        assert_eq!(rd(&mut b, 0xA0_0003, Size::Byte), 0);
        assert_eq!(rd(&mut b, 0xA0_0002, Size::Word), 0x3434);
        wr(&mut b, 0xA0_0007, Size::Byte, 0xBC);
        assert_eq!(rd(&mut b, 0xA0_0007, Size::Byte), 0xBC);
        assert_eq!(rd(&mut b, 0xA0_0006, Size::Byte), 0);
    }

    #[test]
    fn the_bus_request_register_reads_zero_once_granted() {
        let mut b = bus();
        wr(&mut b, 0xA1_1100, Size::Word, 0x0100);
        assert_eq!(
            rd(&mut b, 0xA1_1100, Size::Byte) & 1,
            0,
            "in reset: granted at once"
        );
        wr(&mut b, 0xA1_1200, Size::Word, 0x0100);
        wr(&mut b, 0xA1_1100, Size::Word, 0x0000);
        wr(&mut b, 0xA1_1100, Size::Word, 0x0100);
        // a running Z80 gives the bus up after a few clocks
        let first = rd(&mut b, 0xA1_1100, Size::Byte) & 1;
        b.now += 1000;
        assert_eq!(rd(&mut b, 0xA1_1100, Size::Byte) & 1, 0);
        assert!(first <= 1);
    }

    /// Z80 code at 0: `LD A,$5A ; LD ($0100),A ; JR $` and the 68000 collects the byte.
    #[test]
    fn the_z80_runs_a_program_the_68000_copied() {
        let mut b = bus();
        open(&mut b);
        for (i, byte) in [0x3Eu8, 0x5A, 0x32, 0x00, 0x01, 0x18, 0xFE]
            .iter()
            .enumerate()
        {
            wr(&mut b, 0xA0_0000 + i as u32, Size::Byte, u32::from(*byte));
        }
        wr(&mut b, 0xA1_1200, Size::Word, 0x0000);
        b.now += 10 * 15;
        wr(&mut b, 0xA1_1200, Size::Word, 0x0100);
        wr(&mut b, 0xA1_1100, Size::Word, 0x0000);
        b.now += 2000;
        b.run_z80(b.now);
        wr(&mut b, 0xA1_1100, Size::Word, 0x0100);
        b.now += 1000;
        assert_eq!(rd(&mut b, 0xA0_0100, Size::Byte), 0x5A);
    }

    #[test]
    fn reset_holds_the_z80_and_a_release_starts_it_at_zero() {
        let mut b = bus();
        open(&mut b);
        wr(&mut b, 0xA0_0000, Size::Byte, 0x76); // HALT
        wr(&mut b, 0xA1_1200, Size::Word, 0x0000);
        assert!(b.z80.in_reset());
        b.now += 5000;
        b.run_z80(b.now);
        assert_eq!(b.z80.cpu.pc, 0);
    }

    #[test]
    fn the_bank_register_selects_the_cartridge_area_the_window_shows() {
        let mut b = bus();
        for _ in 0..9 {
            b.z80_write(0x6000, 0);
        }
        assert_eq!(b.z80_read(0x8000), 0x53);
        assert_eq!(b.z80_read(0x8003), 0x41);
        assert_eq!(b.z80_read(0x6000), 0xFF, "the bank register cannot be read");
    }

    #[test]
    fn the_work_ram_does_not_show_through_the_window() {
        let mut b = bus();
        wr(&mut b, 0xFF_8000, Size::Byte, 0x5A);
        for _ in 0..9 {
            b.z80_write(0x6000, 1);
        }
        assert_eq!(b.z80_read(0x8000), 0xFF);
    }

    #[test]
    fn the_z80_ram_repeats_at_2000() {
        let mut b = bus();
        b.z80_write(0x3F01, 0x55);
        assert_eq!(b.z80_read(0x1F01), 0x55);
    }
}
