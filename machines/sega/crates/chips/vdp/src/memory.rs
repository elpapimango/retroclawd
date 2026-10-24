//! VRAM, CRAM and VSRAM and the one way every write reaches them (a data port word or a DMA word).
//! Facts V3, V4, V17 and the probe P02 (`scenarios/probes/p02_vdp_memory.s`, four emulators agree).

use crate::Vdp;

pub const VRAM_BYTES: usize = 0x1_0000;
pub const CRAM_WORDS: usize = 64;
/// 64 words are kept; the chip has 40 (reads at and above index 40 wrap to 0, [O] P02).
pub const VSRAM_WORDS: usize = 64;
const VSRAM_WORDS_FITTED: usize = 40;

/// A bus word `BBB0GGG0RRR0` to the 9-bit CRAM format `BBBGGGRRR`.
pub(crate) fn pack_colour(bus: u16) -> u16 {
    ((bus & 0xE00) >> 3) | ((bus & 0x0E0) >> 2) | ((bus & 0x00E) >> 1)
}

/// The 9-bit CRAM format to the bus word.
pub(crate) fn unpack_colour(cram: u16) -> u16 {
    ((cram & 0x1C0) << 3) | ((cram & 0x038) << 2) | ((cram & 0x007) << 1)
}

impl Vdp {
    /// One word written to the memory the code selects, then the address advances by register 15.
    pub(crate) fn bus_write(&mut self, data: u16) {
        self.fifo[self.fifo_idx] = data;
        self.fifo_idx = (self.fifo_idx + 1) & 3;
        match self.code & 0x0F {
            0x01 => {
                let index = usize::from(self.addr & 0xFFFE);
                // At an odd address the two bytes land swapped.
                let word = if self.addr & 1 != 0 {
                    data.rotate_left(8)
                } else {
                    data
                };
                self.vram[index] = (word >> 8) as u8;
                self.vram[index + 1] = word as u8;
            }
            0x03 => {
                let index = usize::from(self.addr >> 1) & 0x3F;
                self.cram[index] = pack_colour(data);
            }
            0x05 => {
                let index = usize::from(self.addr >> 1) & 0x3F;
                self.vsram[index] = data & 0x7FF;
            }
            _ => {}
        }
        self.addr = self.addr.wrapping_add(u16::from(self.regs[15]));
    }

    /// One word read from the memory the code selects, then the address advances.
    pub(crate) fn bus_read(&mut self) -> u16 {
        let leftover = self.fifo[self.fifo_idx];
        let data = match self.code & 0x1F {
            0x00 => {
                let index = usize::from(self.addr & 0xFFFE);
                u16::from(self.vram[index]) << 8 | u16::from(self.vram[index + 1])
            }
            0x04 => {
                let mut index = usize::from(self.addr >> 1) & 0x3F;
                if index >= VSRAM_WORDS_FITTED {
                    index = 0;
                }
                (self.vsram[index] & 0x7FF) | (leftover & !0x7FF)
            }
            0x08 => {
                let index = usize::from(self.addr >> 1) & 0x3F;
                unpack_colour(self.cram[index]) | (leftover & !0xEEE)
            }
            // The undocumented 8-bit VRAM read: the byte at the address, in the low half.
            0x0C => u16::from(self.vram[usize::from(self.addr)]) | (leftover & !0xFF),
            _ => 0,
        };
        self.addr = self.addr.wrapping_add(u16::from(self.regs[15]));
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vdp() -> Vdp {
        let mut v = Vdp::new(false);
        v.regs[15] = 2;
        v
    }

    #[test]
    fn colours_pack_and_unpack_through_the_nine_bit_format() {
        assert_eq!(pack_colour(0x0EEE), 0x1FF);
        assert_eq!(unpack_colour(0x1FF), 0x0EEE);
        // the bits below each nibble's top three are not stored
        assert_eq!(unpack_colour(pack_colour(0x0123)), 0x0022);
        assert_eq!(unpack_colour(pack_colour(0xFFFF)), 0x0EEE);
    }

    #[test]
    fn a_vram_word_goes_to_an_even_pair_of_bytes_and_swaps_at_an_odd_address() {
        let mut v = vdp();
        v.code = 0x01;
        v.addr = 0x0100;
        v.bus_write(0xABCD);
        assert_eq!(&v.vram[0x100..0x102], &[0xAB, 0xCD]);
        assert_eq!(v.addr, 0x0102);
        v.addr = 0x0201;
        v.bus_write(0x1122);
        assert_eq!(&v.vram[0x200..0x202], &[0x22, 0x11]);
    }

    #[test]
    fn auto_increment_one_at_an_odd_address_overwrites_the_same_word() {
        let mut v = vdp();
        v.regs[15] = 1;
        v.code = 0x01;
        v.addr = 0x0200;
        v.bus_write(0x1122);
        v.bus_write(0x3344);
        assert_eq!(&v.vram[0x200..0x202], &[0x44, 0x33]);
        assert_eq!(v.addr, 0x0202);
    }

    #[test]
    fn the_address_wraps_at_64k() {
        let mut v = vdp();
        v.code = 0x01;
        v.addr = 0xFFFE;
        v.bus_write(0x0102);
        assert_eq!(v.addr, 0);
        assert_eq!(&v.vram[0xFFFE..0x10000], &[1, 2]);
    }

    #[test]
    fn cram_has_64_entries_and_the_address_wraps_at_the_end() {
        let mut v = vdp();
        v.code = 0x03;
        v.addr = 0x7E;
        v.bus_write(0x0EEE);
        v.bus_write(0x0222);
        assert_eq!(v.cram[63], 0x1FF);
        assert_eq!(v.cram[0], 0x049);
    }

    #[test]
    fn a_cram_read_fills_the_unused_bits_from_the_fifo() {
        let mut v = vdp();
        v.code = 0x03;
        v.addr = 0;
        v.bus_write(0xF123);
        v.code = 0x08;
        v.addr = 0;
        // fifo[fifo_idx] is the oldest of the four entries: here the initial 0
        assert_eq!(v.bus_read(), 0x0022);
        for _ in 0..3 {
            v.code = 0x03;
            v.bus_write(0xFFFF);
        }
        v.code = 0x08;
        v.addr = 0;
        // after four writes the oldest entry is the $F123
        assert_eq!(v.bus_read(), 0x0022 | 0xF123 & !0xEEE);
    }

    #[test]
    fn vsram_keeps_eleven_bits_and_reads_wrap_to_zero_from_index_40() {
        let mut v = vdp();
        v.code = 0x05;
        v.addr = 0;
        v.bus_write(0x0001);
        v.addr = 0x4E;
        v.bus_write(0x0777);
        v.bus_write(0x0666);
        v.code = 0x04;
        v.addr = 0x4E;
        assert_eq!(v.bus_read(), 0x0777);
        assert_eq!(v.bus_read(), 0x0001);
    }

    #[test]
    fn the_undocumented_8_bit_read_returns_one_byte() {
        let mut v = vdp();
        v.vram[0x41] = 0x5A;
        v.code = 0x0C;
        v.addr = 0x41;
        assert_eq!(v.bus_read() & 0xFF, 0x5A);
    }
}
