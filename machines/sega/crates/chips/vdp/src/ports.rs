//! The control, data and status ports as the 68000 sees them (facts V1-V4, V22; probes P02, P03).

use crate::{DmaBus, Vdp};

impl Vdp {
    /// A word written to the control port at master clock `now`. Returns the master clock the 68000 must wait until
    /// (`now` when the write did not start a DMA from the bus).
    pub fn write_control(&mut self, value: u16, now: u64, bus: &mut dyn DmaBus) -> u64 {
        if !self.pending {
            self.addr = self.addr_latch | (value & 0x3FFF);
            self.code = (self.code & 0x3C) | ((value >> 14) & 3) as u8;
            if value & 0xC000 == 0x8000 {
                self.write_register(usize::from((value >> 8) & 0x1F), value as u8);
            } else {
                // Only mode 5 takes a two-word command.
                self.pending = self.regs[1] & 0x04 != 0;
            }
            now
        } else {
            self.pending = false;
            self.addr_latch = (value & 3) << 14;
            self.addr = self.addr_latch | (self.addr & 0x3FFF);
            self.code = (self.code & 3) | ((value >> 2) & 0x3C) as u8;
            if self.code & 0x20 != 0 && self.dma_enabled() {
                self.start_dma(now, bus)
            } else {
                now
            }
        }
    }

    pub(crate) fn write_register(&mut self, reg: usize, value: u8) {
        if let Some(r) = self.regs.get_mut(reg) {
            *r = value;
        }
    }

    /// A word written to the data port.
    pub fn write_data(&mut self, value: u16, now: u64) {
        self.pending = false;
        self.bus_write(value);
        if self.dma.fill_pending {
            self.dma_fill(now, value);
        }
    }

    /// A word read from the data port.
    pub fn read_data(&mut self) -> u16 {
        self.pending = false;
        self.bus_read()
    }

    /// The status register (fact V22). Reading it ends a half-written command.
    pub fn read_status(&mut self, now: u64) -> u16 {
        let phase = self.phase(now);
        let mut status = 0u16;
        if self.pal {
            status |= 0x0001;
        }
        if now < self.dma_busy_until {
            status |= 0x0002;
        }
        if self.line_is_blank(phase.line) {
            status |= 0x0008;
        }
        if self.in_hblank(phase.clock) {
            status |= 0x0004;
        }
        if self.vint_flag() {
            status |= 0x0080;
        }
        if self.odd_frame {
            status |= 0x0010;
        }
        if self.sprite_collision {
            status |= 0x0020;
        }
        if self.sprite_overflow {
            status |= 0x0040;
        }
        // The FIFO is empty (the slot-accurate FIFO is Phase 3).
        status |= 0x0200;
        self.pending = false;
        self.sprite_collision = false;
        self.sprite_overflow = false;
        status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoBus;

    impl DmaBus for NoBus {
        fn read_word(&mut self, _addr: u32) -> u16 {
            0
        }
    }

    fn vdp() -> Vdp {
        let mut v = Vdp::new(false);
        v.write_control(0x8004, 0, &mut NoBus);
        v.write_control(0x8104, 0, &mut NoBus);
        v.write_control(0x8F02, 0, &mut NoBus);
        v
    }

    fn command(v: &mut Vdp, long: u32) {
        v.write_control((long >> 16) as u16, 0, &mut NoBus);
        v.write_control(long as u16, 0, &mut NoBus);
    }

    #[test]
    fn a_register_write_sets_the_register() {
        let mut v = Vdp::new(false);
        v.write_control(0x8F02, 0, &mut NoBus);
        assert_eq!(v.reg(15), 2);
        v.write_control(0x9740, 0, &mut NoBus);
        assert_eq!(v.reg(23), 0x40);
    }

    #[test]
    fn the_two_word_command_decodes_address_and_code() {
        let mut v = vdp();
        command(&mut v, 0x4123_0002); // VRAM write at $0123, address bits 15-14 from the low bits of the second word
        assert_eq!(v.code, 0x01);
        assert_eq!(v.addr, 0x8123);
        command(&mut v, 0xC000_0000);
        assert_eq!(v.code, 0x03);
        command(&mut v, 0x4000_0010);
        assert_eq!(v.code, 0x05);
        command(&mut v, 0x0000_0020);
        assert_eq!(v.code, 0x08);
        command(&mut v, 0x0000_0010);
        assert_eq!(v.code, 0x04);
    }

    #[test]
    fn the_first_word_waits_for_the_second_in_mode_5_only() {
        let mut v = Vdp::new(false);
        v.write_control(0x4000, 0, &mut NoBus);
        assert!(!v.pending, "mode 5 is off at power on");
        v.write_control(0x8104, 0, &mut NoBus);
        v.write_control(0x4000, 0, &mut NoBus);
        assert!(v.pending);
        v.read_data();
        assert!(!v.pending);
    }

    #[test]
    fn data_written_after_a_command_lands_at_the_address_and_reads_back() {
        let mut v = vdp();
        command(&mut v, 0x4000_0000);
        v.write_data(0x1234, 0);
        v.write_data(0x5678, 0);
        command(&mut v, 0x0000_0000);
        assert_eq!(v.read_data(), 0x1234);
        assert_eq!(v.read_data(), 0x5678);
    }

    #[test]
    fn a_dma_command_with_dma_disabled_does_nothing() {
        let mut v = vdp();
        v.regs[19] = 4;
        v.regs[23] = 0x00;
        command(&mut v, 0x4000_0080);
        assert_eq!(v.regs[19], 4);
    }

    #[test]
    fn a_dma_fill_starts_with_the_data_word_and_the_busy_flag_follows() {
        let mut v = vdp();
        v.write_control(0x8174, 0, &mut NoBus); // display on, DMA enabled, mode 5
        v.write_control(0x9780, 0, &mut NoBus); // fill
        v.write_control(0x9310, 0, &mut NoBus);
        v.write_control(0x9400, 0, &mut NoBus);
        v.write_control(0x8F01, 0, &mut NoBus);
        command(&mut v, 0x4000_0080);
        assert!(
            v.read_status(0) & 2 != 0,
            "busy while the fill waits for its data"
        );
        v.write_data(0x5500, 0);
        // the data word wrote bytes 0 and 1 and moved the address on by 1; the fill then rewrites byte 1 and the 15 after it
        assert_eq!(&v.vram[0..17], &[0x55; 17]);
        assert_eq!(v.vram[17], 0);
        let status = v.read_status(1);
        assert!(status & 2 != 0, "still busy just after");
        assert!(
            v.read_status(10 * crate::LINE_CLOCKS) & 2 == 0,
            "done a few lines later"
        );
    }

    #[test]
    fn the_status_register_reports_the_region_and_the_blanking() {
        let mut ntsc = Vdp::new(false);
        let mut pal = Vdp::new(true);
        assert_eq!(ntsc.read_status(0) & 1, 0);
        assert_eq!(pal.read_status(0) & 1, 1);
        // the display is off at power on: vertical blanking flag set, FIFO empty
        assert_eq!(ntsc.read_status(0) & 0x208, 0x208);
        ntsc.regs[1] = 0x40;
        assert_eq!(ntsc.read_status(0) & 0x8, 0);
        assert_eq!(ntsc.read_status(230 * crate::LINE_CLOCKS) & 0x8, 0x8);
    }

    #[test]
    fn reading_the_status_ends_a_half_written_command() {
        let mut v = vdp();
        v.write_control(0x4000, 0, &mut NoBus);
        assert!(v.pending);
        v.read_status(0);
        assert!(!v.pending);
    }
}
