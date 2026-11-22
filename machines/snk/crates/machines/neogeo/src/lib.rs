//! The Neo Geo's 68000 bus: system ROM, cartridge program ROM, work RAM, palette RAM and backup RAM, with the first
//! fetches of the BIOS as the goal. An address no fact in `docs/hardware-facts.md` covers stops the machine with a report
//! instead of returning an invented value (the project rule).
#![forbid(unsafe_code)]

pub mod protection;
pub mod rtc;
pub mod sound;
pub mod system;
pub mod video;

pub use sound::Sound;

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
/// Master clocks the watchdog waits without a kick before it resets the machine: about 0.13 s (fact N51).
pub const WATCHDOG_CLOCKS: u64 = 3_120_000;
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

/// Where a 16-bit video RAM address falls in the 0x8800 words: the fast part (`$8000` up) repeats every 2 KiW (fact N44).
fn vram_index(addr: u16) -> usize {
    if addr >= 0x8000 {
        0x8000 | usize::from(addr & 0x7FF)
    } else {
        usize::from(addr)
    }
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
    /// REG_P1CNT and REG_P2CNT as read: up, down, left, right, A, B, C, D in bits 0-7, active low (fact N28).
    pub pads: [u8; 2],
    /// Bits held down (1 = pressed) of REG_STATUS_A (coin 1, coin 2, service) and REG_STATUS_B (start and select of both players) (fact N33).
    pub pressed_a: u8,
    pub pressed_b: u8,
    /// Machine time in master clocks (24 MHz), advanced by the owner of the bus (fact N19).
    pub clock: u64,
    /// The pending interrupts: bit 0 the power-on reset one (level 3), bit 1 the timer (level 2), bit 2 the vertical blank (level 1) (fact N21).
    pub irq: u8,
    /// The auto-animation counter, 3 bits (fact N34).
    pub anim: u8,
    anim_frames: u64,
    /// The LSPC's timer: a down counter in pixel clocks (fact N41), and the master clocks not yet made into pixel clocks.
    timer_counter: u32,
    timer_fraction: u64,
    pub backup: Vec<u8>,
    pub vram: Vec<u16>,
    /// How many VRAM accesses named an address past `$87FF` (a debugging aid).
    pub vram_oob: u64,
    /// The calendar chip, and the date and time it starts at (year, month, day, hour, minute, second) with the weekday of that day (fact N48).
    pub rtc: rtc::Rtc,
    pub rtc_start: ((u32, u32, u32, u32, u32, u32), u32),
    /// The writes to REG_RTCCTRL (value, pc) and the reads of REG_STATUS_A (value, pc), in order (a debugging aid).
    pub rtc_log: Vec<(char, u8, u32)>,
    /// A debugging aid: the vertical blanks raised, the timer interrupts raised and the acknowledgements written for bits 0-2.
    pub irq_counts: [u64; 5],
    /// Which 1 MiB bank of the second program file shows at `$200000` (fact N47).
    pub p_bank: usize,
    /// Where the window at `$200000` starts in the program when a chip picked it (SMA, PVC), else the 1 MiB banks of `p_bank` (fact N53).
    pub p_window: Option<usize>,
    /// The protection board and the PVC chip's 8 KiB of RAM (fact N53).
    pub board: neo_set::carts::Board,
    pub pvc_ram: Vec<u16>,
    /// The cartridge's challenge and response chip, as far as seen (fact N45).
    pub protection: protection::Protection,
    /// A debugging aid: when set, an access no fact covers is logged in `access_log` (with the 68000's pc) and answered with `$FF` or
    /// ignored, instead of stopping the machine.
    pub lenient: bool,
    pub access_log: Vec<(u32, bool, u32, u32)>,
    pub current_pc: u32,
    pub lspc: Lspc,
    pub sound: Sound,
    /// How many times the watchdog was kicked (fact N11).
    pub watchdog_kicks: u64,
    /// The master clock of the last kick; the watchdog runs once it has been kicked (fact N51).
    pub watchdog_last: Option<u64>,
    /// How many times the watchdog reset the machine.
    pub watchdog_resets: u64,
    /// The first report of an access no fact covers; the machine is stopped once it is set.
    pub stop: Option<String>,
}

impl Bus {
    /// `bios` and `p` are the files as the sets store them: each 16-bit word byte-swapped (fact N10).
    pub fn new(mut bios: Vec<u8>, mut p: Vec<u8>, sm1: Vec<u8>, m1: Vec<u8>) -> Bus {
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
            pads: [0xFF; 2],
            pressed_a: 0,
            pressed_b: 0,
            clock: 0,
            irq: 1,
            anim: 0,
            anim_frames: 0,
            timer_counter: 0,
            timer_fraction: 0,
            backup: vec![0; BACKUP_RAM],
            vram: vec![0; VRAM_WORDS],
            vram_oob: 0,
            rtc: rtc::Rtc::default(),
            rtc_start: ((0, 1, 1, 0, 0, 0), 6),
            rtc_log: Vec::new(),
            p_bank: 0,
            p_window: None,
            board: neo_set::carts::Board::Plain,
            pvc_ram: vec![0; 0x1000],
            irq_counts: [0; 5],
            protection: protection::Protection::default(),
            lenient: false,
            access_log: Vec::new(),
            current_pc: 0,
            lspc: Lspc::default(),
            sound: Sound::new(sm1, m1),
            watchdog_kicks: 0,
            watchdog_last: None,
            watchdog_resets: 0,
            stop: None,
        }
    }

    /// True once the watchdog has gone unkicked for `WATCHDOG_CLOCKS` (fact N51).
    pub fn watchdog_expired(&self) -> bool {
        self.watchdog_last
            .is_some_and(|t| self.clock.saturating_sub(t) > WATCHDOG_CLOCKS)
    }

    /// What the watchdog's reset puts back: the registers, the interrupts, the LSPC and the sound side. The RAMs keep their contents.
    pub fn watchdog_reset(&mut self) {
        self.watchdog_resets += 1;
        self.watchdog_last = None;
        self.ctrl = [0; 5];
        self.latch = 0x80;
        self.irq = 1;
        self.lspc = Lspc::default();
        self.timer_counter = 0;
        self.timer_fraction = 0;
        self.p_bank = 0;
        self.p_window = None;
        self.sound.reset_requested = true;
    }

    fn report(&mut self, what: &str, addr: u32, size: Size, value: Option<u32>) {
        if self.lenient {
            if self.access_log.len() < 4000 {
                self.access_log
                    .push((addr, value.is_some(), value.unwrap_or(0), self.current_pc));
            }
            return;
        }
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
        let before_clock = self.clock;
        self.clock += master_clocks;
        let after = self.clock / LINE_CLOCKS;
        // the vertical blank begins at line 248 of every 264
        // the animation counter steps once every (speed + 1) frames unless stalled (fact N34)
        let frames = |n: u64| n / FRAME_LINES;
        for _ in frames(before)..frames(after) {
            self.anim_frames += 1;
            if self.lspc.mode & 8 == 0
                && self
                    .anim_frames
                    .is_multiple_of(u64::from(self.lspc.mode >> 8) + 1)
            {
                self.anim = (self.anim + 1) & 7;
            }
        }
        let blanks = |n: u64| (n + FRAME_LINES - VBLANK_LINE) / FRAME_LINES;
        let blank_at = if blanks(after) > blanks(before) {
            self.irq |= 4;
            self.irq_counts[0] += 1;
            // the clock at which the line counter passed the blank line
            Some((blanks(after) * FRAME_LINES - (FRAME_LINES - VBLANK_LINE)) * LINE_CLOCKS)
        } else {
            None
        };
        // the timer counts the pixel clocks; a reload at the start of the blank splits the span in two (fact N41)
        match blank_at {
            Some(at) if self.lspc.mode & 0x40 != 0 && at >= before_clock => {
                self.run_timer(at - before_clock);
                self.timer_counter = self.timer_reload();
                self.run_timer(self.clock - at);
            }
            _ => self.run_timer(master_clocks),
        }
    }

    fn timer_reload(&self) -> u32 {
        u32::from(self.lspc.timer_high) << 16 | u32::from(self.lspc.timer_low)
    }

    /// Counts `master_clocks` of the timer: it runs out at zero, raises level 2 when enabled and reloads when asked.
    fn run_timer(&mut self, master_clocks: u64) {
        self.timer_fraction += master_clocks;
        let mut pixels = self.timer_fraction / 4;
        self.timer_fraction %= 4;
        while pixels > 0 && self.timer_counter > 0 {
            let step = pixels.min(u64::from(self.timer_counter));
            self.timer_counter -= step as u32;
            pixels -= step;
            if self.timer_counter == 0 {
                if self.lspc.mode & 0x10 != 0 {
                    self.irq |= 2;
                    self.irq_counts[1] += 1;
                }
                if self.lspc.mode & 0x80 != 0 {
                    self.timer_counter = self.timer_reload();
                }
            }
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

    /// A byte as the debugger sees it: no side effects, `$FF` where nothing is.
    pub fn peek(&self, addr: u32) -> u8 {
        self.byte(addr).unwrap_or(0xFF)
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
                let at = if self.p.len() == 0x20_0000 {
                    a + 0x10_0000
                } else {
                    a
                };
                self.p.get(at).copied()
            }
            0x20_0000..=0x2F_FFFF if self.p.len() == 0x20_0000 => self.p.get(a & 0xF_FFFF).copied(),
            // a program with a banked part: the first MiB below, the rest in banks of 1 MiB at `$200000` (fact N47)
            // the PVC chip's RAM and the SMA chip's constant (fact N53)
            0x2F_E000..=0x2F_FFFF if self.board == neo_set::carts::Board::PvcMslug5 => {
                let w = self.pvc_ram[(a - 0x2F_E000) / 2];
                Some(if a & 1 == 0 { (w >> 8) as u8 } else { w as u8 })
            }
            0x2F_E446 | 0x2F_E447 if self.board == neo_set::carts::Board::SmaMslug3 => {
                Some(if a & 1 == 0 { 0x9A } else { 0x37 })
            }
            0x20_0000..=0x2F_FFFF if self.p.len() > 0x20_0000 => {
                let banks = (self.p.len() - 0x10_0000) / 0x10_0000;
                let base = self
                    .p_window
                    .unwrap_or(0x10_0000 + (self.p_bank % banks) * 0x10_0000);
                self.p.get(base + (a & 0xF_FFFF)).copied()
            }
            0x10_0000..=0x1F_FFFF => Some(self.work[a & (WORK_RAM - 1)]),
            0xC0_0000..=0xCF_FFFF => self.bios.get(a & (BIOS_SIZE - 1)).copied(),
            0x40_0000..=0x7F_FFFF => Some(self.palette[self.palette_at(a)]),
            0xD0_0000..=0xD0_FFFF => Some(self.backup[a & (BACKUP_RAM - 1)]),
            _ if a & 0xFE_0081 == 0x30_0001 => Some(self.dips),
            // REG_SYSTYPE / the test switch: bit 7 is the test button (active low), bit 0 the slot count (fact N23)
            _ if a & 0xFE_0081 == 0x30_0081 => Some(0xFF),
            // REG_STATUS_A: coin 1, coin 2 and service are active low and not pressed; bit 6 is the clock's time pulse (facts N18, N19)
            // REG_STATUS_B: start and select buttons, memory card status (active low), bit 7 set on an MVS (fact N22)
            0x38_0000..=0x38_FFFF if a & 1 == 0 && a & 0xFE_00F1 == 0x38_0000 => {
                Some(!self.pressed_b)
            }
            // the joypad ports (fact N28)
            _ if a & 0xFE_0001 == 0x30_0000 => Some(self.pads[0]),
            _ if a & 0xFE_0001 == 0x34_0000 => Some(self.pads[1]),
            // the write-only registers read as an open bus: a `clr` reads before it writes (fact N35)
            _ if a & 0xFE_0001 == 0x3A_0001 => Some(0xFF),
            _ if matches!(
                a & 0xFE_00F1,
                0x38_0001
                    | 0x38_0021
                    | 0x38_0031
                    | 0x38_0041
                    | 0x38_0051
                    | 0x38_0061
                    | 0x38_0063
                    | 0x38_0065
                    | 0x38_0067
                    | 0x38_00E1
                    | 0x38_00E3
                    | 0x38_00E5
                    | 0x38_00E7
            ) =>
            {
                Some(0xFF)
            }
            // REG_SOUND read: the Z80's reply (fact N25)
            _ if a & 0xFE_0001 == 0x32_0000 => Some(self.sound.reply),
            _ if a & 0xFE_0001 == 0x32_0001 => Some(
                (0x07 & !self.pressed_a)
                    | (((self.clock / TP_HALF_PERIOD) & 1) as u8) << 6
                    | u8::from(self.rtc.data_out) << 7,
            ),
            _ => None,
        }
    }

    fn put(&mut self, addr: u32, v: u8) -> bool {
        let a = (addr & 0xFF_FFFF) as usize;
        match a {
            // REG_SOUND written: a command for the Z80 (fact N25)
            _ if a & 0xFE_0001 == 0x32_0000 => self.sound.send_command(v),
            // REG_DIPSW written: any value kicks the watchdog (fact N11)
            _ if a & 0xFE_0001 == 0x30_0001 => {
                self.watchdog_kicks += 1;
                self.watchdog_last = Some(self.clock);
            }
            0x10_0000..=0x1F_FFFF => self.work[a & (WORK_RAM - 1)] = v,
            0x40_0000..=0x7F_FFFF => {
                let at = self.palette_at(a);
                self.palette[at] = v;
            }
            0xD0_0000..=0xD0_FFFF => {
                if (0xD0_0100..0xD0_0102).contains(&a) && self.rtc_log.len() < 6000 {
                    self.rtc_log.push(('B', v, self.current_pc));
                }
                self.backup[a & (BACKUP_RAM - 1)] = v;
            }
            // the system register latch: A3-A1 pick the bit and A4 is the value (fact N14)
            _ if a & 0xFE_0001 == 0x3A_0001 => {
                let (bit, on) = ((a >> 1) & 7, (a >> 4) & 1 != 0);
                self.latch = self.latch & !(1 << bit) | u8::from(on) << bit;
                self.sound.use_cart = self.latch & 0x20 != 0;
            }
            // the write-only output registers (fact N15)
            _ => match a & 0xFE_00F1 {
                0x38_0001 => self.ctrl[0] = v,
                0x38_0021 => self.ctrl[1] = v,
                0x38_0031 => self.ctrl[2] = v,
                0x38_0041 => self.ctrl[3] = v,
                0x38_0051 => {
                    self.ctrl[4] = v;
                    let now = rtc::add_seconds(
                        self.rtc_start.0,
                        self.rtc_start.1,
                        self.clock / 24_000_000,
                    );
                    self.rtc.write(v, &now);
                    if self.rtc_log.len() < 6000 {
                        self.rtc_log.push(('W', v, self.current_pc));
                    }
                }
                r @ (0x38_0061 | 0x38_0063 | 0x38_0065 | 0x38_0067) => {
                    self.coin[(r - 0x38_0061) / 2] = false
                }
                r @ (0x38_00E1 | 0x38_00E3 | 0x38_00E5 | 0x38_00E7) => {
                    self.coin[(r - 0x38_00E1) / 2] = true
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
        // the registers at `$3C0008-$3C000E` read like the ones four below them (fact N42)
        let reg = reg & 7;
        match reg {
            0x0 | 0x2 => Some(self.vram[vram_index(self.lspc.vram_addr)]),
            0x4 => Some(self.lspc.vram_mod),
            // REG_LSPCMODE read: the raster line counter in the upper nine bits, the auto-animation counter in the low three (fact N34)
            0x6 => {
                let line = (self.clock / LINE_CLOCKS) % FRAME_LINES;
                Some((((0xF8 + line) & 0x1FF) as u16) << 7 | u16::from(self.anim))
            }
            _ => None,
        }
    }

    fn lspc_write(&mut self, reg: u32, v: u16) -> bool {
        if reg == 0xC {
            // REG_IRQACK: bit 0 clears the reset interrupt, bit 1 the timer's, bit 2 the vertical blank's (fact N21)
            self.irq &= !(v as u8 & 7);
            self.irq_counts[2] += u64::from(v & 4 != 0);
            self.irq_counts[3] += u64::from(v & 2 != 0);
        }
        let l = &mut self.lspc;
        match reg {
            0x0 => l.vram_addr = v,
            0x2 => {
                if l.vram_addr >= 0x8800 {
                    self.vram_oob += 1;
                }
                self.vram[vram_index(l.vram_addr)] = v;
                l.vram_addr = l.vram_addr.wrapping_add(l.vram_mod);
            }
            0x4 => l.vram_mod = v,
            0x6 => l.mode = v,
            0x8 => l.timer_high = v,
            0xA => {
                l.timer_low = v;
                // bit 5 of the mode: the counter takes the reload value at once (fact N41)
                if l.mode & 0x20 != 0 {
                    self.timer_counter = u32::from(self.lspc.timer_high) << 16 | u32::from(v);
                }
            }
            0xC => l.irq_ack = v,
            0xE => l.timer_stop = v,
            _ => return false,
        }
        true
    }
}

impl M68kBus for Bus {
    fn read(&mut self, addr: u32, size: Size, _fc: FunctionCode) -> Result<u32, BusFault> {
        if size == Size::Byte && (addr & 0xFE_0001) == 0x32_0001 && self.rtc_log.len() < 6000 {
            let v = self.byte(addr).unwrap_or(0);
            if self.rtc_log.iter().rev().find(|e| e.0 == 'R').map(|e| e.1) != Some(v) {
                self.rtc_log.push(('R', v, self.current_pc));
            }
        }
        if size == Size::Byte
            && (0x20_0000..0x30_0000).contains(&addr)
            && let Some(v) = self.protection.read_byte(addr)
        {
            return Ok(u32::from(v));
        }
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
                Ok(if self.lenient { 0xFFFF } else { 0 })
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
        match self.board {
            // the PVC chip: its RAM at `$2FE000`, with the colour packing and the bank register inside it (fact N53)
            neo_set::carts::Board::PvcMslug5 if (0x2F_E000..0x30_0000).contains(&addr) => {
                let offset = (addr as usize - 0x2F_E000) / 2;
                let old = self.pvc_ram[offset];
                self.pvc_ram[offset] = match size {
                    Size::Word => value as u16,
                    Size::Byte if addr & 1 == 0 => (old & 0x00FF) | (value as u16 & 0xFF) << 8,
                    Size::Byte => (old & 0xFF00) | (value as u16 & 0xFF),
                };
                neo_set::carts::pvc_after_write(&mut self.pvc_ram, offset);
                if offset >= 0xff8 {
                    self.p_window = Some(neo_set::carts::pvc_bank_base(&mut self.pvc_ram));
                }
                return Ok(());
            }
            // the SMA chip: the bank register at `$2FFFE4` holds a scrambled number (fact N53)
            neo_set::carts::Board::SmaMslug3 if (0x2F_FFE0..0x30_0000).contains(&addr) => {
                if addr & !1 == 0x2F_FFE4 && size == Size::Word {
                    self.p_window = Some(neo_set::carts::mslug3_bank_base(value as u16));
                }
                return Ok(());
            }
            _ => {}
        }
        // the bank register of a banked program: a write to `$2FFFF0`-`$2FFFFF` (fact N47)
        if (0x2F_FFF0..0x30_0000).contains(&addr) {
            // a program of 2 MiB or less has nothing to switch (Sengoku 3 writes 0 here all the same, fact N47): the write is taken
            if self.p.len() > 0x20_0000 {
                self.p_bank = (value & 0xFF) as usize;
            }
            return Ok(());
        }
        if (0x20_0000..0x30_0000).contains(&addr) {
            let taken = match size {
                Size::Word => self.protection.write_word(addr, value as u16),
                Size::Byte => self.protection.write_byte(addr),
            };
            // a write no cartridge chip answers lands on program ROM and changes nothing (fact N52)
            if taken || self.p.len() > 0x10_0000 {
                return Ok(());
            }
        }
        // the cartridge's fixed program is ROM: a write there changes nothing (fact N52)
        if addr < 0x10_0000 {
            return Ok(());
        }
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

/// The Z80's view of the same machine: its bus is the sound side (facts N24-N27).
impl machine::Bus for Bus {
    fn read(&mut self, addr: u16) -> u8 {
        self.sound.read(addr)
    }

    fn write(&mut self, addr: u16, val: u8) {
        self.sound.write(addr, val);
    }

    fn read_io(&mut self, port: u16) -> u8 {
        self.sound.read_io(port)
    }

    fn write_io(&mut self, port: u16, val: u8) {
        self.sound.write_io(port, val);
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
        Bus::new(bios, p, vec![0; 0x8000], Vec::new())
    }

    #[test]
    fn the_watchdog_runs_only_after_a_kick_and_expires_without_the_next() {
        let mut b = bus();
        b.advance(WATCHDOG_CLOCKS * 2);
        assert!(!b.watchdog_expired(), "never kicked: not running");
        b.write(0x30_0001, Size::Byte, 0, FC).unwrap();
        b.advance(WATCHDOG_CLOCKS - 1000);
        assert!(!b.watchdog_expired());
        b.write(0x30_0001, Size::Byte, 0, FC).unwrap();
        b.advance(WATCHDOG_CLOCKS - 1000);
        assert!(!b.watchdog_expired(), "a kick starts the count again");
        b.advance(2000);
        assert!(b.watchdog_expired());
        b.watchdog_reset();
        assert!(!b.watchdog_expired());
        assert_eq!(b.watchdog_resets, 1);
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
    fn a_program_of_more_than_two_mib_shows_the_bank_a_write_to_2ffff0_selects() {
        let mib = 0x10_0000;
        let mut p = vec![0u8; 4 * mib];
        // each megabyte of the file starts with its own number (words stored byte-swapped as in the sets)
        for n in 0..4 {
            p[n * mib] = n as u8 + 1;
            p[n * mib + 1] = 0x00;
        }
        let mut b = Bus::new(vec![0u8; BIOS_SIZE], p, vec![0; 0x8000], Vec::new());
        b.write(0x3A_0013, Size::Byte, 0, FC).unwrap(); // the cartridge's vectors
        assert_eq!(
            b.read(0, Size::Word, FC),
            Ok(1),
            "the first megabyte stays below"
        );
        assert_eq!(
            b.read(0x20_0000, Size::Word, FC),
            Ok(2),
            "bank 0 is the second megabyte"
        );
        b.write(0x2F_FFF0, Size::Word, 1, FC).unwrap();
        assert_eq!(b.read(0x20_0000, Size::Word, FC), Ok(3));
        b.write(0x2F_FFF0, Size::Word, 2, FC).unwrap();
        assert_eq!(b.read(0x20_0000, Size::Word, FC), Ok(4));
        assert!(b.stop.is_none());
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

    /// Writes the timer's reload value and runs the bus for `pixels` pixel clocks (4 master clocks each).
    fn run_timer(b: &mut Bus, mode: u16, pixels: u64) {
        b.write(0x3C_0006, Size::Word, u32::from(mode), FC).unwrap();
        b.write(0x3C_0008, Size::Word, 0, FC).unwrap();
        b.write(0x3C_000A, Size::Word, 100, FC).unwrap(); // reload 100 (loaded at once when bit 5 is set)
        b.irq = 0;
        b.advance(pixels * 4);
    }

    #[test]
    fn the_timer_raises_level_2_when_it_runs_out_if_enabled_and_loaded_by_the_low_write() {
        let mut b = bus();
        run_timer(&mut b, 0x30, 99);
        assert_eq!(b.irq & 2, 0, "one pixel clock left");
        b.advance(4);
        assert_eq!(b.irq & 2, 2);
        assert_eq!(b.ipl(), 2);
        // disabled: nothing
        let mut b = bus();
        run_timer(&mut b, 0x20, 200);
        assert_eq!(b.irq & 2, 0);
    }

    #[test]
    fn with_the_repeat_bit_the_timer_reloads_and_fires_again() {
        let mut b = bus();
        run_timer(&mut b, 0xB0, 100);
        assert_eq!(b.irq & 2, 2);
        b.write(0x3C_000C, Size::Word, 2, FC).unwrap(); // acknowledge
        assert_eq!(b.irq & 2, 0);
        b.advance(100 * 4);
        assert_eq!(b.irq & 2, 2);
    }

    #[test]
    fn without_the_repeat_bit_it_fires_once() {
        let mut b = bus();
        run_timer(&mut b, 0x30, 100);
        b.write(0x3C_000C, Size::Word, 2, FC).unwrap();
        b.advance(1000 * 4);
        assert_eq!(b.irq & 2, 0);
    }

    #[test]
    fn the_timer_reloads_at_the_start_of_the_vertical_blank_when_asked() {
        let mut b = bus();
        b.write(0x3C_0006, Size::Word, 0x50, FC).unwrap(); // enable, reload at the blank
        b.write(0x3C_0008, Size::Word, 0, FC).unwrap();
        b.write(0x3C_000A, Size::Word, 400, FC).unwrap(); // a reload value, not loaded yet
        b.irq = 0;
        // run to line 248 (the reload), then 399 pixel clocks later nothing, one more and it fires
        b.advance(VBLANK_LINE * LINE_CLOCKS);
        b.irq = 0;
        b.advance(399 * 4);
        assert_eq!(b.irq & 2, 0);
        b.advance(4);
        assert_eq!(b.irq & 2, 2);
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
        b.write(0xE0_0000, Size::Byte, 0, FC).unwrap();
        assert!(b.stop.as_deref().unwrap().contains("$E00000"));
    }
}
