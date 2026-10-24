use machine::{Addr, AudioFormat, FrameRate, Machine, RunStats};
use mc68000::{Cpu, RunState};

use crate::bus::Bus;
use crate::config::{Config, M68K_DIVIDER, Region, frame_clocks};
use crate::sound::SAMPLE_MASTER_CLOCKS;

/// Width and height of the frame until the VDP draws one.
const BLANK_WIDTH: u32 = 320;
const BLANK_HEIGHT: u32 = 224;

pub struct Genesis {
    cpu: Cpu,
    bus: Bus,
    region: Region,
    stop_reported: bool,
}

impl Genesis {
    pub fn new(config: Config) -> Result<Self, String> {
        if config.rom.is_empty() {
            return Err("the cartridge ROM is empty".to_string());
        }
        let mut machine = Self {
            cpu: Cpu::new(),
            bus: Bus::new(&config),
            region: config.region,
            stop_reported: false,
        };
        machine.reset();
        Ok(machine)
    }

    pub fn region(&self) -> Region {
        self.region
    }

    pub fn bus(&self) -> &Bus {
        &self.bus
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    /// Presses or releases a pad button by its name (`Up`, `A`, `Start`, `Mode`, ...) on port 0 or 1.
    pub fn set_pad_button(&mut self, port: usize, name: &str, pressed: bool) -> Result<(), String> {
        let mask = crate::io::BUTTONS
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|&(_, m)| m)
            .ok_or_else(|| format!("unknown pad button \"{name}\""))?;
        if port > 1 {
            return Err(format!("no pad port {port}"));
        }
        self.bus.io.set_button(port, mask, pressed);
        Ok(())
    }

    /// The report of the stop at an address no fact covers, once.
    pub fn take_stop_report(&mut self) -> Option<String> {
        if self.stop_reported {
            return None;
        }
        let report = self.bus.stop()?.report();
        self.stop_reported = true;
        Some(report)
    }
}

impl Machine for Genesis {
    fn clock_hz(&self) -> u64 {
        self.region.master_hz()
    }

    fn frame_hz(&self) -> u64 {
        self.region.master_hz() / frame_clocks(self.region)
    }

    fn frame_rate(&self) -> FrameRate {
        FrameRate {
            num: self.region.master_hz() as u32,
            den: frame_clocks(self.region) as u32,
        }
    }

    fn audio_format(&self) -> AudioFormat {
        // One sample per 24 YM2612 chip cycles (1008 master clocks): 53 267.04 Hz on NTSC, 52 780.5 Hz on PAL.
        AudioFormat {
            channels: 2,
            sample_rate: (self.region.master_hz() / SAMPLE_MASTER_CLOCKS) as u32,
        }
    }

    fn drain_audio(&mut self) -> Vec<f32> {
        self.bus.catch_up();
        self.bus.sound.run_to(self.bus.now);
        self.bus.sound.drain()
    }

    fn run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> RunStats {
        machine::Clock::new(self.frame_hz()).run(self, running)
    }

    fn step(&mut self) -> u32 {
        let start = self.bus.now;
        if self.bus.stop().is_some() {
            self.bus.now += 4 * M68K_DIVIDER;
            return (4 * M68K_DIVIDER) as u32;
        }
        self.bus.pc = self.cpu.pc;
        self.bus.catch_up();
        self.cpu.set_ipl(self.bus.vdp.irq_level());
        self.cpu.step(&mut self.bus);
        self.bus.catch_up();
        (self.bus.now - start) as u32
    }

    fn halted(&self) -> bool {
        self.bus.stop().is_some() || self.cpu.run_state() == RunState::Halted
    }

    fn pc(&self) -> Addr {
        u64::from(self.cpu.pc)
    }

    fn registers(&self) -> Vec<(&'static str, u64)> {
        const D: [&str; 8] = ["D0", "D1", "D2", "D3", "D4", "D5", "D6", "D7"];
        const A: [&str; 7] = ["A0", "A1", "A2", "A3", "A4", "A5", "A6"];
        let c = &self.cpu;
        let mut regs: Vec<(&'static str, u64)> =
            D.iter().zip(c.d).map(|(n, v)| (*n, u64::from(v))).collect();
        regs.extend(A.iter().zip(c.a).map(|(n, v)| (*n, u64::from(v))));
        regs.extend([
            ("A7", u64::from(c.a7())),
            ("USP", u64::from(c.usp)),
            ("SSP", u64::from(c.ssp)),
            ("PC", u64::from(c.pc)),
            ("SR", u64::from(c.sr())),
        ]);
        regs
    }

    fn render_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
        match self.bus.vdp.frame() {
            Some((w, h, rgb)) => Some((w, h, rgb.to_vec())),
            None => Some((
                BLANK_WIDTH,
                BLANK_HEIGHT,
                vec![0; (BLANK_WIDTH * BLANK_HEIGHT * 3) as usize],
            )),
        }
    }

    fn read_memory(&self, addr: Addr) -> u8 {
        self.bus.peek(addr as u32)
    }

    fn load(&mut self, addr: Addr, bytes: &[u8]) -> usize {
        // Only work RAM can be written from outside.
        let mut written = 0;
        for (i, &b) in bytes.iter().enumerate() {
            let a = (addr as u32).wrapping_add(i as u32) & 0x00FF_FFFF;
            if (0xE0_0000..=0xFF_FFFF).contains(&a) {
                self.bus.ram_mut()[(a & 0xFFFF) as usize] = b;
                written += 1;
            }
        }
        written
    }

    fn set_pc(&mut self, addr: Addr) {
        self.cpu.pc = addr as u32;
    }

    fn save_state(&self) -> Vec<u8> {
        Vec::new()
    }

    fn load_state(&mut self, _bytes: &[u8]) -> Result<(), String> {
        Err("the Genesis has no save state yet".to_string())
    }

    fn reset(&mut self) {
        self.stop_reported = false;
        self.bus.take_stop();
        self.cpu.reset(&mut self.bus);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ROM whose reset vectors start the given program at `$200`.
    fn rom(program: &[u16]) -> Vec<u8> {
        let mut rom = vec![0u8; 0x400];
        rom[0..4].copy_from_slice(&0x00FF_FE00u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
        for (i, w) in program.iter().enumerate() {
            rom[0x200 + 2 * i..0x202 + 2 * i].copy_from_slice(&w.to_be_bytes());
        }
        rom
    }

    fn machine(program: &[u16]) -> Genesis {
        Genesis::new(Config::new(rom(program), Region::Usa)).unwrap()
    }

    #[test]
    fn the_frame_rate_is_the_exact_ratio_of_each_region() {
        let g = machine(&[0x60FE]);
        assert_eq!(g.frame_rate().num, 53_693_175);
        assert_eq!(g.frame_rate().den, 896_040);
        let p = Genesis::new(Config::new(rom(&[0x60FE]), Region::Europe)).unwrap();
        assert_eq!(p.frame_rate().num, 53_203_424);
        assert_eq!(p.frame_rate().den, 1_070_460);
        assert_eq!(g.audio_format().channels, 2);
    }

    #[test]
    fn reset_loads_the_vectors_from_the_rom() {
        let g = machine(&[0x60FE]);
        assert_eq!(g.cpu().ssp, 0x00FF_FE00);
    }

    #[test]
    fn a_program_in_the_rom_writes_work_ram() {
        // MOVE.W #$BEEF,$FF0000 ; BRA.S *
        let mut g = machine(&[0x33FC, 0xBEEF, 0x00FF, 0x0000, 0x60FE]);
        for _ in 0..10 {
            g.step();
        }
        assert_eq!(g.read_memory(0xFF_0000), 0xBE);
        assert_eq!(g.read_memory(0xFF_0001), 0xEF);
        assert!(!g.halted());
    }

    #[test]
    fn stepping_returns_master_clocks_that_add_up_to_the_clock() {
        let mut g = machine(&[0x4E71, 0x4E71, 0x60FE]);
        let before = g.bus().now;
        let spent: u32 = (0..3).map(|_| g.step()).sum();
        assert_eq!(u64::from(spent), g.bus().now - before);
        // a NOP is 4 CPU clocks = 28 master clocks
        assert_eq!(machine(&[0x4E71, 0x60FE]).step(), 28);
    }

    #[test]
    fn an_access_nothing_covers_stops_the_machine_once() {
        // MOVE.W $500000,D0
        let mut g = machine(&[0x3039, 0x0050, 0x0000, 0x60FE]);
        g.step();
        assert!(g.halted());
        let report = g.take_stop_report().unwrap();
        assert!(report.contains("$500000"), "{report}");
        assert!(g.take_stop_report().is_none());
    }

    #[test]
    fn an_empty_rom_is_refused() {
        assert!(Genesis::new(Config::new(Vec::new(), Region::Usa)).is_err());
    }
}
