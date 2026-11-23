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
/// Lines of border shown above the first active line.
const TOP_OVERSCAN: usize = 3;
/// Timer: one tick every 1024 high-speed CPU cycles.
const TIMER_CLOCKS: u32 = 3072;

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
    irq_disable: u8,
    io_sel: bool,
    io_clr: bool,
    io_counter: u8,
    /// A multitap is plugged in: the counter picks the pad. Without one the port reads the first pad always.
    multitap: bool,
    /// Buttons held per pad (bit set = pressed): I, II, Select, Run, Up, Right, Down, Left.
    pads: [u8; 5],
    console: Console,
}

/// What the CPU sees on its bus: the card, plus everything in `Hw`.
struct SysBus<'a> {
    card: &'a mut Card,
    hw: &'a mut Hw,
}

impl Hw {
    fn irq_request(&self) -> u8 {
        let mut r = 0;
        if self.vdc.irq() {
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

    fn hw_read(&mut self, offset: u16) -> u8 {
        match offset >> 10 {
            0 => self.vdc.read((offset & 3) as u8),
            1 => self.vce.read((offset & 7) as u8),
            2 => self.psg.read((offset & 15) as u8),
            3 => {
                if offset & 1 == 0 {
                    self.timer_counter & 0x7F
                } else {
                    u8::from(self.timer_enabled)
                }
            }
            4 => self.io_read(),
            5 => match offset & 3 {
                2 => self.irq_disable,
                3 => self.irq_request(),
                _ => 0xFF,
            },
            _ => 0xFF,
        }
    }

    fn hw_write(&mut self, offset: u16, v: u8) {
        match offset >> 10 {
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
            _ => {}
        }
    }
}

impl Bus for SysBus<'_> {
    fn read(&mut self, addr: u32) -> u8 {
        let bank = (addr >> 13) as u8;
        let off = (addr & 0x1FFF) as u16;
        match bank {
            0x00..=0x7F => self.card.read(bank, off),
            0xF8..=0xFB => self.hw.ram[usize::from(off)],
            0xFF => self.hw.hw_read(off),
            _ => 0xFF,
        }
    }

    fn write(&mut self, addr: u32, v: u8) {
        let bank = (addr >> 13) as u8;
        let off = (addr & 0x1FFF) as u16;
        match bank {
            0x00..=0x7F => self.card.write(bank, off, v),
            0xF8..=0xFB => self.hw.ram[usize::from(off)] = v,
            0xFF => self.hw.hw_write(off, v),
            _ => {}
        }
    }

    fn irq_lines(&self) -> u8 {
        self.hw.irq_request() & !self.hw.irq_disable & (IRQ1 | TIMER | IRQ2)
    }

    fn st(&mut self, port: u8, v: u8) {
        self.hw.vdc.write(port, v);
    }
}

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
                timer_counter: 0,
                timer_enabled: false,
                timer_acc: 0,
                timer_irq: false,
                irq_disable: 0,
                io_sel: false,
                io_clr: false,
                io_counter: 0,
                multitap: false,
                pads: [0; 5],
                console,
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
        // the picture window starts a few lines above the first active line, like the television's overscan; the lines
        // outside the picture show the border colour
        let out_row = line as i32 - i32::from(self.hw.vdc.frame_start()) + TOP_OVERSCAN as i32;
        if (0..CANVAS_H as i32).contains(&out_row) {
            let base = out_row as usize * CANVAS_W * 3;
            if drew {
                // the picture's nominal width for the dot clock; a wider register setting is cropped on the right
                let nominal = match self.hw.vce.dot_clock() {
                    0 => 256,
                    1 => 352,
                    _ => 512,
                };
                let width = buf.len().clamp(1, nominal);
                for x in 0..CANVAS_W {
                    let src = buf[x * width / CANVAS_W];
                    let rgb = self.hw.vce.rgb(src);
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
            0xF8..=0xFB => self.hw.ram[usize::from(off)],
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
                irq_disable: self.hw.irq_disable,
                io_sel: self.hw.io_sel,
                io_clr: self.hw.io_clr,
                io_counter: self.hw.io_counter,
                multitap: self.hw.multitap,
                pads: self.hw.pads,
                console: self.hw.console,
            },
            line: self.line,
            line_clock: self.line_clock,
            block: self.card.block(),
            card_len: self.card.len(),
        };
        let mut out = b"RCPC\x01".to_vec();
        out.extend(
            bincode::serde::encode_to_vec(&state, bincode::config::standard())
                .expect("state encodes"),
        );
        out
    }

    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let body = bytes
            .strip_prefix(b"RCPC\x01")
            .ok_or_else(|| "not a PC Engine save state".to_string())?;
        let (state, _): (SaveState, usize) =
            bincode::serde::decode_from_slice(body, bincode::config::standard())
                .map_err(|e| format!("corrupt save state: {e}"))?;
        if state.card_len != self.card.len() {
            return Err("the save state is for a different card".to_string());
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
