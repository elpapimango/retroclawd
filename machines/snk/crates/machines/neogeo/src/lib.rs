//! The Neo Geo's 68000 bus: system ROM, cartridge program ROM, work RAM, palette RAM and backup RAM, with the first
//! fetches of the BIOS as the goal. An address no fact in `docs/hardware-facts.md` covers stops the machine with a report
//! instead of returning an invented value (the project rule).
#![forbid(unsafe_code)]

use mc68000::{BusFault, FunctionCode, IackResponse, M68kBus, Size};

pub const BIOS_SIZE: usize = 0x2_0000;
pub const WORK_RAM: usize = 0x1_0000;
pub const PALETTE_RAM: usize = 0x2000;
pub const BACKUP_RAM: usize = 0x1_0000;
/// Video RAM in 16-bit words: the slow part `$0000-$7FFF` and the fast part `$8000-$87FF` (fact N13).
pub const VRAM_WORDS: usize = 0x8800;
/// Master clocks of one raster line (384 pixels of 4 clocks) and the lines of a frame (fact N20).
pub const LINE_CLOCKS: u64 = 1536;
pub const FRAME_LINES: u64 = 264;
/// The line at which the vertical blank interrupt is raised: after the 8 sync, 16 top border and 224 active lines (fact N20).
const VBLANK_LINE: u64 = 248;
/// Half a period of the clock chip's time pulse in master clocks: one second a period from 24 MHz (fact N19).
const TP_HALF_PERIOD: u64 = 24_000_000 / 2;

/// The video processor's registers as the 68000 sees them (facts N12, N13). Nothing is drawn yet.
#[derive(Clone, Debug, Default)]
pub struct Lspc {
    pub vram_addr: u16,
    pub vram_mod: u16,
    pub mode: u16,
    pub timer_high: u16,
    pub timer_low: u16,
    pub irq_ack: u16,
    pub timer_stop: u16,
}

/// Swaps the two bytes of every 16-bit word.
fn swap_words(data: &mut [u8]) {
    for w in data.as_chunks_mut::<2>().0 {
        w.swap(0, 1);
    }
}

/// What the 68000 sees. The cartridge's P ROM is `p`.
pub struct Bus {
    pub bios: Vec<u8>,
    pub p: Vec<u8>,
    pub work: Vec<u8>,
    /// Two banks of `PALETTE_RAM` bytes; the window at `$400000` shows the one the latch selects (fact N14).
    pub palette: Vec<u8>,
    /// The system register latch: bit n holds the value written to the pair of registers `$3A0001 + 2n` and `$3A0011 + 2n`
    /// (fact N14). Bit 1 set: the cartridge's vector table; bit 7 set: palette bank 0.
    pub latch: u8,
    /// The last values written to REG_POUTPUT, REG_SLOT, REG_LEDLATCHES, REG_LEDDATA and REG_RTCCTRL (fact N15).
    pub ctrl: [u8; 5],
    /// The coin counter 1 and 2 and coin lockout 1 and 2 lines: true when the SET register sank current (fact N16).
    pub coin: [bool; 4],
    /// REG_DIPSW as read: active low, so `$FF` is every switch off (fact N17).
    pub dips: u8,
    /// Machine time in master clocks (24 MHz), advanced by the owner of the bus (fact N19).
    pub clock: u64,
    /// The pending interrupts: bit 0 the power-on reset one (level 3), bit 1 the timer (level 2), bit 2 the vertical blank (level 1) (fact N21).
    pub irq: u8,
    pub backup: Vec<u8>,
    pub vram: Vec<u16>,
    pub lspc: Lspc,
    /// How many times the watchdog was kicked (fact N11).
    pub watchdog_kicks: u64,
    /// The first report of an access no fact covers; the machine is stopped once it is set.
    pub stop: Option<String>,
}

impl Bus {
    /// `bios` and `p` are the files as the sets store them: each 16-bit word byte-swapped (fact N10).
    pub fn new(mut bios: Vec<u8>, mut p: Vec<u8>) -> Bus {
        swap_words(&mut bios);
        swap_words(&mut p);
        Bus {
            bios,
            p,
            work: vec![0; WORK_RAM],
            palette: vec![0; 2 * PALETTE_RAM],
            latch: 0x80,
            ctrl: [0; 5],
            coin: [false; 4],
            dips: 0xFF,
            clock: 0,
            irq: 1,
            backup: vec![0; BACKUP_RAM],
            vram: vec![0; VRAM_WORDS],
            lspc: Lspc::default(),
            watchdog_kicks: 0,
            stop: None,
        }
    }

    fn report(&mut self, what: &str, addr: u32, size: Size, value: Option<u32>) {
        if self.stop.is_none() {
            let w = if size == Size::Byte { "byte" } else { "word" };
            self.stop = Some(match value {
                Some(v) => {
                    format!("stop: {what} {w} ${addr:06X} = ${v:X}: no fact covers this address")
                }
                None => format!("stop: {what} {w} ${addr:06X}: no fact covers this address"),
            });
        }
    }

    /// Moves machine time on by `master_clocks`, raising the vertical blank interrupt at its line.
    pub fn advance(&mut self, master_clocks: u64) {
        let before = self.clock / LINE_CLOCKS;
        self.clock += master_clocks;
        let after = self.clock / LINE_CLOCKS;
        // the vertical blank begins at line 248 of every 264
        let blanks = |n: u64| (n + FRAME_LINES - VBLANK_LINE) / FRAME_LINES;
        if blanks(after) > blanks(before) {
            self.irq |= 4;
        }
    }

    /// The interrupt priority level the 68000 sees (fact N21): the reset interrupt is level 3, the timer 2, the blank 1.
    pub fn ipl(&self) -> u8 {
        if self.irq & 1 != 0 {
            3
        } else if self.irq & 2 != 0 {
            2
        } else if self.irq & 4 != 0 {
            1
        } else {
            0
        }
    }

    /// Where `a` of the palette window falls in the two banks (fact N14).
    fn palette_at(&self, a: usize) -> usize {
        let bank = usize::from(self.latch & 0x80 == 0);
        bank * PALETTE_RAM + (a & (PALETTE_RAM - 1))
    }

    /// The byte at `addr` in a plain memory region, if one is there (facts N5-N9, N14).
    fn byte(&self, addr: u32) -> Option<u8> {
        let a = (addr & 0xFF_FFFF) as usize;
        match a {
            // the vector table: the system ROM's until the program asks for the cartridge's (fact N14)
            0x00_0000..=0x00_007F if self.latch & 2 == 0 => self.bios.get(a).copied(),
            // the first bank of the cartridge's program; with a 2 MiB program the halves are swapped (fact N5)
            0x00_0000..=0x0F_FFFF => {
                let at = if self.p.len() >= 0x20_0000 {
                    a + 0x10_0000
                } else {
                    a
                };
                self.p.get(at).copied()
            }
            0x20_0000..=0x2F_FFFF if self.p.len() >= 0x20_0000 => self.p.get(a & 0xF_FFFF).copied(),
            0x10_0000..=0x1F_FFFF => Some(self.work[a & (WORK_RAM - 1)]),
            0xC0_0000..=0xC1_FFFF => self.bios.get(a & (BIOS_SIZE - 1)).copied(),
            0x40_0000..=0x7F_FFFF => Some(self.palette[self.palette_at(a)]),
            0xD0_0000..=0xD0_FFFF => Some(self.backup[a & (BACKUP_RAM - 1)]),
            _ if a & 0xFE_0081 == 0x30_0001 => Some(self.dips),
            // REG_SYSTYPE / the test switch: bit 7 is the test button (active low), bit 0 the slot count (fact N23)
            _ if a & 0xFE_0081 == 0x30_0081 => Some(0xFF),
            // REG_STATUS_A: coin 1, coin 2 and service are active low and not pressed; bit 6 is the clock's time pulse (facts N18, N19)
            // REG_STATUS_B: start and select buttons, memory card status (active low), bit 7 set on an MVS (fact N22)
            0x38_0000..=0x38_FFFF if a & 1 == 0 && a & 0xFE_00F1 == 0x38_0000 => Some(0xFF),
            _ if a & 0xFE_0001 == 0x32_0001 => {
                Some(0x07 | (((self.clock / TP_HALF_PERIOD) & 1) as u8) << 6)
            }
            _ => None,
        }
    }

    fn put(&mut self, addr: u32, v: u8) -> bool {
        let a = (addr & 0xFF_FFFF) as usize;
        match a {
            // REG_DIPSW written: any value kicks the watchdog (fact N11)
            _ if a & 0xFE_0001 == 0x30_0001 => self.watchdog_kicks += 1,
            0x10_0000..=0x1F_FFFF => self.work[a & (WORK_RAM - 1)] = v,
            0x40_0000..=0x7F_FFFF => {
                let at = self.palette_at(a);
                self.palette[at] = v;
            }
            0xD0_0000..=0xD0_FFFF => self.backup[a & (BACKUP_RAM - 1)] = v,
            // the system register latch: A3-A1 pick the bit and A4 is the value (fact N14)
            _ if a & 0xFE_0001 == 0x3A_0001 => {
                let (bit, on) = ((a >> 1) & 7, (a >> 4) & 1 != 0);
                self.latch = self.latch & !(1 << bit) | u8::from(on) << bit;
            }
            // the write-only output registers (fact N15)
            _ => match a & 0xFE_00F1 {
                0x38_0001 => self.ctrl[0] = v,
                0x38_0021 => self.ctrl[1] = v,
                0x38_0031 => self.ctrl[2] = v,
                0x38_0041 => self.ctrl[3] = v,
                0x38_0051 => self.ctrl[4] = v,
                r @ (0x38_0061 | 0x38_0063 | 0x38_0065 | 0x38_0067) => {
                    self.coin[((r - 0x38_0061) / 2) as usize] = false
                }
                r @ (0x38_00E1 | 0x38_00E3 | 0x38_00E5 | 0x38_00E7) => {
                    self.coin[((r - 0x38_00E1) / 2) as usize] = true
                }
                _ => return false,
            },
        }
        true
    }
}

impl Bus {
    /// A word access to the video processor's registers `$3C0000-$3C000F`; `None` for a register no fact covers.
    fn lspc_read(&mut self, reg: u32) -> Option<u16> {
        match reg {
            0x0 | 0x2 => Some(self.vram[usize::from(self.lspc.vram_addr) % VRAM_WORDS]),
            0x4 => Some(self.lspc.vram_mod),
            _ => None,
        }
    }

    fn lspc_write(&mut self, reg: u32, v: u16) -> bool {
        if reg == 0xC {
            // REG_IRQACK: bit 0 clears the reset interrupt, bit 1 the timer's, bit 2 the vertical blank's (fact N21)
            self.irq &= !(v as u8 & 7);
        }
        let l = &mut self.lspc;
        match reg {
            0x0 => l.vram_addr = v,
            0x2 => {
                self.vram[usize::from(l.vram_addr) % VRAM_WORDS] = v;
                l.vram_addr = l.vram_addr.wrapping_add(l.vram_mod);
            }
            0x4 => l.vram_mod = v,
            0x6 => l.mode = v,
            0x8 => l.timer_high = v,
            0xA => l.timer_low = v,
            0xC => l.irq_ack = v,
            0xE => l.timer_stop = v,
            _ => return false,
        }
        true
    }
}

impl M68kBus for Bus {
    fn read(&mut self, addr: u32, size: Size, _fc: FunctionCode) -> Result<u32, BusFault> {
        if size == Size::Word && (addr & 0xFF_FFF0) == 0x3C_0000 {
            return match self.lspc_read(addr & 0xF) {
                Some(v) => Ok(u32::from(v)),
                None => {
                    self.report("read", addr, size, None);
                    Ok(0)
                }
            };
        }
        let value = match size {
            Size::Byte => self.byte(addr).map(u32::from),
            Size::Word => match (self.byte(addr), self.byte(addr | 1)) {
                (Some(hi), Some(lo)) => Some(u32::from(hi) << 8 | u32::from(lo)),
                _ => None,
            },
        };
        match value {
            Some(v) => Ok(v),
            None => {
                self.report("read", addr, size, None);
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
        if size == Size::Word && (addr & 0xFF_FFF0) == 0x3C_0000 {
            if !self.lspc_write(addr & 0xF, value as u16) {
                self.report("write", addr, size, Some(value));
            }
            return Ok(());
        }
        let ok = match size {
            Size::Byte => self.put(addr, value as u8),
            Size::Word => self.put(addr, (value >> 8) as u8) && self.put(addr | 1, value as u8),
        };
        if !ok {
            self.report("write", addr, size, Some(value));
        }
        Ok(())
    }

    fn iack(&mut self, _level: u8) -> IackResponse {
        IackResponse::Autovector
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FC: FunctionCode = FunctionCode::SUPERVISOR_DATA;

    fn bus() -> Bus {
        // words stored byte-swapped: the BIOS's first word is $1234, the cartridge's $5678
        let mut bios = vec![0u8; BIOS_SIZE];
        bios[0] = 0x34;
        bios[1] = 0x12;
        let mut p = vec![0u8; 0x1000];
        p[0] = 0x78;
        p[1] = 0x56;
        Bus::new(bios, p)
    }

    #[test]
    fn dumps_are_swapped_back_and_the_bios_vectors_show_until_the_cartridge_is_asked_for() {
        let mut b = bus();
        assert_eq!(b.read(0xC0_0000, Size::Word, FC), Ok(0x1234));
        assert_eq!(
            b.read(0, Size::Word, FC),
            Ok(0x1234),
            "the system ROM's vectors at reset"
        );
        b.write(0x3A_0013, Size::Byte, 0, FC).unwrap(); // SWPROM
        assert_eq!(b.read(0, Size::Word, FC), Ok(0x5678));
        b.write(0x3A_0003, Size::Byte, 0, FC).unwrap(); // SWPBIOS
        assert_eq!(b.read(0, Size::Word, FC), Ok(0x1234));
    }

    #[test]
    fn work_ram_mirrors_and_palette_ram_has_two_banks() {
        let mut b = bus();
        b.write(0x10_0010, Size::Word, 0xBEEF, FC).unwrap();
        assert_eq!(b.read(0x11_0010, Size::Word, FC), Ok(0xBEEF));
        b.write(0x40_0000, Size::Word, 0x1111, FC).unwrap();
        b.write(0x3A_000F, Size::Byte, 0, FC).unwrap(); // PALBANK1
        assert_eq!(b.read(0x40_0000, Size::Word, FC), Ok(0));
        b.write(0x40_0000, Size::Word, 0x2222, FC).unwrap();
        b.write(0x3A_001F, Size::Byte, 0, FC).unwrap(); // PALBANK0
        assert_eq!(b.read(0x40_0000, Size::Word, FC), Ok(0x1111));
        assert_eq!(b.read(0x40_2000, Size::Word, FC), Ok(0x1111), "mirrored");
    }

    #[test]
    fn video_ram_is_written_through_the_data_port_with_the_modulo() {
        let mut b = bus();
        b.write(0x3C_0004, Size::Word, 2, FC).unwrap();
        b.write(0x3C_0000, Size::Word, 0x100, FC).unwrap();
        b.write(0x3C_0002, Size::Word, 0xAAAA, FC).unwrap();
        b.write(0x3C_0002, Size::Word, 0xBBBB, FC).unwrap();
        assert_eq!(b.vram[0x100], 0xAAAA);
        assert_eq!(b.vram[0x102], 0xBBBB);
    }

    #[test]
    fn an_address_no_fact_covers_stops_the_machine_with_a_report() {
        let mut b = bus();
        b.write(0x38_0065, Size::Byte, 0, FC).unwrap();
        assert!(b.stop.as_deref().unwrap().contains("$380065"));
    }
}
