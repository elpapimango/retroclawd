//! The NEC PC Engine / TurboGrafx-16 with a HuCard: the HuC6280 on a bus of 8 KB of RAM, the card, the
//! HuC6270 video controller, the HuC6260 colour encoder and the sound generator, plus the timer, the interrupt
//! registers and the joypad port that sit inside the CPU chip. Facts: `machines/nec/docs/hardware-facts.md`.
//!
//! Time is kept in master clocks (21.477272 MHz): the CPU takes 3 per cycle at high speed and 12 at low speed,
//! a scan line is 1365 of them (455 CPU cycles) and a frame 262 or 263 lines.
#![forbid(unsafe_code)]

use huc6260::Vce;
use huc6270::Vdc;
use huc6280::{Bus, Cpu, IRQ1, IRQ2, TIMER};
use huc6280_psg::Psg;
use pce_card::Card;
use serde::{Deserialize, Serialize};

pub const MASTER_HZ: u64 = 21_477_272;
pub const LINE_CLOCKS: u32 = 1365;
/// The nominal frame: 263 lines (the colour encoder's long frame, which every game seen sets).
pub const FRAME_LINES: u32 = 263;
pub const FRAME_CLOCKS: u32 = LINE_CLOCKS * FRAME_LINES;
pub const SAMPLE_RATE: u32 = huc6280_psg::SAMPLE_RATE;
/// The picture is always 512 dots wide: narrower modes are widened.
pub const CANVAS_W: usize = 512;
const CANVAS_H: usize = 243;
/// The first frame line in the picture window: 3 lines above the first active line of the usual vertical registers
/// (`VPR` $0F02: sync 2, start 15, first active line 2 + 15 + 2 = 19) (fact C3).
const WINDOW_FIRST_LINE: i32 = 16;
/// Timer: one tick every 1024 high-speed CPU cycles.
const TIMER_CLOCKS: u32 = 3072;
/// A read of the timer counter falls this many CPU cycles after the start of its instruction (the last cycle of
/// `LDA abs`) (fact M11).
const TIMER_READ_CYCLES: u32 = 4;
/// For this many master clocks after an underflow the counter reads `$7F` before the reload value is in (fact M11).
const TIMER_UNDERFLOW_SHOWN: u32 = 3;
/// The backup unit's RAM: 2 KB at the start of bank `$F7` (fact M9).
pub const BRAM_SIZE: usize = 0x800;
const BRAM_BANK: u8 = 0xF7;

/// A backup RAM as the System Card's format leaves it empty: `HUBM`, the end of the RAM (`$8800` in the window the
/// card maps it to) and the first free byte (`$8010`) (fact M10).
#[must_use]
pub fn formatted_bram() -> Vec<u8> {
    let mut b = vec![0; BRAM_SIZE];
    b[..8].copy_from_slice(&[b'H', b'U', b'B', b'M', 0x00, 0x88, 0x10, 0x80]);
    b
}

/// Which console to be: the I/O port reports it to the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Console {
    PcEngine,
    TurboGrafx16,
}

#[derive(Serialize, Deserialize)]
struct Hw {
    ram: Vec<u8>,
    vdc: Vdc,
    vce: Vce,
    psg: Psg,
    timer_reload: u8,
    timer_counter: u8,
    timer_enabled: bool,
    timer_acc: u32,
    timer_irq: bool,
    /// The last tick was an underflow (fact M11).
    timer_underflowed: bool,
    /// The CPU's buffer for its own ports (`$0800-$17FF`): the last byte written or read there; the bits a register
    /// does not drive read from it (fact M12).
    io_buffer: u8,
    /// Master clocks from the start of the instruction running to its timer read (fact M11).
    #[serde(skip)]
    read_delay: u32,
    irq_disable: u8,
    io_sel: bool,
    io_clr: bool,
    io_counter: u8,
    /// A multitap is plugged in: the counter picks the pad. Without one the port reads the first pad always.
    multitap: bool,
    /// Buttons held per pad (bit set = pressed): I, II, Select, Run, Up, Right, Down, Left.
    pads: [u8; 5],
    console: Console,
    /// The backup unit (fact M9): plugged in, its RAM, and whether the RAM is unlocked.
    backup_unit: bool,
    bram: Vec<u8>,
    bram_unlocked: bool,
    /// A SuperGrafx: its second video controller and priority controller (`ram` is then 32 KB).
    sgx: Option<Box<Sgx>>,
}

/// The SuperGrafx's additions (facts S1-S3): a second video controller and the video priority controller.
#[derive(Clone, Serialize, Deserialize)]
struct Sgx {
    vdc2: Vdc,
    /// `$0008`, `$0009`: four priority nibbles, one per window region.
    prio: [u8; 2],
    /// `$000A-$000B` and `$000C-$000D`: the two windows' widths (10 bits).
    win: [u16; 2],
    /// `$000E` bit 0: the ST0/ST1/ST2 instructions go to the second controller.
    st_vdc2: bool,
}

impl Sgx {
    fn new() -> Self {
        Self {
            vdc2: Vdc::new(),
            prio: [0; 2],
            win: [0; 2],
            st_vdc2: false,
        }
    }

    fn read(&self, port: u16) -> u8 {
        match port & 7 {
            0 => self.prio[0],
            1 => self.prio[1],
            2 => self.win[0] as u8,
            3 => (self.win[0] >> 8) as u8,
            4 => self.win[1] as u8,
            5 => (self.win[1] >> 8) as u8,
            6 => u8::from(self.st_vdc2),
            _ => 0xFF,
        }
    }

    fn write(&mut self, port: u16, v: u8) {
        let set_lo = |w: &mut u16| *w = (*w & 0x300) | u16::from(v);
        let set_hi = |w: &mut u16| *w = (*w & 0xFF) | (u16::from(v & 3) << 8);
        match port & 7 {
            0 => self.prio[0] = v,
            1 => self.prio[1] = v,
            2 => set_lo(&mut self.win[0]),
            3 => set_hi(&mut self.win[0]),
            4 => set_lo(&mut self.win[1]),
            5 => set_hi(&mut self.win[1]),
            6 => self.st_vdc2 = v & 1 != 0,
            _ => {}
        }
    }

    /// Combines the two controllers' lines dot by dot (facts S2, S3). A dot value is a palette index: 0 is
    /// transparent (the backdrop), `$100` and up a sprite.
    fn mix(&self, line1: &mut [u16], line2: &[u16]) {
        for (x, p) in line1.iter_mut().enumerate() {
            // inside a window when x < width - 64; the region picks one of the four nibbles
            let inside = |w: u16| (x as i32) < i32::from(w) - 64;
            let region =
                usize::from(!inside(self.win[0])) | (usize::from(!inside(self.win[1])) << 1);
            let nibble = (self.prio[region >> 1] >> ((region & 1) * 4)) & 0x0F;
            let a = if nibble & 1 != 0 { *p } else { 0 };
            let b = if nibble & 2 != 0 {
                line2.get(x).copied().unwrap_or(0)
            } else {
                0
            };
            let opaque = |v: u16| v & 0x0F != 0;
            let sprite = |v: u16| v >= 0x100;
            *p = match (nibble >> 2) & 3 {
                // the second controller's sprites in front of the first's background
                1 if opaque(b) && sprite(b) && !(opaque(a) && sprite(a)) => b,
                // the second controller's background in front of the first's sprites
                2 if opaque(a) && sprite(a) && opaque(b) && !sprite(b) => b,
                _ if opaque(a) => a,
                _ if opaque(b) => b,
                _ => 0,
            };
        }
    }
}

/// What the CPU sees on its bus: the card, plus everything in `Hw`.
struct SysBus<'a> {
    card: &'a mut Card,
    hw: &'a mut Hw,
}

impl Hw {
    fn irq_request(&self) -> u8 {
        let mut r = 0;
        if self.vdc.irq() || self.sgx.as_ref().is_some_and(|s| s.vdc2.irq()) {
            r |= IRQ1;
        }
        if self.timer_irq {
            r |= TIMER;
        }
        r
    }

    fn io_read(&self) -> u8 {
        let index = if self.multitap {
            usize::from(self.io_counter)
        } else {
            0
        };
        let pad = self.pads.get(index).copied().unwrap_or(0);
        let nibble = if self.io_sel {
            // directions: up, right, down, left
            (pad >> 4) & 0x0F
        } else {
            pad & 0x0F
        };
        let region = match self.console {
            Console::PcEngine => 0x00,
            Console::TurboGrafx16 => 0x40,
        };
        // bits 4-5: no CD-ROM unit attached; bit 7 unused
        (!nibble & 0x0F) | 0x30 | region | 0x80
    }

    /// The RAM index of an address in banks `$F8-$FB`: one 8 KB repeated, or 32 KB on a SuperGrafx (fact S1).
    fn ram_index(&self, bank: u8, off: u16) -> usize {
        if self.sgx.is_some() {
            usize::from(bank - 0xF8) * 0x2000 + usize::from(off)
        } else {
            usize::from(off)
        }
    }

    fn hw_read(&mut self, offset: u16) -> u8 {
        match offset >> 10 {
            // a SuperGrafx decodes A3-A4: first controller, priority controller, second controller (fact S1)
            0 if let Some(s) = &mut self.sgx => match (offset >> 3) & 3 {
                0 => self.vdc.read((offset & 3) as u8),
                1 => s.read(offset),
                2 => s.vdc2.read((offset & 3) as u8),
                _ => 0xFF,
            },
            0 => self.vdc.read((offset & 3) as u8),
            1 => self.vce.read((offset & 7) as u8),
            // the sound chip's registers are write-only: a read gives the buffer (fact M12)
            2 => self.io_buffer,
            3 => {
                let v = if offset & 1 == 0 {
                    (self.io_buffer & 0x80) | self.timer_read()
                } else {
                    (self.io_buffer & 0xFE) | u8::from(self.timer_enabled)
                };
                self.io_buffer = v;
                v
            }
            4 => {
                let v = self.io_read();
                self.io_buffer = v;
                v
            }
            5 => {
                let v = match offset & 3 {
                    2 => (self.io_buffer & 0xF8) | self.irq_disable,
                    3 => (self.io_buffer & 0xF8) | self.irq_request(),
                    _ => self.io_buffer,
                };
                self.io_buffer = v;
                v
            }
            6 => {
                // a read of `$1803` locks the backup RAM (fact M9); what the registers read is not known here
                if self.backup_unit && offset & 0x0F == 3 {
                    self.bram_unlocked = false;
                }
                0xFF
            }
            _ => 0xFF,
        }
    }

    fn hw_write(&mut self, offset: u16, v: u8) {
        if (2..=5).contains(&(offset >> 10)) {
            self.io_buffer = v;
        }
        match offset >> 10 {
            0 if let Some(s) = &mut self.sgx => match (offset >> 3) & 3 {
                0 => self.vdc.write((offset & 3) as u8, v),
                1 => s.write(offset, v),
                2 => s.vdc2.write((offset & 3) as u8, v),
                _ => {}
            },
            0 => self.vdc.write((offset & 3) as u8, v),
            1 => self.vce.write((offset & 7) as u8, v),
            2 => self.psg.write((offset & 15) as u8, v),
            3 => {
                if offset & 1 == 0 {
                    self.timer_reload = v & 0x7F;
                } else {
                    let on = v & 1 != 0;
                    if on && !self.timer_enabled {
                        self.timer_counter = self.timer_reload;
                        self.timer_acc = 0;
                    }
                    self.timer_enabled = on;
                }
            }
            4 => {
                let sel = v & 1 != 0;
                let clr = v & 2 != 0;
                if clr {
                    self.io_counter = 0;
                } else if sel && !self.io_sel {
                    self.io_counter = self.io_counter.saturating_add(1);
                }
                self.io_sel = sel;
                self.io_clr = clr;
            }
            5 => match offset & 3 {
                2 => self.irq_disable = v & 7,
                3 => self.timer_irq = false,
                _ => {}
            },
            // a write of `$1807` with bit 7 set unlocks the backup RAM (fact M9)
            6 if self.backup_unit && offset & 0x0F == 7 && v & 0x80 != 0 => {
                self.bram_unlocked = true;
            }
            _ => {}
        }
    }

    /// The counter as a read sees it, at its cycle inside the instruction: a tick due before that cycle has happened,
    /// and just after an underflow the counter shows `$7F` (fact M11).
    fn timer_read(&self) -> u8 {
        if !self.timer_enabled {
            return self.timer_counter & 0x7F;
        }
        let t = self.timer_acc + self.read_delay;
        if t >= TIMER_CLOCKS {
            let since = t - TIMER_CLOCKS;
            return if self.timer_counter != 0 {
                self.timer_counter - 1
            } else if since < TIMER_UNDERFLOW_SHOWN {
                0x7F
            } else {
                self.timer_reload
            };
        }
        if self.timer_underflowed && t < TIMER_UNDERFLOW_SHOWN {
            return 0x7F;
        }
        self.timer_counter & 0x7F
    }

    /// The backup RAM byte at `off` of bank `$F7`, when the unit is there and unlocked (fact M9).
    fn bram_slot(&mut self, off: u16) -> Option<&mut u8> {
        if self.backup_unit && self.bram_unlocked {
            self.bram.get_mut(usize::from(off))
        } else {
            None
        }
    }
}

impl Bus for SysBus<'_> {
    fn read(&mut self, addr: u32) -> u8 {
        let bank = (addr >> 13) as u8;
        let off = (addr & 0x1FFF) as u16;
        match bank {
            0x00..=0x7F => self.card.read(bank, off),
            BRAM_BANK => self.hw.bram_slot(off).map_or(0xFF, |b| *b),
            0xF8..=0xFB => self.hw.ram[self.hw.ram_index(bank, off)],
            0xFF => self.hw.hw_read(off),
            _ => 0xFF,
        }
    }

    fn write(&mut self, addr: u32, v: u8) {
        let bank = (addr >> 13) as u8;
        let off = (addr & 0x1FFF) as u16;
        match bank {
            0x00..=0x7F => self.card.write(bank, off, v),
            BRAM_BANK => {
                if let Some(b) = self.hw.bram_slot(off) {
                    *b = v;
                }
            }
            0xF8..=0xFB => {
                let i = self.hw.ram_index(bank, off);
                self.hw.ram[i] = v;
            }
            0xFF => self.hw.hw_write(off, v),
            _ => {}
        }
    }

    fn irq_lines(&self) -> u8 {
        self.hw.irq_request() & !self.hw.irq_disable & (IRQ1 | TIMER | IRQ2)
    }

    fn st(&mut self, port: u8, v: u8) {
        match &mut self.hw.sgx {
            Some(s) if s.st_vdc2 => s.vdc2.write(port, v),
            _ => self.hw.vdc.write(port, v),
        }
    }
}

/// The save state's magic and format: 2 since the backup unit.
const STATE_MAGIC: &[u8] = b"RCPC\x02";

#[derive(Serialize, Deserialize)]
struct SaveState {
    cpu: Cpu,
    hw: Hw,
    line: u32,
    line_clock: u32,
    block: u8,
    card_len: usize,
}

pub struct PcEngine {
    pub cpu: Cpu,
    card: Card,
    hw: Hw,
    line: u32,
    line_clock: u32,
    canvas: Vec<u8>,
    line_buf: Vec<u16>,
}

impl PcEngine {
    /// Builds a console around a HuCard image.
    pub fn from_card(bytes: Vec<u8>, console: Console) -> Result<Self, String> {
        let card = Card::from_bytes(bytes).map_err(|e| e.to_string())?;
        let mut e = Self {
            cpu: Cpu::new(),
            card,
            hw: Hw {
                ram: vec![0; 0x2000],
                vdc: Vdc::new(),
                vce: Vce::new(),
                psg: Psg::new(),
                timer_reload: 0,
                timer_counter: 0x7F,
                timer_enabled: false,
                timer_acc: 0,
                timer_irq: false,
                timer_underflowed: false,
                io_buffer: 0,
                read_delay: 0,
                irq_disable: 0,
                io_sel: false,
                io_clr: false,
                io_counter: 0,
                multitap: false,
                pads: [0; 5],
                console,
                backup_unit: true,
                bram: formatted_bram(),
                bram_unlocked: false,
                sgx: None,
            },
            line: 0,
            line_clock: 0,
            canvas: vec![0; CANVAS_W * CANVAS_H * 3],
            line_buf: Vec::new(),
        };
        e.reset();
        Ok(e)
    }

    /// Plugs in (or removes) a multitap so pads 2-5 can be read.
    pub fn set_multitap(&mut self, on: bool) {
        self.hw.multitap = on;
    }

    /// Makes the console a SuperGrafx (or a plain PC Engine again): the second video controller, the priority
    /// controller and 32 KB of RAM (fact S1). Done before running; it starts the RAM afresh.
    pub fn set_supergrafx(&mut self, on: bool) {
        if on {
            self.hw.sgx = Some(Box::new(Sgx::new()));
            self.hw.ram = vec![0; 0x8000];
        } else {
            self.hw.sgx = None;
            self.hw.ram = vec![0; 0x2000];
        }
    }

    /// Whether the console is a SuperGrafx.
    #[must_use]
    pub fn is_supergrafx(&self) -> bool {
        self.hw.sgx.is_some()
    }

    /// Plugs in the backup unit with this RAM (`BRAM_SIZE` bytes), or removes it (`None`).
    pub fn set_backup_ram(&mut self, bram: Option<Vec<u8>>) -> Result<(), String> {
        match bram {
            Some(b) if b.len() != BRAM_SIZE => Err(format!(
                "the backup RAM is {BRAM_SIZE} bytes, not {}",
                b.len()
            )),
            Some(b) => {
                self.hw.backup_unit = true;
                self.hw.bram = b;
                Ok(())
            }
            None => {
                self.hw.backup_unit = false;
                self.hw.bram = formatted_bram();
                Ok(())
            }
        }
    }

    /// The backup unit's RAM, when one is plugged in.
    #[must_use]
    pub fn backup_ram(&self) -> Option<&[u8]> {
        self.hw.backup_unit.then_some(&self.hw.bram[..])
    }

    pub fn reset(&mut self) {
        let mut bus = SysBus {
            card: &mut self.card,
            hw: &mut self.hw,
        };
        self.cpu = Cpu::new();
        self.cpu.reset(&mut bus);
        self.line = 0;
        self.line_clock = 0;
    }

    /// Runs one instruction (or interrupt entry); returns the master clocks it took.
    pub fn step(&mut self) -> u32 {
        self.hw.read_delay = TIMER_READ_CYCLES * if self.cpu.fast { 3 } else { 12 };
        let mut bus = SysBus {
            card: &mut self.card,
            hw: &mut self.hw,
        };
        let cycles = self.cpu.step(&mut bus);
        let master = cycles * if self.cpu.fast { 3 } else { 12 };
        self.advance(master);
        master
    }

    fn advance(&mut self, master: u32) {
        let hw = &mut self.hw;
        hw.psg.run(u64::from(master));
        if hw.timer_enabled {
            hw.timer_acc += master;
            while hw.timer_acc >= TIMER_CLOCKS {
                hw.timer_acc -= TIMER_CLOCKS;
                hw.timer_underflowed = hw.timer_counter == 0;
                if hw.timer_counter == 0 {
                    hw.timer_counter = hw.timer_reload;
                    hw.timer_irq = true;
                } else {
                    hw.timer_counter -= 1;
                }
            }
        }
        self.line_clock += master;
        while self.line_clock >= LINE_CLOCKS {
            self.line_clock -= LINE_CLOCKS;
            self.next_line();
        }
    }

    fn next_line(&mut self) {
        let lines = u32::from(self.hw.vdc.frame_lines());
        let line = self.line;
        let mut buf = std::mem::take(&mut self.line_buf);
        let drew = self.hw.vdc.start_line(line as u16, &mut buf).is_some();
        if let Some(s) = &mut self.hw.sgx {
            // both controllers run from the same timing; the second one's line is laid under or over the first's
            let mut buf2 = Vec::new();
            s.vdc2.start_line(line as u16, &mut buf2);
            if drew {
                s.mix(&mut buf, &buf2);
            }
        }
        // the picture window is a fixed stretch of the frame's lines (fact C3): it starts 3 lines above where the usual
        // vertical registers (`VPR` $0F02) put the first active line, wherever a game puts its picture; the lines
        // outside the picture show the border colour
        let out_row = line as i32 - WINDOW_FIRST_LINE;
        if (0..CANVAS_H as i32).contains(&out_row) {
            let base = out_row as usize * CANVAS_W * 3;
            if drew {
                // the window is the dot clock's nominal width; a narrower picture sits in its middle with the border on
                // both sides, a wider one starts at the left edge and is cropped on the right (fact C5)
                let nominal: usize = match self.hw.vce.dot_clock() {
                    0 => 256,
                    1 => 352,
                    _ => 512,
                };
                let width = buf.len();
                let left = nominal.saturating_sub(width) / 2;
                let border = self.hw.vce.rgb(0x100);
                for x in 0..CANVAS_W {
                    let dot = x * nominal / CANVAS_W;
                    let rgb = match dot.checked_sub(left) {
                        Some(i) if i < width => self.hw.vce.rgb(buf[i]),
                        _ => border,
                    };
                    self.canvas[base + x * 3..base + x * 3 + 3].copy_from_slice(&rgb);
                }
            } else {
                // outside the picture the border shows palette entry $100
                let border = self.hw.vce.rgb(0x100);
                for x in 0..CANVAS_W {
                    self.canvas[base + x * 3..base + x * 3 + 3].copy_from_slice(&border);
                }
            }
        }
        self.line_buf = buf;
        self.line += 1;
        if self.line >= lines {
            self.line = 0;
        }
    }

    /// The picture: 512 dots wide and 243 lines, the window the reference emulators show (3 lines above the first
    /// active line and 240 below it; a taller display is cropped).
    #[must_use]
    pub fn render_rgb(&self) -> (u32, u32, Vec<u8>) {
        (CANVAS_W as u32, CANVAS_H as u32, self.canvas.clone())
    }

    pub fn drain_audio(&mut self) -> Vec<f32> {
        self.hw.psg.drain()
    }

    /// Sets a pad button by key name; false for an unknown name.
    pub fn set_key(&mut self, key: &str, pressed: bool) -> bool {
        let (pad, bit) = match key {
            "Joy1B1" | "Joy1I" => (0, 0x01),
            "Joy1B2" | "Joy1II" => (0, 0x02),
            "Joy1Select" => (0, 0x04),
            "Joy1Run" | "Start1" => (0, 0x08),
            "Joy1Up" => (0, 0x10),
            "Joy1Right" => (0, 0x20),
            "Joy1Down" => (0, 0x40),
            "Joy1Left" => (0, 0x80),
            "Joy2B1" | "Joy2I" => (1, 0x01),
            "Joy2B2" | "Joy2II" => (1, 0x02),
            "Joy2Select" => (1, 0x04),
            "Joy2Run" | "Start2" => (1, 0x08),
            "Joy2Up" => (1, 0x10),
            "Joy2Right" => (1, 0x20),
            "Joy2Down" => (1, 0x40),
            "Joy2Left" => (1, 0x80),
            _ => return false,
        };
        if pressed {
            self.hw.pads[pad] |= bit;
        } else {
            self.hw.pads[pad] &= !bit;
        }
        true
    }

    /// A read with no side effects for debuggers: a physical address.
    #[must_use]
    pub fn peek(&self, addr: u32) -> u8 {
        let bank = (addr >> 13) as u8;
        let off = (addr & 0x1FFF) as u16;
        match bank {
            0x00..=0x7F => self.card.read(bank, off),
            BRAM_BANK if self.hw.backup_unit => {
                self.hw.bram.get(usize::from(off)).copied().unwrap_or(0xFF)
            }
            0xF8..=0xFB => self.hw.ram[self.hw.ram_index(bank, off)],
            _ => 0xFF,
        }
    }

    /// A read of a logical (CPU) address through the mapping registers.
    #[must_use]
    pub fn peek_logical(&self, addr: u16) -> u8 {
        self.peek(self.cpu.physical(addr))
    }

    #[must_use]
    pub fn vdc(&self) -> &Vdc {
        &self.hw.vdc
    }

    #[must_use]
    pub fn vce(&self) -> &Vce {
        &self.hw.vce
    }

    /// The timer's reload value and whether it runs (for debuggers).
    #[must_use]
    pub fn timer(&self) -> (u8, bool) {
        (self.hw.timer_reload, self.hw.timer_enabled)
    }

    /// The interrupt disable mask and whether the timer request is pending (for debuggers).
    #[must_use]
    pub fn irq_state(&self) -> (u8, bool, u8) {
        (
            self.hw.irq_disable,
            self.hw.timer_irq,
            self.hw.timer_counter,
        )
    }

    /// The scan line the beam is on.
    #[must_use]
    pub fn line(&self) -> u32 {
        self.line
    }

    pub fn save_state(&self) -> Vec<u8> {
        let state = SaveState {
            cpu: self.cpu,
            hw: Hw {
                ram: self.hw.ram.clone(),
                vdc: self.hw.vdc.clone(),
                vce: self.hw.vce.clone(),
                psg: self.hw.psg.clone(),
                timer_reload: self.hw.timer_reload,
                timer_counter: self.hw.timer_counter,
                timer_enabled: self.hw.timer_enabled,
                timer_acc: self.hw.timer_acc,
                timer_irq: self.hw.timer_irq,
                timer_underflowed: self.hw.timer_underflowed,
                io_buffer: self.hw.io_buffer,
                read_delay: 0,
                irq_disable: self.hw.irq_disable,
                io_sel: self.hw.io_sel,
                io_clr: self.hw.io_clr,
                io_counter: self.hw.io_counter,
                multitap: self.hw.multitap,
                pads: self.hw.pads,
                console: self.hw.console,
                backup_unit: self.hw.backup_unit,
                bram: self.hw.bram.clone(),
                bram_unlocked: self.hw.bram_unlocked,
                sgx: self.hw.sgx.clone(),
            },
            line: self.line,
            line_clock: self.line_clock,
            block: self.card.block(),
            card_len: self.card.len(),
        };
        let mut out = STATE_MAGIC.to_vec();
        out.extend(
            bincode::serde::encode_to_vec(&state, bincode::config::standard())
                .expect("state encodes"),
        );
        out
    }

    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.starts_with(b"RCPC\x01") {
            return Err(
                "the save state was made before the backup unit (format 1) and cannot be read"
                    .to_string(),
            );
        }
        let body = bytes
            .strip_prefix(STATE_MAGIC)
            .ok_or_else(|| "not a PC Engine save state".to_string())?;
        let (state, _): (SaveState, usize) =
            bincode::serde::decode_from_slice(body, bincode::config::standard())
                .map_err(|e| format!("corrupt save state: {e}"))?;
        if state.card_len != self.card.len() {
            return Err("the save state is for a different card".to_string());
        }
        if state.hw.sgx.is_some() != self.hw.sgx.is_some() {
            return Err(
                "the save state is for the other console (PC Engine / SuperGrafx)".to_string(),
            );
        }
        self.cpu = state.cpu;
        self.hw = state.hw;
        self.line = state.line;
        self.line_clock = state.line_clock;
        self.card.set_block(state.block);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 128 KB card whose reset code sets up a tiny picture and loops.
    fn tiny_card() -> Vec<u8> {
        let mut rom = vec![0xEAu8; 0x20000];
        // reset at $E000: CSH; SEI; LDA #$F8; TAM #$02 (map RAM at $4000)... then spin
        let code: &[u8] = &[0xD4, 0x78, 0x4C, 0x02, 0xE0];
        rom[..code.len()].copy_from_slice(code);
        rom[0x1FFE] = 0x00;
        rom[0x1FFF] = 0xE0;
        rom
    }

    #[test]
    fn it_resets_into_the_card() {
        let e = PcEngine::from_card(tiny_card(), Console::PcEngine).unwrap();
        assert_eq!(e.cpu.pc, 0xE000);
        assert!(!e.cpu.fast);
    }

    #[test]
    fn a_frame_is_a_whole_number_of_lines() {
        let mut e = PcEngine::from_card(tiny_card(), Console::PcEngine).unwrap();
        let mut clocks = 0u64;
        while clocks < u64::from(FRAME_CLOCKS) * 3 {
            clocks += u64::from(e.step());
        }
        assert!(e.line() < 263);
        assert!(e.vdc().frame >= 3);
    }

    #[test]
    fn the_timer_raises_its_interrupt() {
        let mut rom = vec![0xEAu8; 0x20000];
        // LDA #$02: STA $0C00 (reload 2); LDA #1: STA $0C01 (start): needs bank $FF mapped at $0000..:
        // LDA #$FF; TAM #$01; LDA #2; STA $0000+$0C00 -> use page 0 at $0000
        let code: &[u8] = &[
            0xA9, 0xFF, 0x53, 0x01, // map bank $FF at $0000
            0xA9, 0x02, 0x8D, 0x00, 0x0C, // reload
            0xA9, 0x01, 0x8D, 0x01, 0x0C, // enable
            0x4C, 0x0E, 0xE0, // spin
        ];
        rom[..code.len()].copy_from_slice(code);
        rom[0x1FFE] = 0x00;
        rom[0x1FFF] = 0xE0;
        let mut e = PcEngine::from_card(rom, Console::PcEngine).unwrap();
        for _ in 0..2000 {
            e.step();
        }
        assert!(e.hw.timer_irq);
    }

    /// Bank `$F7` reads `$FF` until `$1807` is written with bit 7 set; a read of `$1803` locks it again (fact M9).
    #[test]
    fn the_backup_ram_is_behind_a_lock() {
        let mut e = PcEngine::from_card(tiny_card(), Console::PcEngine).unwrap();
        let bram = 0xF7 << 13;
        let mut bus = SysBus {
            card: &mut e.card,
            hw: &mut e.hw,
        };
        assert_eq!(bus.read(bram), 0xFF);
        bus.write(bram + 0x20, 0x55);
        bus.write((0xFF << 13) | 0x1807, 0x80);
        assert_eq!(bus.read(bram), b'H');
        assert_eq!(bus.read(bram + 0x20), 0, "the write while locked was lost");
        bus.write(bram + 0x20, 0x55);
        assert_eq!(bus.read(bram + 0x20), 0x55);
        assert_eq!(bus.read(bram + 0x800), 0xFF, "past the 2 KB");
        bus.read((0xFF << 13) | 0x1803);
        assert_eq!(bus.read(bram + 0x20), 0xFF);
        assert_eq!(e.backup_ram().unwrap()[0x20], 0x55);
        // without the unit nothing unlocks
        e.set_backup_ram(None).unwrap();
        let mut bus = SysBus {
            card: &mut e.card,
            hw: &mut e.hw,
        };
        bus.write((0xFF << 13) | 0x1807, 0x80);
        assert_eq!(bus.read(bram), 0xFF);
    }

    /// The priority controller's four modes on one dot each (fact S2) and the window regions (fact S3).
    #[test]
    fn the_supergrafx_mixes_two_lines() {
        let mut s = Sgx::new();
        let bg1 = 0x001;
        let spr1 = 0x101;
        let bg2 = 0x011;
        let spr2 = 0x111;
        let one = |s: &Sgx, a: u16, b: u16| {
            let mut l = [a];
            s.mix(&mut l, &[b]);
            l[0]
        };
        // both windows 0: every dot is in region 3, the high nibble of $0009
        s.prio = [0, 0x30];
        assert_eq!(one(&s, bg1, spr2), bg1);
        assert_eq!(one(&s, 0, spr2), spr2);
        s.prio = [0, 0x70];
        assert_eq!(
            one(&s, bg1, spr2),
            spr2,
            "mode 1: VDC 2 sprites over VDC 1 background"
        );
        assert_eq!(one(&s, spr1, spr2), spr1);
        assert_eq!(one(&s, bg1, bg2), bg1);
        s.prio = [0, 0xB0];
        assert_eq!(
            one(&s, spr1, bg2),
            bg2,
            "mode 2: VDC 2 background over VDC 1 sprites"
        );
        assert_eq!(one(&s, spr1, spr2), spr1);
        assert_eq!(one(&s, bg1, spr2), bg1);
        s.prio = [0, 0x10];
        assert_eq!(one(&s, 0, spr2), 0, "VDC 2 off");
        // windows: dot 0 inside window 1 only (width $41) -> region 2, the low nibble of $0009
        s.win = [0x41, 0];
        s.prio = [0x00, 0x02];
        assert_eq!(one(&s, bg1, bg2), bg2);
    }

    #[test]
    fn supergrafx_ram_is_four_banks() {
        let mut e = PcEngine::from_card(tiny_card(), Console::PcEngine).unwrap();
        e.set_supergrafx(true);
        let mut bus = SysBus {
            card: &mut e.card,
            hw: &mut e.hw,
        };
        bus.write(0xF8 << 13, 1);
        bus.write(0xF9 << 13, 2);
        assert_eq!(bus.read(0xF8 << 13), 1);
        assert_eq!(bus.read(0xF9 << 13), 2);
        // the priority controller answers at $0008-$000E, the first controller still at $0000
        bus.write((0xFF << 13) | 0x0009, 0x37);
        bus.write((0xFF << 13) | 0x000B, 0xFF);
        bus.write((0xFF << 13) | 0x000E, 0x01);
        let s = e.hw.sgx.as_ref().unwrap();
        assert_eq!((s.prio[1], s.win[0], s.st_vdc2), (0x37, 0x300, true));
    }

    /// The Download card stores the video status in `$1403` and reads bit 5 back from it (fact M12).
    #[test]
    fn undriven_port_bits_read_the_last_byte_on_the_cpu_s_ports() {
        let mut e = PcEngine::from_card(tiny_card(), Console::PcEngine).unwrap();
        let mut bus = SysBus {
            card: &mut e.card,
            hw: &mut e.hw,
        };
        let page = 0xFF << 13;
        bus.write(page | 0x1403, 0x20);
        assert_eq!(bus.read(page | 0x1403) & 0xF8, 0x20);
        bus.write(page | 0x1402, 0xF8 | 0x02);
        assert_eq!(bus.read(page | 0x1402), 0xF8 | 0x02);
        bus.write(page | 0x0800, 0x5A);
        assert_eq!(bus.read(page | 0x0800), 0x5A);
    }

    #[test]
    fn save_state_round_trips() {
        let mut e = PcEngine::from_card(tiny_card(), Console::PcEngine).unwrap();
        for _ in 0..100 {
            e.step();
        }
        let s = e.save_state();
        let pc = e.cpu.pc;
        for _ in 0..100 {
            e.step();
        }
        e.load_state(&s).unwrap();
        assert_eq!(e.cpu.pc, pc);
    }
}
