//! Atari 800: NMOS 6502, ANTIC, GTIA, POKEY and MOS 6520 PIA.
//! Hardware sources and explicit limits: `atari-800.intake.toml`.
//! Accurate mode clocks Chips on each bus access; fast mode batches accesses
//! before clocking the instruction. IRQ/NMI delivery remains instruction-level.
#![forbid(unsafe_code)]

use antic::{Antic, HEIGHT, WIDTH};
use atari_media::{Cartridge, Cas, CasPlayer, CasPosition, Mapper};
use gtia::Gtia;
use machine::{AUDIO_SAMPLE_RATE, Addr, Bus, Clock, FrameRate, Machine, RunStats};
use pia_6520::{Pia, Side};
use pokey::Pokey;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::VecDeque;
mod sio;
use sio::{Sio, SioMediaState, SioState};
pub mod mmu;
pub use mmu::Profile;
use mmu::{Access, Mapping, Mmu};

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
    fn validate(&self, profile: Profile) -> Result<(), String> {
        Mmu::new(profile)
            .validate_memory(
                profile.ram_size(),
                self.os.len(),
                self.basic.as_ref().map(Vec::len),
            )
            .map_err(|error| format!("{}: {error}", profile.id()))
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
    #[serde(skip)]
    nmi_ready_at: u64,
    #[serde(skip)]
    bus_value: u8,
    #[serde(skip)]
    irq_sample: bool,
}

impl Hardware {
    fn irq(&self) -> bool {
        self.pokey.irq() || self.pia.irq(Side::A) || self.pia.irq(Side::B)
    }
}

struct AtariBus {
    hardware: Hardware,
    roms: Roms,
    timing: Timing,
    in_cpu: bool,
    palette: [[u8; 3]; 256],
    audio: Vec<f32>,
    sio: Sio,
    mmu: Mmu,
    cartridge: Option<Cartridge>,
    tape: Option<CasPlayer>,
}

fn memory_read(ram: &[u8], roms: &Roms, mapping: Mapping) -> u8 {
    match mapping {
        Mapping::Ram(index) => ram[index],
        Mapping::Os(index) => roms.os[index],
        Mapping::Basic(index) => roms.basic.as_ref().map_or(0xff, |basic| basic[index]),
        Mapping::Unmapped => 0xff,
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
            _ => self
                .cartridge
                .as_ref()
                .and_then(|cart| cart.read(address))
                .unwrap_or_else(|| {
                    memory_read(
                        &self.hardware.ram,
                        &self.roms,
                        self.mmu.map_read(
                            address,
                            Access::Cpu,
                            self.hardware.pia.drive(Side::B),
                            self.roms.basic.is_some(),
                        ),
                    )
                }),
        }
    }
    fn writable(&self, address: u16) -> bool {
        self.write_index(address).is_some()
    }
    fn write_index(&self, address: u16) -> Option<usize> {
        if self
            .cartridge
            .as_ref()
            .and_then(|cart| cart.read(address))
            .is_some()
        {
            return None;
        }
        self.mmu.map_write(
            address,
            Access::Cpu,
            self.hardware.pia.drive(Side::B),
            self.roms.basic.is_some(),
        )
    }
    fn tick(&mut self) {
        let h = &mut self.hardware;
        let x = h.antic.x;
        let y = h.antic.y;
        if x == 0 {
            h.gtia.begin_scanline();
        }
        let mut data_bus = h.bus_value;
        h.antic.tick_with_bus(
            |address| {
                let value = self
                    .cartridge
                    .as_ref()
                    .and_then(|cart| cart.read(address))
                    .unwrap_or_else(|| {
                        memory_read(
                            &h.ram,
                            &self.roms,
                            self.mmu.map_read(
                                address,
                                Access::Antic,
                                h.pia.drive(Side::B),
                                self.roms.basic.is_some(),
                            ),
                        )
                    });
                data_bus = value;
                value
            },
            h.bus_value,
        );
        h.bus_value = data_bus;
        if let Some((player, byte)) = h.antic.take_pm_transfer() {
            if player == 4 {
                h.gtia.dma_missiles(byte, y & 1 != 0);
            } else {
                h.gtia
                    .dma_player(usize::from(player), byte, y & 1 != 0)
                    .expect("ANTIC only transfers players 0-3");
            }
        }
        let visible = (8..248).contains(&y);
        for half in 0..2 {
            // GTIA's visible interval begins at HPOS $22 (CPU clock 18).
            if !(18..112).contains(&x) {
                h.gtia.set_hires(h.antic.hires());
            }
            let pixel = (17..113)
                .contains(&x)
                .then(|| usize::from(x - 17) * 4 + half * 2);
            let playfield = pixel.map_or([0; 2], |pixel| {
                [h.antic.pixel_at(pixel), h.antic.pixel_at(pixel + 1)]
            });
            let next = pixel
                .filter(|&pixel| pixel + 3 < WIDTH)
                .map_or([0; 2], |pixel| {
                    [h.antic.pixel_at(pixel + 2), h.antic.pixel_at(pixel + 3)]
                });
            let colors = h
                .gtia
                .tick_color_clock_with_next(playfield, next, h.antic.video_active());
            if let Some(pixel) = pixel.filter(|_| visible) {
                let start = usize::from(y - 8) * WIDTH + pixel;
                h.screen[start..start + 2].copy_from_slice(&colors);
            }
        }
        if h.antic.take_nmi() {
            h.nmi = true;
            // AHR 4.8: NMI at cycle 8, ordinary acknowledgment from cycle 10.
            h.nmi_ready_at = h.antic.cycles + 1;
        }
        // Poll before the final clock's pin update, not at the following
        // instruction boundary (AHR 3, IRQ timing; Acid800 POKEY IRQ sled).
        h.irq_sample = h.irq();
        h.pokey.tick();
        h.pia.tick(1);
        let input = self.sio.tick(
            h.pia.c2_output(Side::B).unwrap_or(true),
            h.pokey.serial_output(),
        );
        let tape_input = self
            .tape
            .as_mut()
            .is_none_or(|tape| tape.tick(1, h.pia.c2_output(Side::A) == Some(false)));
        h.pokey.set_serial_input(input && tape_input);
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
            // DMA HALT stops every access; WSYNC RDY only holds reads.
            while if write {
                self.hardware.antic.cpu_dma_blocked()
            } else {
                self.hardware.antic.cpu_read_blocked()
            } {
                self.tick();
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
        if address & 0xff00 == 0xd000 {
            // GTIA collision latches include this clock's video before the
            // CPU samples them, including the final visible clock at $DD.
            self.finish();
            let value = self.peek(address);
            self.hardware.bus_value = value;
            return value;
        }
        let value = if address & 0xff00 == 0xd300 {
            self.hardware.pia.read_alt(address as u8)
        } else {
            self.peek(address)
        };
        self.hardware.bus_value = value;
        self.finish();
        value
    }
    fn write(&mut self, address: u16, value: u8) {
        self.access(true);
        self.hardware.bus_value = value;
        if address & 0xff1f == 0xd01e {
            // HITCLR's CPU write strobe wins over collisions in this bus
            // clock (AHR 6.6; Acid800 antic_virtdma.s at raster clock 111).
            self.finish();
            self.hardware.gtia.write(address as u8, value);
            return;
        }
        match address & 0xff00 {
            0xd000 => self.hardware.gtia.write(address as u8, value),
            0xd200 => self.hardware.pokey.write(address as u8, value),
            0xd300 => self.hardware.pia.write_alt(address as u8, value),
            0xd400 => self.hardware.antic.write(address as u8, value),
            0xd500 => {
                if let Some(cart) = &mut self.cartridge {
                    cart.write_control(address, value);
                }
            }
            _ => {
                if let Some(index) = self.write_index(address) {
                    self.hardware.ram[index] = value;
                }
            }
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
    fn take_nmi_edge(&mut self) -> bool {
        std::mem::take(&mut self.hardware.nmi)
    }
    fn interrupt_status_pushed(&mut self) {
        // AHR 4.8 "Missed NMIs": a short ANTIC pulse coincident with
        // interrupt acknowledgment is lost, unlike an earlier hijacking edge.
        if self.hardware.nmi_ready_at == self.hardware.antic.cycles + 1 {
            self.hardware.nmi = false;
        }
    }
}

pub struct Atari800 {
    pub cpu: mos6502::Cpu,
    bus: AtariBus,
    program: Option<XexLoader>,
}

#[derive(Serialize, Deserialize)]
struct XexLoader {
    segments: VecDeque<(u16, Vec<u8>)>,
    return_pc: u16,
    return_sp: u8,
    armed: bool,
}

#[derive(Serialize, Deserialize)]
struct StateV1 {
    magic: [u8; 4],
    version: u8,
    rom: [u8; 20],
    timing: Timing,
    cpu: mos6502::Cpu,
    hardware: Hardware,
}

#[derive(Serialize, Deserialize)]
struct State {
    base: StateV1,
    serial: pokey::SerialState,
    sio: SioState,
    program: Option<XexLoader>,
}

#[derive(Serialize, Deserialize)]
struct TimingState {
    antic: antic::AnticTimingState,
    gtia: gtia::GtiaGraphicsState,
    pokey: pokey::PokeyTimingState,
    nmi_ready_at: u64,
    bus_value: u8,
    irq_sample: bool,
}

#[derive(Serialize, Deserialize)]
struct VideoControlState {
    antic: antic::AnticDmaState,
    gtia: gtia::GtiaVideoState,
    pia: pia_6520::PiaControlState,
    mmu: mmu::MmuState,
    sio: SioMediaState,
    cartridge: Option<CartridgeState>,
    tape: Option<TapeState>,
}

#[derive(Serialize, Deserialize)]
struct CartridgeState {
    rom: Vec<u8>,
    mapper: u32,
    bank: u8,
}

#[derive(Serialize, Deserialize)]
struct TapeState {
    bytes: Vec<u8>,
    record: u32,
    cycle: u64,
}

impl Atari800 {
    pub fn new(roms: Roms, pal: bool, timing: Timing) -> Result<Self, String> {
        Self::with_profile(Profile::Atari800, roms, pal, timing)
    }
    pub fn with_profile(
        profile: Profile,
        roms: Roms,
        pal: bool,
        timing: Timing,
    ) -> Result<Self, String> {
        roms.validate(profile)?;
        let mut machine = Self {
            cpu: mos6502::Cpu::new(),
            program: None,
            bus: AtariBus {
                roms,
                timing,
                in_cpu: false,
                palette: Gtia::palette(),
                audio: Vec::new(),
                sio: Sio::new(if pal { 1_773_447 } else { 1_789_772 }),
                mmu: Mmu::new(profile),
                cartridge: None,
                tape: None,
                hardware: Hardware {
                    ram: vec![0; profile.ram_size()],
                    antic: Antic::new(pal),
                    gtia: Gtia::new(pal),
                    pokey: Pokey::new(),
                    pia: Pia::new(),
                    joystick_inputs: [0xff; 2],
                    screen: vec![0; WIDTH * HEIGHT],
                    sample_phase: 0,
                    nmi: false,
                    nmi_ready_at: 0,
                    bus_value: 0xff,
                    irq_sample: false,
                },
            },
        };
        if profile.is_xl_xe() {
            machine.bus.hardware.gtia.set_trigger(3, true)?;
        }
        machine.cpu.reset(&mut machine.bus);
        Ok(machine)
    }
    pub fn profile(&self) -> Profile {
        self.bus.mmu.profile()
    }
    pub fn joystick_count(&self) -> usize {
        if self.profile().is_xl_xe() { 2 } else { 4 }
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
        if port >= self.joystick_count() || directions > 15 {
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
    pub fn attach_disk(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.bus.sio.attach(bytes)
    }
    pub fn eject_disk(&mut self) {
        self.bus.sio.detach();
    }
    pub fn disk_attached(&self) -> bool {
        self.bus.sio.attached()
    }
    pub fn export_disk(&self) -> Option<Vec<u8>> {
        self.bus.sio.export()
    }
    pub fn disk_reads(&self) -> u64 {
        self.bus.sio.reads()
    }
    pub fn disk_sector(&self) -> u16 {
        self.bus.sio.sector()
    }
    pub fn disk_busy(&self) -> bool {
        self.bus.sio.busy()
    }
    pub fn attach_disk_in(
        &mut self,
        drive: u8,
        extension: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        self.bus.sio.attach_format(drive, extension, bytes)
    }
    pub fn eject_disk_in(&mut self, drive: u8) -> Result<(), String> {
        self.bus.sio.detach_in(drive)
    }
    pub fn disk_attached_in(&self, drive: u8) -> bool {
        self.bus.sio.attached_in(drive)
    }
    pub fn disk_dirty_in(&self, drive: u8) -> bool {
        self.bus.sio.dirty_in(drive)
    }
    pub fn export_disk_in(&self, drive: u8) -> Option<Vec<u8>> {
        self.bus.sio.export_in(drive)
    }
    pub fn disk_extension_in(&self, drive: u8) -> Option<&'static str> {
        self.bus.sio.extension_in(drive)
    }
    pub fn mark_disk_saved_in(&mut self, drive: u8) -> Result<(), String> {
        self.bus.sio.mark_saved_in(drive)
    }
    pub fn attach_cartridge(&mut self, extension: &str, bytes: &[u8]) -> Result<(), String> {
        let cartridge = if extension.eq_ignore_ascii_case("car") {
            Cartridge::parse_car(bytes)?
        } else if extension.eq_ignore_ascii_case("rom") || extension.eq_ignore_ascii_case("bin") {
            Cartridge::parse_raw(bytes, None)?
        } else {
            return Err(format!(
                "unsupported Atari cartridge format {extension}; expected car, rom or bin"
            ));
        };
        self.bus.cartridge = Some(cartridge);
        self.update_cartridge_sense();
        Ok(())
    }
    fn update_cartridge_sense(&mut self) {
        if self.profile().is_xl_xe() {
            self.bus
                .hardware
                .gtia
                .set_trigger(3, self.bus.cartridge.is_none())
                .expect("GTIA trigger 3");
        }
    }
    pub fn eject_cartridge(&mut self) {
        self.bus.cartridge = None;
        self.update_cartridge_sense();
    }
    pub fn cartridge(&self) -> Option<&Cartridge> {
        self.bus.cartridge.as_ref()
    }
    pub fn attach_tape(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.bus.tape = Some(CasPlayer::new(Cas::parse(bytes)?, self.clock_hz())?);
        Ok(())
    }
    pub fn eject_tape(&mut self) {
        self.bus.tape = None;
    }
    pub fn tape(&self) -> Option<&CasPlayer> {
        self.bus.tape.as_ref()
    }
    pub fn tape_motor(&self) -> bool {
        self.bus.hardware.pia.c2_output(Side::A) == Some(false)
    }
    pub fn load_xex(&mut self, bytes: &[u8]) -> Result<(), String> {
        let xex = atari_media::Xex::parse(bytes)?;
        for segment in &xex.segments {
            if segment
                .bytes
                .iter()
                .enumerate()
                .any(|(i, _)| !self.can_write(segment.start.wrapping_add(i as u16)))
            {
                return Err("Atari XEX segment overlaps ROM, I/O or unmapped memory".to_string());
            }
        }
        let entry = xex
            .segments
            .first()
            .ok_or("Atari XEX has no segments")?
            .start;
        self.load(0x2e0, &entry.to_le_bytes());
        self.load(0x2e2, &[0, 0]);
        self.program = Some(XexLoader {
            segments: xex
                .segments
                .into_iter()
                .map(|segment| (segment.start, segment.bytes))
                .collect(),
            return_pc: self.cpu.pc,
            return_sp: self.cpu.sp,
            armed: false,
        });
        self.next_xex_segment();
        Ok(())
    }
    fn next_xex_segment(&mut self) {
        let mut loader = self.program.take().expect("active XEX loader");
        while let Some((start, bytes)) = loader.segments.pop_front() {
            self.load(u64::from(start), &bytes);
            let init = u16::from_le_bytes([self.read_memory(0x2e2), self.read_memory(0x2e3)]);
            self.load(0x2e2, &[0, 0]);
            if init != 0 {
                // Execute INITAD on the real Core; resume loading only when its
                // RTS returns to the caller's PC and stack depth. No ROM patch.
                let ret = loader.return_pc.wrapping_sub(1);
                self.load(0x100 | u64::from(self.cpu.sp), &[(ret >> 8) as u8]);
                self.cpu.sp = self.cpu.sp.wrapping_sub(1);
                self.load(0x100 | u64::from(self.cpu.sp), &[ret as u8]);
                self.cpu.sp = self.cpu.sp.wrapping_sub(1);
                self.cpu.pc = init;
                self.cpu.halted = false;
                loader.armed = false;
                self.program = Some(loader);
                return;
            }
        }
        self.cpu.pc = u16::from_le_bytes([self.read_memory(0x2e0), self.read_memory(0x2e1)]);
        self.cpu.halted = false;
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
        if self.program.as_ref().is_some_and(|loader| {
            loader.armed && self.cpu.pc == loader.return_pc && self.cpu.sp == loader.return_sp
        }) {
            self.next_xex_segment();
        }
        if let Some(loader) = &mut self.program {
            loader.armed = true;
        }
        if self.bus.hardware.antic.cycles >= self.bus.hardware.nmi_ready_at
            && std::mem::take(&mut self.bus.hardware.nmi)
        {
            self.cpu.request_nmi();
        }
        if self.bus.hardware.irq_sample {
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
            if let Some(index) = self.bus.write_index(address) {
                self.bus.hardware.ram[index] = value;
                count += 1;
            }
        }
        count
    }
    fn set_pc(&mut self, address: Addr) {
        self.program = None;
        self.cpu.pc = address as u16;
        self.cpu.halted = false;
    }
    fn drain_audio(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.bus.audio)
    }
    fn reset(&mut self) {
        self.program = None;
        if self.profile().is_xl_xe() {
            self.bus.hardware.antic.reset();
            self.bus.hardware.pia = Pia::new();
            self.bus
                .hardware
                .pia
                .set_input(Side::A, self.bus.hardware.joystick_inputs[0]);
            self.bus.hardware.nmi = false;
            self.bus.hardware.irq_sample = false;
            self.cpu.reset(&mut self.bus);
        } else {
            self.bus.hardware.antic.reset_button();
            self.bus.hardware.nmi = true;
        }
        self.bus.hardware.nmi_ready_at = 0;
        self.cpu.halted = false;
        self.bus.audio.clear();
    }
    fn save_state(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(
            (
                State {
                    base: StateV1 {
                        magic: *b"A800",
                        version: 5,
                        rom: self.bus.roms.fingerprint(),
                        timing: self.timing(),
                        cpu: self.cpu,
                        hardware: self.bus.hardware.clone(),
                    },
                    serial: self.bus.hardware.pokey.serial_state(),
                    sio: self.bus.sio.save(),
                    program: self.program.as_ref().map(|loader| XexLoader {
                        segments: loader.segments.clone(),
                        return_pc: loader.return_pc,
                        return_sp: loader.return_sp,
                        armed: loader.armed,
                    }),
                },
                self.cpu.irq_poll_mask(),
                TimingState {
                    antic: self.bus.hardware.antic.save_timing(),
                    gtia: self.bus.hardware.gtia.save_graphics(),
                    pokey: self.bus.hardware.pokey.save_timing(),
                    nmi_ready_at: self.bus.hardware.nmi_ready_at,
                    bus_value: self.bus.hardware.bus_value,
                    irq_sample: self.bus.hardware.irq_sample,
                },
                VideoControlState {
                    antic: self.bus.hardware.antic.save_dma(),
                    gtia: self.bus.hardware.gtia.save_video(),
                    pia: self.bus.hardware.pia.save_control(),
                    mmu: self.bus.mmu.state(),
                    sio: self.bus.sio.save_media(),
                    cartridge: self.bus.cartridge.as_ref().map(|cart| CartridgeState {
                        rom: cart.rom().to_vec(),
                        mapper: cart.mapper().car_type(),
                        bank: cart.bank(),
                    }),
                    tape: self.bus.tape.as_ref().map(|tape| TapeState {
                        bytes: tape.to_bytes(),
                        record: tape.position().record,
                        cycle: tape.position().cycle,
                    }),
                },
            ),
            bincode::config::standard(),
        )
        .expect("Atari 800 state encoding")
    }
    fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > 67_108_864 {
            return Err("Atari 800 state exceeds 64 MiB".to_string());
        }
        if bytes.get(..4) != Some(b"A800") || !matches!(bytes.get(4), Some(1..=5)) {
            return Err("invalid Atari 800 state header".to_string());
        }
        if bytes[4] < 5 && self.profile() != Profile::Atari800 {
            return Err(
                "legacy Atari state belongs to atari-800, not an XL/XE profile".to_string(),
            );
        }
        let (mut state, serial, mut sio, program, used) = if bytes[4] == 1 {
            if self.disk_attached_in(0) || self.disk_attached_in(1) {
                return Err(
                    "legacy Atari 800 state has no disk; eject it before restoring".to_string(),
                );
            }
            let (base, used): (StateV1, usize) = bincode::serde::decode_from_slice(
                bytes,
                bincode::config::standard().with_limit::<67_108_864>(),
            )
            .map_err(|e| format!("invalid legacy Atari 800 state: {e}"))?;
            (
                base,
                pokey::SerialState::default(),
                Sio::new(self.clock_hz()),
                None,
                used,
            )
        } else {
            let (state, used): (State, usize) = bincode::serde::decode_from_slice(
                bytes,
                bincode::config::standard().with_limit::<67_108_864>(),
            )
            .map_err(|e| format!("invalid Atari 800 state: {e}"))?;
            let sio = Sio::restore(state.sio, self.clock_hz())?;
            (state.base, state.serial, sio, state.program, used)
        };
        let (mask, extra) = if bytes[4] >= 3 {
            bincode::serde::decode_from_slice::<Option<bool>, _>(
                &bytes[used..],
                bincode::config::standard(),
            )
            .map_err(|e| format!("invalid Atari 800 IRQ state: {e}"))?
        } else {
            (None, 0)
        };
        let used = used + extra;
        let extra = if bytes[4] >= 4 {
            let (timing, extra): (TimingState, usize) = bincode::serde::decode_from_slice(
                &bytes[used..],
                bincode::config::standard().with_limit::<67_108_864>(),
            )
            .map_err(|e| format!("invalid Atari 800 timing state: {e}"))?;
            if timing.nmi_ready_at > state.hardware.antic.cycles.saturating_add(2) {
                return Err("invalid Atari 800 NMI detection phase".to_string());
            }
            state.hardware.antic.restore_timing(timing.antic)?;
            state.hardware.gtia.restore_graphics(timing.gtia)?;
            state.hardware.pokey.restore_timing(timing.pokey)?;
            state.hardware.nmi_ready_at = timing.nmi_ready_at;
            state.hardware.bus_value = timing.bus_value;
            state.hardware.irq_sample = timing.irq_sample;
            extra
        } else {
            state.hardware.antic.restore_legacy_timing();
            state.hardware.gtia.initialize_legacy_graphics();
            state.hardware.pokey.reconstruct_legacy_timing()?;
            state.hardware.nmi_ready_at = 0;
            state.hardware.bus_value = 0xff;
            state.hardware.irq_sample = state.hardware.irq();
            0
        };
        let used = used + extra;
        let mut cartridge = None;
        let mut tape = None;
        let extra = if bytes[4] == 5 {
            let (control, extra): (VideoControlState, usize) = bincode::serde::decode_from_slice(
                &bytes[used..],
                bincode::config::standard().with_limit::<67_108_864>(),
            )
            .map_err(|e| format!("invalid Atari 800 video/control state: {e}"))?;
            state.hardware.antic.restore_dma(control.antic)?;
            state.hardware.gtia.restore_video(control.gtia)?;
            state.hardware.pia.restore_control(&control.pia)?;
            Mmu::from_state(
                &control.mmu,
                self.profile(),
                state.hardware.ram.len(),
                self.bus.roms.os.len(),
                self.bus.roms.basic.as_ref().map(Vec::len),
            )?;
            sio.restore_media(control.sio)?;
            if let Some(cart) = control.cartridge {
                let mut restored =
                    Cartridge::parse_raw(&cart.rom, Some(Mapper::from_car_type(cart.mapper)?))?;
                restored.restore_bank(cart.bank)?;
                cartridge = Some(restored);
            }
            if let Some(saved) = control.tape {
                let mut restored = CasPlayer::new(Cas::parse(&saved.bytes)?, self.clock_hz())?;
                restored.restore_position(CasPosition {
                    record: saved.record,
                    cycle: saved.cycle,
                })?;
                tape = Some(restored);
            }
            extra
        } else {
            state.hardware.antic.restore_legacy_dma();
            state.hardware.gtia.initialize_legacy_video();
            state.hardware.pia.initialize_legacy_control();
            0
        };
        if used + extra != bytes.len() {
            return Err("invalid Atari 800 state header or trailing bytes".to_string());
        }
        state.cpu.restore_irq_poll_mask(mask);
        if state.rom != self.bus.roms.fingerprint()
            || state.timing != self.timing()
            || state.hardware.antic.pal() != self.pal()
        {
            return Err("Atari 800 state ROM, Region or execution mode differs".to_string());
        }
        if state.hardware.ram.len() != self.profile().ram_size()
            || state.hardware.screen.len() != WIDTH * HEIGHT
            || state.hardware.sample_phase >= self.clock_hz()
        {
            return Err("invalid Atari 800 memory or audio state".to_string());
        }
        state.hardware.antic.validate()?;
        state.hardware.gtia.validate()?;
        state.hardware.pokey.validate()?;
        state.hardware.pia.validate()?;
        state.hardware.pokey.restore_serial(serial)?;
        if let Some(loader) = &program {
            let mut length = 0usize;
            for (start, bytes) in &loader.segments {
                length = length.saturating_add(bytes.len());
                if bytes.is_empty()
                    || bytes.len() > 65536 - usize::from(*start)
                    || bytes.iter().enumerate().any(|(i, _)| {
                        let address = start.wrapping_add(i as u16);
                        cartridge
                            .as_ref()
                            .and_then(|cart| cart.read(address))
                            .is_some()
                            || self
                                .bus
                                .mmu
                                .map_write(
                                    address,
                                    Access::Cpu,
                                    state.hardware.pia.drive(Side::B),
                                    self.bus.roms.basic.is_some(),
                                )
                                .is_none()
                    })
                {
                    return Err("invalid pending Atari XEX segment in State".to_string());
                }
            }
            if length > 16_777_216 || loader.segments.len() > 65536 {
                return Err("pending Atari XEX exceeds State limits".to_string());
            }
        }
        self.cpu = state.cpu;
        self.bus.hardware = state.hardware;
        self.bus.sio = sio;
        self.bus.cartridge = cartridge;
        self.bus.tape = tape;
        self.program = program;
        self.bus.audio.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn xl_machine(profile: Profile) -> Atari800 {
        let mut os = vec![0xea; profile.os_size()];
        let vector = os.len() - 4;
        os[vector..vector + 2].copy_from_slice(&[0, 0xe0]);
        Atari800::with_profile(
            profile,
            Roms {
                os,
                basic: Some(vec![0x55; 8192]),
            },
            true,
            Timing::Accurate,
        )
        .unwrap()
    }

    fn portb(m: &mut Atari800, value: u8) {
        for (address, value) in [(0xd303, 0), (0xd301, 0xff), (0xd303, 4), (0xd301, value)] {
            m.write_bus(address, value);
        }
    }

    #[test]
    fn xl_reset_restores_banking_without_erasing_ram_raster_or_gtia() {
        for profile in [Profile::Atari800Xl, Profile::Atari65Xe, Profile::Atari130Xe] {
            let mut m = xl_machine(profile);
            portb(&mut m, 0xfe);
            assert_eq!(m.load(0xc000, &[0x5a]), 1);
            m.write_bus(0xd01a, 0x24);
            m.write_bus(0xd404, 3);
            m.write_bus(0xd40e, 0xc0);
            let position = m.position();
            m.reset();
            assert_eq!(m.cpu.pc, 0xe000);
            assert_eq!(m.read_memory(0xc000), 0xea);
            assert_eq!(m.bus.hardware.ram[0xc000], 0x5a);
            assert_eq!(m.position(), position);
            assert_eq!(m.chip_registers("gtia")[0x1a], 0x24);
            assert_eq!(m.chip_registers("antic")[4], 3);
            assert_eq!(m.chip_registers("antic")[14], 0);
            assert_eq!(m.read_memory(0xd013), 0, "XL cartridge absent pin");
            assert!(m.set_joystick(2, 0, false).is_err());
        }
    }

    #[test]
    fn xe_cpu_and_antic_fetch_from_independently_selected_physical_ram() {
        for pins in [0xe3, 0xd3] {
            let mut m = xl_machine(Profile::Atari130Xe);
            m.bus.hardware.ram[0x2000..0x2006].copy_from_slice(&[0x4e, 0, 0x40, 0x41, 0, 0x20]);
            m.bus.hardware.ram[0x4000..0x4030].fill(0);
            m.bus.hardware.ram[0x10000..0x10030].fill(0xff);
            portb(&mut m, pins);
            assert_eq!(m.read_memory(0x4000), if pins == 0xe3 { 0xff } else { 0 });
            for (register, value) in [(0, 0x22), (2, 0), (3, 0x20)] {
                m.bus.hardware.antic.write(register, value);
            }
            while m.position() != (40, 8) {
                m.bus.tick();
            }
            assert_eq!(
                m.bus.hardware.antic.pixel_at(64),
                if pins == 0xd3 { 3 } else { 0 }
            );
            let saved = m.save_state();
            portb(&mut m, 0xff);
            m.load_state(&saved).unwrap();
            assert_eq!(m.read_memory(0x4000), if pins == 0xe3 { 0xff } else { 0 });
        }
    }

    #[test]
    fn version5_restores_media_phase_and_rejects_bad_extensions_atomically() {
        type Envelope = (State, Option<bool>, TimingState, VideoControlState);
        let mut m = xl_machine(Profile::Atari130Xe);
        let mut cartridge = vec![0x55; 32768];
        cartridge[..8192].fill(0xaa);
        m.bus.cartridge = Some(Cartridge::parse_raw(&cartridge, Some(Mapper::Xegs32K)).unwrap());
        m.update_cartridge_sense();
        assert_eq!(m.read_memory(0xd013), 1);
        m.write_bus(0xd500, 1);
        assert_eq!(m.load(0x8000, &[3]), 0);
        let mut image = vec![0; 144];
        image[..6].copy_from_slice(&[0x96, 0x02, 8, 0, 128, 0]);
        m.attach_disk_in(1, "atr", &image).unwrap();
        let tape = [b"FUJI".as_slice(), &[0; 4], b"data", &[1, 0, 0, 0, 0x55]].concat();
        m.attach_tape(&tape).unwrap();
        m.write_bus(0xd302, 0x34);
        m.load_program(0x2000, &[0xea, 0x4c, 0, 0x20]).unwrap();
        m.set_pc(0x2000);
        for _ in 0..50 {
            m.step();
        }
        let saved = m.save_state();
        for _ in 0..100 {
            m.step();
        }
        let future = m.save_state();
        m.eject_cartridge();
        m.eject_tape();
        m.eject_disk_in(1).unwrap();
        m.load_state(&saved).unwrap();
        assert_eq!(m.export_disk_in(1).unwrap(), image);
        assert!(m.tape().unwrap().position().cycle > 0);
        assert_eq!(m.cartridge().unwrap().bank(), 1);
        for _ in 0..100 {
            m.step();
        }
        assert_eq!(m.save_state(), future);
        for case in 0..3 {
            let (state, mask, timing, mut control): Envelope =
                bincode::serde::decode_from_slice(&saved, bincode::config::standard())
                    .unwrap()
                    .0;
            match case {
                0 => control.mmu.profile = Profile::Atari65Xe,
                1 => control.cartridge.as_mut().unwrap().bank = 255,
                _ => control.tape.as_mut().unwrap().cycle = u64::MAX,
            }
            let bad = bincode::serde::encode_to_vec(
                (state, mask, timing, control),
                bincode::config::standard(),
            )
            .unwrap();
            assert!(m.load_state(&bad).is_err());
            assert_eq!(m.save_state(), future);
        }
        for cut in [1, 7, 64] {
            assert!(m.load_state(&saved[..saved.len() - cut]).is_err());
            assert_eq!(m.save_state(), future);
        }
    }

    #[test]
    fn published_version4_envelope_migrates_but_cannot_become_an_xl_state() {
        let mut m = machine(Timing::Accurate);
        let saved = m.save_state();
        let (mut state, mask, timing): (State, Option<bool>, TimingState) =
            bincode::serde::decode_from_slice(&saved, bincode::config::standard())
                .unwrap()
                .0;
        state.base.version = 4;
        let old = bincode::serde::encode_to_vec((state, mask, timing), bincode::config::standard())
            .unwrap();
        m.load_state(&old).unwrap();
        assert_eq!(m.save_state()[4], 5);
        let migrated = m.save_state();
        m.load_state(&old).unwrap();
        assert_eq!(m.save_state(), migrated);
        let mut xl = xl_machine(Profile::Atari800Xl);
        let before = xl.save_state();
        assert!(
            xl.load_state(&old)
                .unwrap_err()
                .contains("legacy Atari state belongs")
        );
        assert_eq!(xl.save_state(), before);
    }

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
    fn irq_latch_continues_after_cli_and_version2_restores_live_i() {
        for timing in [Timing::Fast, Timing::Accurate] {
            let mut m = machine(timing);
            m.load_program(0x4000, &[0x58, 0x78, 0xea]).unwrap();
            m.set_pc(0x4000);
            m.step();
            let saved = m.save_state();
            m.step();
            let future = m.save_state();
            m.load_state(&saved).unwrap();
            assert_eq!(m.cpu.irq_poll_mask(), Some(true));
            m.step();
            assert_eq!(m.save_state(), future);
            let (mut old, _): (State, usize) =
                bincode::serde::decode_from_slice(&saved, bincode::config::standard()).unwrap();
            old.base.version = 2;
            let legacy = bincode::serde::encode_to_vec(old, bincode::config::standard()).unwrap();
            m.load_state(&legacy).unwrap();
            assert_eq!(m.cpu.irq_poll_mask(), None);
            let before = m.save_state();
            assert!(m.load_state(&saved[..saved.len() - 1]).is_err());
            assert_eq!(m.save_state(), before);
        }
    }

    #[test]
    fn xex_runs_init_before_later_segments_and_restores_mid_init() {
        use atari_media::{Segment, Xex};
        let bytes = Xex {
            segments: vec![
                Segment {
                    start: 0x2000,
                    bytes: vec![0xa9, 1, 0x8d, 0, 0x40, 0x60],
                },
                Segment {
                    start: 0x2e2,
                    bytes: vec![0, 0x20],
                },
                Segment {
                    start: 0x4000,
                    bytes: vec![2],
                },
                Segment {
                    start: 0x3000,
                    bytes: vec![0xa9, 3, 0x8d, 1, 0x40, 0x4c, 5, 0x30],
                },
                Segment {
                    start: 0x2e0,
                    bytes: vec![0, 0x30],
                },
            ],
        }
        .to_bytes()
        .unwrap();
        let mut m = machine(Timing::Accurate);
        m.load_xex(&bytes).unwrap();
        assert_eq!(m.pc(), 0x2000);
        m.step();
        let saved = m.save_state();
        for _ in 0..20 {
            m.step();
        }
        assert_eq!(m.read_memory(0x4000), 2);
        assert_eq!(m.read_memory(0x4001), 3);
        let future = m.save_state();
        m.load_state(&saved).unwrap();
        for _ in 0..20 {
            m.step();
        }
        assert_eq!(m.save_state(), future);
        let bad = Xex {
            segments: vec![
                Segment {
                    start: 0x4000,
                    bytes: vec![9],
                },
                Segment {
                    start: 0xa000,
                    bytes: vec![9],
                },
            ],
        }
        .to_bytes()
        .unwrap();
        assert!(m.load_xex(&bad).is_err());
        assert_eq!(m.save_state(), future);
    }
    #[test]
    fn legacy_state_is_readable_and_invalid_disk_state_is_atomic() {
        let mut m = machine(Timing::Fast);
        m.load_program(0x4000, &[42]).unwrap();
        let (state, _): (State, usize) =
            bincode::serde::decode_from_slice(&m.save_state(), bincode::config::standard())
                .unwrap();
        let mut old = state.base;
        old.version = 1;
        let bytes = bincode::serde::encode_to_vec(old, bincode::config::standard()).unwrap();
        m.load_program(0x4000, &[99]).unwrap();
        m.load_state(&bytes).unwrap();
        assert_eq!(m.read_memory(0x4000), 42);
        let disk = atari_media::Atr::blank(720, 128).unwrap().to_bytes();
        m.attach_disk(&disk).unwrap();
        let saved = m.save_state();
        assert!(m.load_state(&bytes).is_err());
        assert!(m.attach_disk(b"not an ATR").is_err());
        assert_eq!(m.save_state(), saved);
        m.eject_disk();
        m.load_state(&saved).unwrap();
        assert_eq!(m.export_disk(), Some(disk));
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
    fn rmw_wsync_restarts_the_next_opcode_after_either_refresh_alignment() {
        for start in [20, 21] {
            let mut m = machine(Timing::Accurate);
            m.load_program(0x4000, &[0xee, 0x0a, 0xd4, 0x8d, 0x1e, 0xd0])
                .unwrap();
            m.set_pc(0x4000);
            while m.cycles() < start {
                m.bus.tick();
            }
            m.step();
            m.step();
            assert_eq!(m.bus.hardware.antic.x, 109);
            assert_eq!(m.cpu.pc, 0x4006);
        }
    }

    #[test]
    fn gtia_collision_read_includes_the_current_color_clocks() {
        let mut m = machine(Timing::Accurate);
        m.bus.hardware.ram[0x2000..0x2006].copy_from_slice(&[0x4e, 0, 0x30, 0x41, 0, 0x20]);
        m.bus.hardware.ram[0x3000..0x3030].fill(0xff);
        for (register, value) in [(0, 0x23), (2, 0), (3, 0x20)] {
            m.bus.hardware.antic.write(register, value);
        }
        m.bus.hardware.gtia.write(4, 0xdd);
        m.bus.hardware.gtia.write(0x11, 2);
        while m.bus.hardware.antic.y != 8 || m.bus.hardware.antic.x != 111 {
            m.bus.tick();
        }
        assert_eq!(m.bus.hardware.gtia.read(0), 0);
        let saved = m.save_state();
        m.bus.in_cpu = true;
        assert_eq!(m.bus.read(0xd000), 4);
        m.load_state(&saved).unwrap();
        m.bus.write(0xd0fe, 0);
        assert_eq!(
            m.bus.hardware.gtia.read(0),
            0,
            "HITCLR wins this clock's collision, including mirrored writes"
        );
    }

    #[test]
    fn antic_player_dma_reaches_gtia_collision_latches_and_survives_state() {
        let mut m = machine(Timing::Accurate);
        m.bus.hardware.ram[0x2000..0x2006].copy_from_slice(&[0x4e, 0, 0x30, 0x41, 0, 0x20]);
        m.bus.hardware.ram[0x3000..0x3028].fill(0xff);
        m.bus.hardware.ram[0x3408] = 0x80;
        for (register, value) in [(0, 0x3e), (2, 0), (3, 0x20), (7, 0x30)] {
            m.bus.hardware.antic.write(register, value);
        }
        m.bus.hardware.gtia.write(0, 0x50);
        m.bus.hardware.gtia.write(0x1d, 2);
        while m.bus.hardware.antic.y != 8 || m.bus.hardware.antic.x != 40 {
            m.bus.tick();
        }
        let saved = m.save_state();
        for _ in 0..60 {
            m.bus.tick();
        }
        assert_eq!(m.bus.hardware.gtia.read(4) & 4, 4);
        let future = m.save_state();
        m.load_state(&saved).unwrap();
        for _ in 0..60 {
            m.bus.tick();
        }
        assert_eq!(m.save_state(), future);
    }

    #[test]
    fn irq_asserted_after_the_last_poll_waits_through_one_instruction() {
        let mut m = machine(Timing::Accurate);
        m.load_program(0x4000, &[0xe8, 0xe8, 0xea]).unwrap();
        m.set_pc(0x4000);
        m.cpu.p &= !0x04;
        m.bus.hardware.pia.write_alt(2, 1);
        m.bus.hardware.pia.set_c1(Side::A, false);
        assert!(m.bus.hardware.pia.irq(Side::A));
        let saved = m.save_state();
        assert_eq!(m.step(), 2);
        assert_eq!(m.cpu.x, 1);
        assert_eq!(m.step(), 7);
        let future = m.save_state();
        m.load_state(&saved).unwrap();
        m.step();
        m.step();
        assert_eq!(m.save_state(), future);
    }

    #[test]
    fn timing_extension_continues_chip_pipelines_and_rejects_bad_data_atomically() {
        for timing in [Timing::Fast, Timing::Accurate] {
            let mut m = machine(timing);
            m.load_program(0x4000, &[0xea, 0x4c, 0, 0x40]).unwrap();
            m.set_pc(0x4000);
            m.bus.hardware.pokey.write(15, 3);
            m.bus.hardware.pokey.write(0, 1);
            m.bus.hardware.pokey.write(1, 0x8f);
            m.bus.hardware.pokey.write(8, 0x40);
            for delay in [0, 1, 3, 67] {
                m.bus.hardware.pokey.write(9, 0);
                m.bus.hardware.gtia.write(0, 0x40);
                m.bus.hardware.gtia.write(0x0d, 0x80);
                for _ in 0..delay {
                    m.bus.tick();
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
                assert_eq!(m.save_state(), future, "pipeline delay {delay}");
                let (state, mask, mut extension): (State, Option<bool>, TimingState) =
                    bincode::serde::decode_from_slice::<(State, Option<bool>, TimingState), _>(
                        &saved,
                        bincode::config::standard(),
                    )
                    .unwrap()
                    .0;
                extension.nmi_ready_at = u64::MAX;
                let malformed = bincode::serde::encode_to_vec(
                    (state, mask, extension),
                    bincode::config::standard(),
                )
                .unwrap();
                assert!(
                    m.load_state(&malformed)
                        .unwrap_err()
                        .contains("NMI detection phase")
                );
                assert_eq!(m.save_state(), future);
                for cut in [1, 3, 10] {
                    assert!(m.load_state(&saved[..saved.len() - cut]).is_err());
                    assert_eq!(m.save_state(), future);
                }
            }
        }
    }

    #[test]
    fn virtual_font_dma_samples_cpu_data_without_reading_font_memory() {
        let mut m = machine(Timing::Accurate);
        m.bus.hardware.ram[0x2000..0x2003].copy_from_slice(&[0x57, 0, 0x30]);
        m.bus.hardware.antic.write(0, 0x23);
        m.bus.hardware.antic.write(2, 0);
        m.bus.hardware.antic.write(3, 0x20);
        m.bus.hardware.antic.write(4, 2);
        m.bus.hardware.antic.write(9, 0x40);
        while m.bus.hardware.antic.y != 8 || m.bus.hardware.antic.x != 106 {
            m.bus.tick();
        }
        m.bus.hardware.ram[0x5000] = 0x50;
        m.bus.hardware.bus_value = 0;
        m.bus.in_cpu = true;
        assert_eq!(m.bus.read(0x5000), 0x50);
        assert_eq!(m.bus.hardware.antic.pixel_at(374), 1);
        assert_eq!(m.bus.hardware.bus_value, 0x50);
    }

    #[test]
    fn nmi_pulse_during_status_push_is_lost_but_an_earlier_one_hijacks() {
        for start in [4, 5] {
            let mut m = machine(Timing::Accurate);
            m.bus.roms.os[10234..10236].copy_from_slice(&[0x00, 0xe1]);
            m.bus.roms.os[10238..10240].copy_from_slice(&[0x00, 0xe2]);
            m.load_program(0x2000, &[0, 0xea]).unwrap();
            m.set_pc(0x2000);
            m.bus.hardware.antic.write(14, 0x40);
            while m.bus.hardware.antic.y != 248 || m.bus.hardware.antic.x != start {
                m.bus.tick();
            }
            assert_eq!(m.step(), 7);
            assert_eq!(m.cpu.pc, if start == 4 { 0xe200 } else { 0xe100 });
            assert!(!m.bus.hardware.nmi);
            assert_eq!(m.step(), 2, "no second NMI follows the sequence");
        }
    }

    #[test]
    fn timing_state_preserves_an_nmi_between_edge_and_detection() {
        let mut m = machine(Timing::Accurate);
        m.bus.hardware.antic.write(14, 0x40);
        while m.bus.hardware.antic.y != 248 || m.bus.hardware.antic.x != 9 {
            m.bus.tick();
        }
        assert!(m.bus.hardware.nmi);
        assert_eq!(m.bus.hardware.nmi_ready_at, m.cycles() + 1);
        let saved = m.save_state();
        for _ in 0..3 {
            m.step();
        }
        let future = m.save_state();
        m.load_state(&saved).unwrap();
        for _ in 0..3 {
            m.step();
        }
        assert_eq!(m.save_state(), future);
        let (mut state, _): (State, usize) =
            bincode::serde::decode_from_slice(&saved, bincode::config::standard()).unwrap();
        state.base.version = 3;
        let legacy =
            bincode::serde::encode_to_vec((state, None::<bool>), bincode::config::standard())
                .unwrap();
        m.load_state(&legacy).unwrap();
        assert!(m.bus.hardware.nmi);
        assert_eq!(m.bus.hardware.nmi_ready_at, 0);
    }

    #[test]
    fn accurate_vbi_hijacks_brk_without_changing_its_stack_frame() {
        let mut m = machine(Timing::Accurate);
        m.bus.roms.os[10234..10236].copy_from_slice(&[0x00, 0xe1]);
        m.bus.roms.os[10238..10240].copy_from_slice(&[0x00, 0xe2]);
        // Acid800 cpu_bugs.s: enable VBI, LDA abs, LDA zp, NOP, BRK.
        m.load_program(
            0x2000,
            &[
                0xa9, 0x40, 0x8d, 0x0e, 0xd4, 0xad, 0x00, 0x01, 0xa5, 0x01, 0xea, 0x00,
            ],
        )
        .unwrap();
        m.set_pc(0x2000);
        while m.bus.hardware.antic.y != 247 || m.bus.hardware.antic.x != 104 {
            m.bus.tick();
        }
        for _ in 0..5 {
            m.step();
        }
        assert_eq!(m.cpu.pc, 0x200b);
        let saved = m.save_state();
        assert_eq!(m.step(), 7);
        assert_eq!(m.cpu.pc, 0xe100, "VBI must replace BRK's IRQ vector");
        assert_eq!(m.read_memory(0x01fa), 0x20);
        assert_eq!(m.read_memory(0x01f9), 0x0d);
        assert_eq!(m.read_memory(0x01f8) & 0x30, 0x30);
        assert!(!m.bus.hardware.nmi, "hijacked edge must not fire twice");
        let future = m.save_state();
        m.load_state(&saved).unwrap();
        m.step();
        assert_eq!(m.save_state(), future);
    }

    #[test]
    fn accurate_dma_halts_each_rmw_write() {
        for start in [20, 21] {
            let mut m = machine(Timing::Accurate);
            m.load_program(0x2000, &[0xee, 0x00, 0x40]).unwrap();
            m.set_pc(0x2000);
            m.bus.hardware.ram[0x4000] = 0x81;
            for _ in 0..start {
                m.bus.tick();
            }
            // Either RMW write reaches refresh slot 25: six CPU cycles plus DMA.
            assert_eq!(m.step(), 7, "INC starting at raster cycle {start}");
            assert_eq!(m.read_memory(0x4000), 0x82);
        }
    }

    #[test]
    fn wsync_stalls_reads_not_rmw_writes_and_returns_elapsed_clocks() {
        let mut m = machine(Timing::Accurate);
        m.load_program(0x2000, &[0xee, 0x0a, 0xd4, 0xea]).unwrap();
        m.set_pc(0x2000);
        assert_eq!(m.step(), 6);
        assert!(m.step() > 90);
        assert_eq!(m.cpu.pc, 0x2004);
        assert!(m.cycles() >= 107);
    }
}
