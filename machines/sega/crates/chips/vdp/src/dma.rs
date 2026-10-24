//! DMA: from the 68000 bus into VRAM, CRAM or VSRAM, a VRAM fill and a VRAM copy. The data moves when the command is
//! given; the time it takes is computed from the per-line transfer counts (fact V21b, from the table published with
//! Charles MacDonald's VDP document, repeated in the reference emulators' sources, and confirmed by probes `p34_*`,
//! `p35_*`: a fill runs at the slot count, 16 and 18 bytes a line while the picture is drawn; slot-exact timing is Phase 3).

use crate::Vdp;
use serde::{Deserialize, Serialize};

/// The 68000 side of a DMA transfer: the memory the VDP reads.
pub trait DmaBus {
    /// The word at `addr` (a 24-bit byte address, even).
    fn read_word(&mut self, addr: u32) -> u16;
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct DmaState {
    /// A fill is armed and starts with the next data port write.
    pub(crate) fill_pending: bool,
}

/// Units per line `[active, blank]` for `[H32, H40]` (fact V21b): bus to VRAM counts words, so half the bus table; bus to
/// CRAM/VSRAM counts words.
const BUS_WORDS_VRAM: [[u64; 2]; 2] = [[8, 83], [9, 102]];
const BUS_WORDS_CRAM: [[u64; 2]; 2] = [[16, 166], [18, 204]];
const FILL_BYTES: [[u64; 2]; 2] = [[16, 166], [18, 204]];
const COPY_BYTES: [[u64; 2]; 2] = [[8, 83], [9, 102]];

impl Vdp {
    fn dma_length(&self) -> u32 {
        match u32::from(self.regs[20]) << 8 | u32::from(self.regs[19]) {
            0 => 0x1_0000,
            n => n,
        }
    }

    /// Registers 19-22 after a transfer: the source advances by the length and the length reads zero.
    fn dma_finish_registers(&mut self) {
        let end = (u16::from(self.regs[22]) << 8 | u16::from(self.regs[21]))
            .wrapping_add(u16::from(self.regs[20]) << 8 | u16::from(self.regs[19]));
        self.regs[21] = end as u8;
        self.regs[22] = (end >> 8) as u8;
        self.regs[19] = 0;
        self.regs[20] = 0;
    }

    /// Starts the DMA the command selected (register 23 bits 7-6). Returns the master clock the 68000 is held until.
    pub(crate) fn start_dma(&mut self, now: u64, bus: &mut dyn DmaBus) -> u64 {
        match self.regs[23] >> 6 {
            2 => {
                self.dma.fill_pending = true;
                self.dma_busy_until = u64::MAX;
                now
            }
            3 => {
                self.dma_copy(now);
                now
            }
            _ => self.dma_bus(now, bus),
        }
    }

    fn rate(&self, table: &[[u64; 2]; 2]) -> (u64, u64) {
        let row = table[usize::from(self.h40())];
        (row[0], row[1])
    }

    /// DMA from the 68000 bus: the 68000 waits until it is done.
    fn dma_bus(&mut self, now: u64, bus: &mut dyn DmaBus) -> u64 {
        let length = self.dma_length();
        let high = u32::from(self.regs[23] & 0x7F) << 17;
        let mut source = high | (u32::from(self.regs[22]) << 8 | u32::from(self.regs[21])) << 1;
        for _ in 0..length {
            let word = bus.read_word(source);
            // The source address wraps inside its 128 KB block (fact V15).
            source = high | (source.wrapping_add(2) & 0x1_FFFF);
            self.bus_write(word);
        }
        self.dma_finish_registers();
        let (active, blank) = if self.code & 0x06 == 0 {
            self.rate(&BUS_WORDS_VRAM)
        } else {
            self.rate(&BUS_WORDS_CRAM)
        };
        self.dma_end(now, u64::from(length), active, blank)
    }

    /// VRAM copy: bytes inside VRAM; it needs CD4 set, otherwise nothing happens.
    fn dma_copy(&mut self, now: u64) {
        let length = self.dma_length();
        if self.code & 0x10 != 0 {
            let mut source = u16::from(self.regs[22]) << 8 | u16::from(self.regs[21]);
            for _ in 0..length {
                self.vram[usize::from(self.addr)] = self.vram[usize::from(source)];
                source = source.wrapping_add(1);
                self.addr = self.addr.wrapping_add(u16::from(self.regs[15]));
            }
        }
        self.dma_finish_registers();
        let (active, blank) = self.rate(&COPY_BYTES);
        self.dma_busy_until = self.dma_end(now, u64::from(length), active, blank);
    }

    /// The fill that was armed runs with the data written by the port write that started it.
    pub(crate) fn dma_fill(&mut self, now: u64, data: u16) {
        self.dma.fill_pending = false;
        let length = self.dma_length();
        match self.code & 0x0F {
            0x01 => {
                for _ in 0..length {
                    self.vram[usize::from(self.addr)] = (data >> 8) as u8;
                    self.addr = self.addr.wrapping_add(u16::from(self.regs[15]));
                }
            }
            0x03 => {
                let packed = crate::memory::pack_colour(data);
                for _ in 0..length {
                    self.cram[usize::from(self.addr >> 1) & 0x3F] = packed;
                    self.addr = self.addr.wrapping_add(u16::from(self.regs[15]));
                }
            }
            0x05 => {
                for _ in 0..length {
                    self.vsram[usize::from(self.addr >> 1) & 0x3F] = data & 0x7FF;
                    self.addr = self.addr.wrapping_add(u16::from(self.regs[15]));
                }
            }
            _ => {
                self.addr = self
                    .addr
                    .wrapping_add((u32::from(self.regs[15]).wrapping_mul(length)) as u16);
            }
        }
        self.dma_finish_registers();
        let (active, blank) = self.rate(&FILL_BYTES);
        self.dma_busy_until = self.dma_end(now, u64::from(length), active, blank);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Source(Vec<u16>);

    impl DmaBus for Source {
        fn read_word(&mut self, addr: u32) -> u16 {
            self.0[(addr as usize >> 1) % self.0.len()]
        }
    }

    fn vdp() -> Vdp {
        let mut v = Vdp::new(false);
        v.regs[1] = 0x74;
        v.regs[15] = 2;
        v
    }

    fn command(v: &mut Vdp, code: u8, addr: u16) {
        v.addr = addr;
        v.code = code;
    }

    #[test]
    fn memory_to_vram_copies_words_and_leaves_the_registers_advanced() {
        let mut v = vdp();
        let mut src = Source((0..16).map(|i| 0x1000 + i).collect());
        v.regs[19] = 4;
        v.regs[21] = 2;
        command(&mut v, 0x21, 0x0100);
        let until = v.start_dma(0, &mut src);
        assert_eq!(
            &v.vram[0x100..0x108],
            &[0x10, 0x02, 0x10, 0x03, 0x10, 0x04, 0x10, 0x05]
        );
        assert_eq!(v.addr, 0x0108);
        assert_eq!(
            (v.regs[19], v.regs[20], v.regs[21], v.regs[22]),
            (0, 0, 6, 0)
        );
        assert!(until > 0);
    }

    #[test]
    fn length_zero_means_65536_words() {
        let mut v = vdp();
        let mut src = Source(vec![0xAAAA; 8]);
        command(&mut v, 0x21, 0);
        v.start_dma(0, &mut src);
        assert_eq!(v.addr, 0); // 65536 words of 2 bytes wrap the 16-bit address exactly
        assert_eq!(v.vram[0xFFFE], 0xAA);
    }

    #[test]
    fn the_source_wraps_at_a_128k_boundary() {
        struct Record(Vec<u32>);
        impl DmaBus for Record {
            fn read_word(&mut self, addr: u32) -> u16 {
                self.0.push(addr);
                0
            }
        }
        let mut v = vdp();
        let mut src = Record(Vec::new());
        v.regs[19] = 4;
        // word address $FFFE = byte address $1FFFC in block 0; reg 23 = 1 selects block $20000
        v.regs[21] = 0xFE;
        v.regs[22] = 0xFF;
        v.regs[23] = 0x01;
        command(&mut v, 0x21, 0);
        v.start_dma(0, &mut src);
        assert_eq!(src.0, vec![0x3_FFFC, 0x3_FFFE, 0x2_0000, 0x2_0002]);
    }

    #[test]
    fn a_fill_repeats_the_high_byte_after_the_data_word() {
        let mut v = vdp();
        v.regs[19] = 8;
        v.regs[15] = 1;
        command(&mut v, 0x21, 0x0200);
        v.dma_fill(0, 0x7700);
        assert_eq!(&v.vram[0x200..0x208], &[0x77; 8]);
        assert_eq!(v.addr, 0x0208);
    }

    #[test]
    fn copy_moves_bytes_inside_vram_and_needs_cd4() {
        let mut v = vdp();
        v.vram[0x10..0x14].copy_from_slice(&[1, 2, 3, 4]);
        v.regs[19] = 4;
        v.regs[21] = 0x10;
        v.regs[15] = 1;
        command(&mut v, 0x20, 0x0020);
        v.dma_copy(0);
        assert_eq!(&v.vram[0x20..0x24], &[0, 0, 0, 0]);
        command(&mut v, 0x30, 0x0020);
        v.regs[19] = 4;
        v.regs[21] = 0x10;
        v.dma_copy(0);
        assert_eq!(&v.vram[0x20..0x24], &[1, 2, 3, 4]);
    }

    #[test]
    fn the_busy_flag_stays_until_a_fill_has_its_data_then_clears_after_the_time() {
        let mut v = vdp();
        v.regs[23] = 0x80;
        let mut src = Source(vec![0]);
        command(&mut v, 0x21, 0);
        v.start_dma(0, &mut src);
        assert_eq!(v.dma_busy_until, u64::MAX);
        v.regs[19] = 18;
        v.dma_fill(0, 0);
        assert!(v.dma_busy_until < u64::MAX);
    }
}
