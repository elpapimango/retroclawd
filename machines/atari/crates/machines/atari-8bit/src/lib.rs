//! Atari 800: NMOS 6502, ANTIC, GTIA, POKEY and MOS 6520 PIA.
//! Hardware sources and explicit limits: `atari-800.intake.toml`.
//! Accurate mode clocks Chips on each bus access; fast mode batches accesses
//! before clocking the instruction. IRQ/NMI delivery remains instruction-level.
#![forbid(unsafe_code)]

use antic::{Antic, HEIGHT, WIDTH};
use gtia::Gtia;
use machine::{AUDIO_SAMPLE_RATE, Addr, Bus, Clock, FrameRate, Machine, RunStats};
use pia_6520::{Pia, Side};
use pokey::Pokey;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Timing {
    Fast,
    Accurate,
}

/// Altirra Hardware Reference, section 5.8, tables 11 and 12.
pub const KEYS: &[(&str, u8)] = &[
    ("KeyL", 0x00),
    ("KeyJ", 0x01),
    ("Semicolon", 0x02),
    ("KeyK", 0x05),
    ("Plus", 0x06),
    ("Asterisk", 0x07),
    ("KeyO", 0x08),
    ("KeyP", 0x0a),
    ("KeyU", 0x0b),
    ("Return", 0x0c),
    ("KeyI", 0x0d),
    ("Minus", 0x0e),
    ("Equals", 0x0f),
    ("KeyV", 0x10),
    ("KeyC", 0x12),
    ("KeyB", 0x15),
    ("KeyX", 0x16),
    ("KeyZ", 0x17),
    ("Digit4", 0x18),
    ("Digit3", 0x1a),
    ("Digit6", 0x1b),
    ("Escape", 0x1c),
    ("Digit5", 0x1d),
    ("Digit2", 0x1e),
    ("Digit1", 0x1f),
    ("Comma", 0x20),
    ("Space", 0x21),
    ("Period", 0x22),
    ("KeyN", 0x23),
    ("KeyM", 0x25),
    ("Slash", 0x26),
    ("Inverse", 0x27),
    ("KeyR", 0x28),
    ("KeyE", 0x2a),
    ("KeyY", 0x2b),
    ("Tab", 0x2c),
    ("KeyT", 0x2d),
    ("KeyW", 0x2e),
    ("KeyQ", 0x2f),
    ("Digit9", 0x30),
    ("Digit0", 0x32),
    ("Digit7", 0x33),
    ("Backspace", 0x34),
    ("Digit8", 0x35),
    ("LessThan", 0x36),
    ("GreaterThan", 0x37),
    ("KeyF", 0x38),
    ("KeyH", 0x39),
    ("KeyD", 0x3a),
    ("CapsLock", 0x3c),
    ("KeyG", 0x3d),
    ("KeyS", 0x3e),
    ("KeyA", 0x3f),
];

pub struct Roms {
    pub os: Vec<u8>,
    pub basic: Option<Vec<u8>>,
}

impl Roms {
    fn validate(&self) -> Result<(), String> {
        if self.os.len() != 10240 || self.basic.as_ref().is_some_and(|b| b.len() != 8192) {
            return Err(
                "Atari 800 requires a 10240-byte OS ROM and an optional 8192-byte BASIC ROM"
                    .to_string(),
            );
        }
        Ok(())
    }
    fn fingerprint(&self) -> [u8; 20] {
        let mut hash = Sha1::new();
        hash.update(&self.os);
        if let Some(basic) = &self.basic {
            hash.update(basic);
        }
        hash.finalize().into()
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Hardware {
    ram: Vec<u8>,
    antic: Antic,
    gtia: Gtia,
    pokey: Pokey,
    pia: Pia,
    joystick_inputs: [u8; 2],
    screen: Vec<u8>,
    sample_phase: u64,
    nmi: bool,
}

struct AtariBus {
    hardware: Hardware,
    roms: Roms,
    timing: Timing,
    in_cpu: bool,
    palette: [[u8; 3]; 256],
    audio: Vec<f32>,
}

fn memory_read(ram: &[u8], roms: &Roms, address: u16) -> u8 {
    match address {
        0xa000..=0xbfff if roms.basic.is_some() => {
            roms.basic.as_ref().expect("BASIC mapped")[usize::from(address - 0xa000)]
        }
        0..=0xbfff => ram[usize::from(address)],
        0xd800..=0xffff => roms.os[usize::from(address - 0xd800)],
        _ => 0xff,
    }
}

impl AtariBus {
    fn clock_hz(&self) -> u64 {
        // Integer Machine interface: floor the crystal-derived half-Hz rates.
        // Keep the frame ratio consistent, preserving 114 clocks per scan line.
        if self.hardware.antic.pal() {
            1_773_447
        } else {
            1_789_772
        }
    }
    fn peek(&self, address: u16) -> u8 {
        match address & 0xff00 {
            0xd000 => self.hardware.gtia.read(address as u8),
            0xd200 => self.hardware.pokey.read(address as u8),
            0xd300 => self.hardware.pia.peek_alt(address as u8),
            0xd400 => self.hardware.antic.read(address as u8),
            _ => memory_read(&self.hardware.ram, &self.roms, address),
        }
    }
    fn writable(&self, address: u16) -> bool {
        address < 0xc000 && !(address >= 0xa000 && self.roms.basic.is_some())
    }
    fn tick(&mut self) {
        let h = &mut self.hardware;
        let y = h.antic.y;
        let line = h
            .antic
            .tick(|address| memory_read(&h.ram, &self.roms, address));
        h.nmi |= h.antic.take_nmi();
        h.pokey.tick();
        h.pia.tick(1);
        if line && (8..248).contains(&y) {
            let start = usize::from(y - 8) * WIDTH;
            for (pixel, &code) in h.screen[start..start + WIDTH]
                .iter_mut()
                .zip(h.antic.line())
            {
                *pixel = h.gtia.color(code);
            }
        }
        self.hardware.sample_phase += u64::from(AUDIO_SAMPLE_RATE);
        if self.hardware.sample_phase >= self.clock_hz() {
            self.hardware.sample_phase -= self.clock_hz();
            let sample = self.hardware.pokey.output()
                + if self.hardware.gtia.speaker() {
                    0.1
                } else {
                    0.0
                };
            if self.audio.len() >= AUDIO_SAMPLE_RATE as usize * 2 {
                self.audio.clear();
            }
            self.audio.push(sample);
        }
    }
    fn access(&mut self, write: bool) {
        if self.in_cpu && self.timing == Timing::Accurate {
            // RDY holds reads, not writes, including the second RMW write.
            if !write {
                while self.hardware.antic.cpu_read_blocked() {
                    self.tick();
                }
            }
        }
    }
    fn finish(&mut self) {
        if self.in_cpu && self.timing == Timing::Accurate {
            self.tick();
        }
    }
}

impl Bus for AtariBus {
    fn read(&mut self, address: u16) -> u8 {
        self.access(false);
        let value = if address & 0xff00 == 0xd300 {
            self.hardware.pia.read_alt(address as u8)
        } else {
            self.peek(address)
        };
        self.finish();
        value
    }
    fn write(&mut self, address: u16, value: u8) {
        self.access(true);
        match address & 0xff00 {
            0xd000 => self.hardware.gtia.write(address as u8, value),
            0xd200 => self.hardware.pokey.write(address as u8, value),
            0xd300 => self.hardware.pia.write_alt(address as u8, value),
            0xd400 => self.hardware.antic.write(address as u8, value),
            _ if self.writable(address) => self.hardware.ram[usize::from(address)] = value,
            _ => {}
        }
        self.finish();
    }
    fn dummy_cycle(&mut self, address: u16, write: bool, value: u8) {
        if write {
            self.write(address, value);
        } else {
            self.read(address);
        }
    }
}

pub struct Atari800 {
    pub cpu: mos6502::Cpu,
    bus: AtariBus,
}

#[derive(Serialize, Deserialize)]
struct State {
    magic: [u8; 4],
    version: u8,
    rom: [u8; 20],
    timing: Timing,
    cpu: mos6502::Cpu,
    hardware: Hardware,
}

impl Atari800 {
    pub fn new(roms: Roms, pal: bool, timing: Timing) -> Result<Self, String> {
        roms.validate()?;
        let mut machine = Self {
            cpu: mos6502::Cpu::new(),
            bus: AtariBus {
                roms,
                timing,
                in_cpu: false,
                palette: Gtia::palette(),
                audio: Vec::new(),
                hardware: Hardware {
                    ram: vec![0; 49152],
                    antic: Antic::new(pal),
                    gtia: Gtia::new(pal),
                    pokey: Pokey::new(),
                    pia: Pia::new(),
                    joystick_inputs: [0xff; 2],
                    screen: vec![0; WIDTH * HEIGHT],
                    sample_phase: 0,
                    nmi: false,
                },
            },
        };
        machine.cpu.reset(&mut machine.bus);
        Ok(machine)
    }
    pub fn pal(&self) -> bool {
        self.bus.hardware.antic.pal()
    }
    pub fn timing(&self) -> Timing {
        self.bus.timing
    }
    pub fn cycles(&self) -> u64 {
        self.bus.hardware.antic.cycles
    }
    pub fn position(&self) -> (u8, u16) {
        (self.bus.hardware.antic.x, self.bus.hardware.antic.y)
    }
    pub fn can_write(&self, address: u16) -> bool {
        self.bus.writable(address)
    }
    pub fn write_bus(&mut self, address: u16, value: u8) {
        self.bus.write(address, value);
    }
    pub fn chip_registers(&self, name: &str) -> Vec<u8> {
        match name {
            "antic" => (0..16)
                .map(|i| self.bus.hardware.antic.register(i))
                .collect(),
            "gtia" => (0..32)
                .map(|i| self.bus.hardware.gtia.register(i))
                .collect(),
            "pokey" => (0..16)
                .map(|i| self.bus.hardware.pokey.register(i))
                .collect(),
            "pia" => (0..4).map(|i| self.bus.hardware.pia.peek_alt(i)).collect(),
            _ => Vec::new(),
        }
    }
    pub fn set_keyboard(&mut self, keys: u64, shift: bool, control: bool, break_down: bool) {
        self.bus
            .hardware
            .pokey
            .set_keys(keys, shift, control, break_down);
    }
    pub fn set_console(&mut self, bit: u8, pressed: bool) -> Result<(), String> {
        self.bus.hardware.gtia.set_console(bit, pressed)
    }
    pub fn set_joystick(&mut self, port: usize, directions: u8, fire: bool) -> Result<(), String> {
        if port > 3 || directions > 15 {
            return Err("invalid Atari 800 joystick port or directions".to_string());
        }
        let side = if port < 2 { Side::A } else { Side::B };
        let shift = (port & 1) * 4;
        let input = &mut self.bus.hardware.joystick_inputs[port / 2];
        *input = (*input & !(15 << shift)) | ((!directions & 15) << shift);
        self.bus.hardware.pia.set_input(side, *input);
        self.bus.hardware.gtia.set_trigger(port, fire)
    }
    pub fn load_program(&mut self, address: u16, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > 65536
            || bytes
                .iter()
                .enumerate()
                .any(|(i, _)| !self.can_write(address.wrapping_add(i as u16)))
        {
            return Err("Atari 800 Program overlaps ROM, I/O or unmapped memory".to_string());
        }
        self.load(u64::from(address), bytes);
        Ok(())
    }
}

impl Machine for Atari800 {
    fn clock_hz(&self) -> u64 {
        self.bus.clock_hz()
    }
    fn frame_hz(&self) -> u64 {
        if self.pal() { 50 } else { 60 }
    }
    fn frame_rate(&self) -> FrameRate {
        FrameRate {
            num: self.clock_hz() as u32,
            den: 114 * if self.pal() { 312 } else { 262 },
        }
    }
    fn run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> RunStats {
        Clock::new(self.frame_hz()).run(self, running)
    }
    fn step(&mut self) -> u32 {
        if std::mem::take(&mut self.bus.hardware.nmi) {
            self.cpu.request_nmi();
        }
        if self.bus.hardware.pokey.irq()
            || self.bus.hardware.pia.irq(Side::A)
            || self.bus.hardware.pia.irq(Side::B)
        {
            self.cpu.request_irq();
        }
        let start = self.cycles();
        self.bus.in_cpu = true;
        let cycles = self.cpu.step(&mut self.bus);
        self.bus.in_cpu = false;
        if self.bus.timing == Timing::Fast {
            for _ in 0..cycles {
                while self.bus.hardware.antic.cpu_read_blocked() {
                    self.bus.tick();
                }
                self.bus.tick();
            }
        }
        (self.cycles() - start) as u32
    }
    fn halted(&self) -> bool {
        self.cpu.halted
    }
    fn pc(&self) -> Addr {
        u64::from(self.cpu.pc)
    }
    fn registers(&self) -> Vec<(&'static str, u64)> {
        vec![
            ("A", u64::from(self.cpu.a)),
            ("X", u64::from(self.cpu.x)),
            ("Y", u64::from(self.cpu.y)),
            ("SP", u64::from(self.cpu.sp)),
            ("PC", u64::from(self.cpu.pc)),
            ("P", u64::from(self.cpu.p)),
        ]
    }
    fn render_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
        Some((
            WIDTH as u32,
            HEIGHT as u32,
            self.bus
                .hardware
                .screen
                .iter()
                .flat_map(|&color| self.bus.palette[usize::from(color)])
                .collect(),
        ))
    }
    fn read_memory(&self, address: Addr) -> u8 {
        self.bus.peek(address as u16)
    }
    fn load(&mut self, address: Addr, bytes: &[u8]) -> usize {
        let mut count = 0;
        for (i, &value) in bytes.iter().enumerate() {
            let address = (address as u16).wrapping_add(i as u16);
            if self.can_write(address) {
                self.bus.hardware.ram[usize::from(address)] = value;
                count += 1;
            }
        }
        count
    }
    fn set_pc(&mut self, address: Addr) {
        self.cpu.pc = address as u16;
        self.cpu.halted = false;
    }
    fn drain_audio(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.bus.audio)
    }
    fn reset(&mut self) {
        self.bus.hardware.antic.reset_button();
        self.bus.hardware.nmi = true;
        self.cpu.halted = false;
        self.bus.audio.clear();
    }
    fn save_state(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(
            State {
                magic: *b"A800",
                version: 1,
                rom: self.bus.roms.fingerprint(),
                timing: self.timing(),
                cpu: self.cpu,
                hardware: self.bus.hardware.clone(),
            },
            bincode::config::standard(),
        )
        .expect("Atari 800 state encoding")
    }
    fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > 1_048_576 {
            return Err("Atari 800 state exceeds 1 MiB".to_string());
        }
        let (state, used): (State, usize) = bincode::serde::decode_from_slice(
            bytes,
            bincode::config::standard().with_limit::<1_048_576>(),
        )
        .map_err(|e| format!("invalid Atari 800 state: {e}"))?;
        if used != bytes.len() || state.magic != *b"A800" || state.version != 1 {
            return Err("invalid Atari 800 state header or trailing bytes".to_string());
        }
        if state.rom != self.bus.roms.fingerprint()
            || state.timing != self.timing()
            || state.hardware.antic.pal() != self.pal()
        {
            return Err("Atari 800 state ROM, Region or execution mode differs".to_string());
        }
        if state.hardware.ram.len() != 49152
            || state.hardware.screen.len() != WIDTH * HEIGHT
            || state.hardware.sample_phase >= self.clock_hz()
        {
            return Err("invalid Atari 800 memory or audio state".to_string());
        }
        state.hardware.antic.validate()?;
        state.hardware.gtia.validate()?;
        state.hardware.pokey.validate()?;
        state.hardware.pia.validate()?;
        self.cpu = state.cpu;
        self.bus.hardware = state.hardware;
        self.bus.audio.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn machine(timing: Timing) -> Atari800 {
        let mut os = vec![0xea; 10240];
        os[10236..10238].copy_from_slice(&[0, 0xe0]);
        Atari800::new(
            Roms {
                os,
                basic: Some(vec![0x55; 8192]),
            },
            true,
            timing,
        )
        .unwrap()
    }
    #[test]
    fn map_rom_protection_pia_mirrors_and_atomic_program_load() {
        let mut m = machine(Timing::Accurate);
        assert_eq!(m.pc(), 0xe000);
        assert_eq!(m.load(0x9fff, &[1, 2]), 1);
        assert_eq!(m.read_memory(0xa000), 0x55);
        assert_eq!(m.read_memory(0xc000), 0xff);
        m.write_bus(0xd302, 4);
        assert_eq!(m.read_memory(0xd3fe), 4);
        m.set_joystick(0, 1, true).unwrap();
        assert_eq!(m.read_memory(0xd300), 0xfe);
        assert_eq!(m.read_memory(0xd010), 0);
        let before = m.save_state();
        assert!(m.load_program(0x9fff, &[2, 3]).is_err());
        assert_eq!(m.save_state(), before);
    }
    #[test]
    fn deterministic_future_and_failed_restore_does_not_mutate() {
        for timing in [Timing::Fast, Timing::Accurate] {
            let mut m = machine(timing);
            for _ in 0..100 {
                m.step();
            }
            let saved = m.save_state();
            for _ in 0..100 {
                m.step();
            }
            let future = m.save_state();
            m.load_state(&saved).unwrap();
            for _ in 0..100 {
                m.step();
            }
            assert_eq!(m.save_state(), future);
            let mut bad = saved.clone();
            bad.push(0);
            assert!(m.load_state(&bad).is_err());
            assert_eq!(m.save_state(), future);
            m.reset();
            assert_eq!(m.read_memory(0xd40f), 0x3f);
        }
    }
    #[test]
    fn joystick_inputs_survive_pia_output_direction_changes() {
        let mut m = machine(Timing::Accurate);
        m.write_bus(0xd300, 0xff);
        m.write_bus(0xd302, 4);
        m.write_bus(0xd300, 0);
        m.set_joystick(0, 1, false).unwrap();
        m.set_joystick(1, 2, false).unwrap();
        m.write_bus(0xd302, 0);
        m.write_bus(0xd300, 0);
        m.write_bus(0xd302, 4);
        assert_eq!(m.read_memory(0xd300), 0xde);
    }
    #[test]
    fn accurate_bus_stalls_reads_not_rmw_writes_and_returns_elapsed_clocks() {
        let mut m = machine(Timing::Accurate);
        m.load_program(0x2000, &[0xee, 0x0a, 0xd4, 0xea]).unwrap();
        m.set_pc(0x2000);
        assert_eq!(m.step(), 6);
        assert!(m.step() > 90);
        assert_eq!(m.cpu.pc, 0x2004);
        assert!(m.cycles() >= 107);
    }
}
