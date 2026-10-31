//! The two processors and the bus together, advanced in master clocks (24 MHz): the 68000 at master / 2, the Z80 at
//! master / 6 (fact N4), the Z80 run lazily up to the 68000's clock.

use crate::Bus;
use crate::video::{self, Sources, SpriteLayout};
use mc68000::Cpu;
use std::io::Read;

const Z80_DIVIDER: u64 = 6;

/// The graphics data that is not in the bus: the board's fix tiles and the cartridge's S and C ROMs.
pub struct Graphics {
    pub sfix: Vec<u8>,
    /// The vertical shrink table, `000-lo.lo` of the system set.
    pub lo: Vec<u8>,
    pub s: Vec<u8>,
    pub c: Vec<u8>,
}

/// The 68000 BIOS files tried in order when none is asked for (fact N2).
const BIOS_CHOICES: [&str; 6] = [
    "sp-s2.sp1",
    "sp-s3.sp1",
    "sp-u2.sp1",
    "sp-e.sp1",
    "uni-bios_3_3.rom",
    "uni-bios_4_0.rom",
];

/// Master clocks in a frame (fact N20).
pub const FRAME_CLOCKS: u64 = crate::LINE_CLOCKS * crate::FRAME_LINES;

pub struct NeoGeo {
    pub graphics: Graphics,
    pub cpu: Cpu,
    pub z80: z80::Cpu,
    pub bus: Bus,
    /// Master clocks the Z80 has run up to.
    z80_clock: u64,
    nmi_was_high: bool,
    /// A debugging aid: with `trace_nth_nmi` set, the Z80 program counters from the entry of that NMI on (up to 600).
    pub trace_nth_nmi: Option<usize>,
    pub z80_trace: Vec<u16>,
    nmis: usize,
}

impl NeoGeo {
    /// Resets the 68000 from the vector table the bus shows at `$000000`.
    pub fn new(mut bus: Bus, graphics: Graphics) -> NeoGeo {
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        NeoGeo {
            graphics,
            cpu,
            z80: z80::Cpu::new(),
            bus,
            z80_clock: 0,
            nmi_was_high: false,
            trace_nth_nmi: None,
            z80_trace: Vec::new(),
            nmis: 0,
        }
    }

    /// Builds the machine from the BIOS set (`neogeo.zip`) and a cartridge set (fact N2, N24).
    pub fn from_sets(bios_zip: &[u8], game_zip: &[u8]) -> Result<NeoGeo, String> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bios_zip))
            .map_err(|e| format!("rom_bios is not a readable zip: {e}"))?;
        let mut file = |name: &str| -> Option<Vec<u8>> {
            let mut data = Vec::new();
            zip.by_name(name)
                .ok()?
                .take(1 << 20)
                .read_to_end(&mut data)
                .ok()?;
            Some(data)
        };
        let bios = BIOS_CHOICES.iter().find_map(|n| file(n)).ok_or(
            "rom_bios holds none of the 68000 BIOS files this machine knows (sp-s2.sp1, ...)",
        )?;
        let sm1 = file("sm1.sm1").ok_or("rom_bios has no sm1.sm1 (the sound program)")?;
        let sfix = file("sfix.sfix").ok_or("rom_bios has no sfix.sfix (the fix tiles)")?;
        let lo = file("000-lo.lo").ok_or("rom_bios has no 000-lo.lo (the shrink table)")?;
        let set = neo_set::load(game_zip).map_err(|e| format!("the game set: {e}"))?;
        let graphics = Graphics {
            sfix,
            lo,
            s: set.s,
            c: set.c,
        };
        let mut bus = Bus::new(bios, set.p, sm1, set.m);
        bus.sound.v_a = set.v_a;
        bus.sound.v_b = set.v_b;
        Ok(NeoGeo::new(bus, graphics))
    }

    /// The picture as RGB bytes, 320 x 224 (fact N29-N32).
    pub fn render_rgb(&self) -> (u32, u32, Vec<u8>) {
        let bus = &self.bus;
        let bank = usize::from(bus.latch & 0x80 == 0) * crate::PALETTE_RAM;
        let src = Sources {
            vram: &bus.vram,
            palette: &bus.palette[bank..bank + crate::PALETTE_RAM],
            fix: if bus.latch & 0x20 != 0 && !self.graphics.s.is_empty() {
                &self.graphics.s
            } else {
                &self.graphics.sfix
            },
            sprites: &self.graphics.c,
            layout: SpriteLayout::default(),
            lo: &self.graphics.lo,
            anim: bus.anim,
        };
        let mut out = vec![0u32; video::WIDTH * video::HEIGHT];
        video::render(&src, &mut out);
        let mut rgb = Vec::with_capacity(out.len() * 3);
        for px in out {
            rgb.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, px as u8]);
        }
        (video::WIDTH as u32, video::HEIGHT as u32, rgb)
    }

    /// The sound made since the last call: interleaved left and right at `ym2610::SAMPLE_RATE`, as floats (fact Y1).
    pub fn drain_audio(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        for (l, r) in self.bus.sound.ym.take_samples() {
            out.push(f32::from(l) / 32768.0);
            out.push(f32::from(r) / 32768.0);
        }
        out
    }

    /// A host key by name (`Joy1Up`, `Coin1`, `Start1`...); `false` for a name this machine does not have (fact N33).
    pub fn set_key(&mut self, name: &str, pressed: bool) -> bool {
        let bit = |mask: &mut u8, n: u8| {
            if pressed {
                *mask |= 1 << n;
            } else {
                *mask &= !(1 << n);
            }
        };
        let name = name.to_ascii_lowercase();
        let pad = |player: usize, button: &str| -> Option<u8> {
            let _ = player;
            ["up", "down", "left", "right", "a", "b", "c", "d"]
                .iter()
                .position(|b| *b == button)
                .map(|n| n as u8)
        };
        if let Some(rest) = name.strip_prefix("joy")
            && let Some(player) = rest.chars().next().and_then(|c| c.to_digit(10))
            && (1..=2).contains(&player)
            && let Some(n) = pad(player as usize, &rest[1..])
        {
            let held = &mut self.bus.pads[player as usize - 1];
            if pressed {
                *held &= !(1 << n);
            } else {
                *held |= 1 << n;
            }
            return true;
        }
        match name.as_str() {
            "coin1" => bit(&mut self.bus.pressed_a, 0),
            "coin2" => bit(&mut self.bus.pressed_a, 1),
            "service" => bit(&mut self.bus.pressed_a, 2),
            "start1" => bit(&mut self.bus.pressed_b, 0),
            "select1" => bit(&mut self.bus.pressed_b, 1),
            "start2" => bit(&mut self.bus.pressed_b, 2),
            "select2" => bit(&mut self.bus.pressed_b, 3),
            _ => return false,
        }
        true
    }

    /// Runs one 68000 instruction, then the Z80 up to the same time. Returns the master clocks that passed.
    pub fn step(&mut self) -> u64 {
        self.cpu.set_ipl(self.bus.ipl());
        let clocks = u64::from(self.cpu.step(&mut self.bus)) * 2;
        self.bus.advance(clocks);
        self.catch_up_z80();
        clocks
    }

    fn catch_up_z80(&mut self) {
        while self.z80_clock < self.bus.clock {
            if self.bus.sound.reset_requested {
                self.bus.sound.reset_requested = false;
                self.z80 = z80::Cpu::new();
                self.nmi_was_high = false;
            }
            // the NMI line is edge triggered
            let high = self.bus.sound.nmi_line();
            if high && !self.nmi_was_high {
                self.z80.request_nmi();
            }
            self.nmi_was_high = high;
            // the sound chip's interrupt is a level: asked for on every step while it is up (fact N38)
            if self.bus.sound.ym.irq() {
                self.z80.request_interrupt();
            }
            if self.z80.pc == 0x66 {
                self.nmis += 1;
            }
            if self.trace_nth_nmi == Some(self.nmis) && self.z80_trace.len() < 600 {
                self.z80_trace.push(self.z80.pc);
            }
            let t = self.z80.step(&mut self.bus);
            // the chip runs at 8 MHz, twice the Z80's clock
            let s = &mut self.bus.sound;
            let rom_b = if s.v_b.is_empty() { &s.v_a } else { &s.v_b };
            s.ym.advance(u32::from(t.max(1)) * 2, &s.v_a, rom_b);
            self.z80_clock += u64::from(t.max(1)) * Z80_DIVIDER;
        }
    }
}
