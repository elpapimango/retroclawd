//! The machine: CPU, bus and scheduler, stepped one instruction at a time.
//!
//! Fast mode (spec section 11): the CPU runs a whole instruction, then every event due by the new time is
//! processed at its own timestamp before the next instruction; the 68000 samples interrupts only between
//! instructions (UM 6.3.2), so no event is visible earlier than it would be on the hardware.

use machine::{Addr, FrameRate, Machine, RunStats};
use mc68000::{Cpu, RunState};

use crate::bus::{IoStop, StBus};
use crate::config::{Config, Region};
use crate::diag::Diag;
use crate::scheduler::{self, CLASS_VIDEO, Event, Scheduler};

/// Interrupt level of the vertical blank (ST Internals vector table: `$070` "Interrupt level 4 (VBL)").
const VBL_LEVEL: u8 = 4;
/// The picture until the Shifter exists: black, the size of the high-resolution screen.
const SCREEN: (u32, u32) = (640, 400);

pub struct AtariSt {
    cpu: Cpu,
    bus: StBus,
    sched: Scheduler,
    region: Region,
    now: u64,
    vbl_count: u64,
    last_vbl_at: u64,
    diag: Diag,
}

impl AtariSt {
    pub fn new(cfg: Config) -> Result<Self, String> {
        let mut m = Self {
            cpu: Cpu::new(),
            bus: StBus::new(&cfg)?,
            sched: Scheduler::new(),
            region: cfg.region,
            now: 0,
            vbl_count: 0,
            last_vbl_at: 0,
            diag: Diag::new(cfg.trace_limit),
        };
        m.power_on();
        Ok(m)
    }

    fn power_on(&mut self) {
        self.sched = Scheduler::new();
        self.sched
            .schedule(self.cycles_per_frame(), CLASS_VIDEO, Event::Vbl);
        let clocks = self.cpu.reset(&mut self.bus);
        self.advance(clocks);
    }

    pub fn cycles_per_frame(&self) -> u64 {
        scheduler::cycles_per_frame(self.region)
    }

    /// Vertical blanks so far, and the time of the last one.
    pub fn vbl_count(&self) -> u64 {
        self.vbl_count
    }

    pub fn last_vbl_at(&self) -> u64 {
        self.last_vbl_at
    }

    /// Emulated time in CPU clocks.
    pub fn now(&self) -> u64 {
        self.now
    }

    pub fn io_stop(&self) -> Option<&IoStop> {
        self.bus.stop()
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    /// The PCs of the first `trace_limit` instructions.
    pub fn trace(&self) -> &[u32] {
        self.diag.trace()
    }

    /// The I/O accesses so far, in Hatari's trace format by `IoAccess::hatari_line`.
    pub fn io_log(&self) -> &[crate::iolog::IoAccess] {
        self.bus.io_log()
    }

    pub fn trace_full(&self) -> bool {
        self.diag.trace_full()
    }

    /// The diagnostic report of the I/O stop, once, after the machine has stopped.
    /// The diagnostic report of a stop (an unemulated device, or a halted CPU), once.
    pub fn take_stop_report(&mut self) -> Option<String> {
        if self.diag.reported() {
            return None;
        }
        let clock = self.clock_hz();
        let headline = match self.bus.stop() {
            Some(stop) => Diag::io_headline(stop, clock),
            None if self.cpu.run_state() == RunState::Halted => {
                Diag::halt_headline(self.cpu.halt_reason(), self.now, clock)
            }
            None => return None,
        };
        // The instruction that touched the device, not the one after it.
        let pc = self.diag.last_pc().unwrap_or(self.cpu.pc);
        let bytes: Vec<u8> = (0..10).map(|i| self.bus.peek(pc.wrapping_add(i))).collect();
        let text = format!(
            "${pc:06X} {}",
            mc68000::disassemble::disassemble(&bytes, pc).0
        );
        self.diag.take_report(&self.cpu, &headline, &text)
    }

    /// Lets `clocks` pass and processes the events due by then.
    fn advance(&mut self, clocks: u32) {
        self.now += clocks as u64;
        self.bus.now = self.now;
        while let Some((at, _, event)) = self.sched.pop_due(self.now) {
            match event {
                Event::Vbl => {
                    self.vbl_count += 1;
                    self.last_vbl_at = at;
                    self.bus.vbl_pending = true;
                    self.sched
                        .schedule(at + self.cycles_per_frame(), CLASS_VIDEO, Event::Vbl);
                }
            }
        }
        self.cpu
            .set_ipl(if self.bus.vbl_pending { VBL_LEVEL } else { 0 });
    }
}

impl Machine for AtariSt {
    fn clock_hz(&self) -> u64 {
        (scheduler::crystal_hz(self.region) / scheduler::CPU_DIVIDER) as u64
    }

    fn frame_hz(&self) -> u64 {
        match self.region {
            Region::Pal => 50,
            Region::Ntsc => 60,
        }
    }

    fn frame_rate(&self) -> FrameRate {
        FrameRate {
            num: scheduler::crystal_hz(self.region),
            den: scheduler::CPU_DIVIDER * self.cycles_per_frame() as u32,
        }
    }

    fn run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> RunStats {
        machine::Clock::new(self.frame_hz()).run(self, running)
    }

    fn step(&mut self) -> u32 {
        let clocks = if self.bus.stop().is_some() {
            4
        } else {
            if self.cpu.run_state() == RunState::Running {
                self.diag.instruction(self.cpu.pc, self.cpu.prefetch()[0]);
                self.bus.pc = self.cpu.pc;
            }
            let clocks = self.cpu.step(&mut self.bus);
            if let Some(e) = self.cpu.take_exception() {
                self.diag.exception(e);
            }
            clocks
        };
        self.advance(clocks);
        clocks
    }

    fn halted(&self) -> bool {
        self.bus.stop().is_some() || self.cpu.run_state() == RunState::Halted
    }

    fn pc(&self) -> Addr {
        self.cpu.pc as Addr
    }

    fn registers(&self) -> Vec<(&'static str, u64)> {
        const D: [&str; 8] = ["D0", "D1", "D2", "D3", "D4", "D5", "D6", "D7"];
        const A: [&str; 7] = ["A0", "A1", "A2", "A3", "A4", "A5", "A6"];
        let c = &self.cpu;
        let mut regs: Vec<(&'static str, u64)> =
            D.iter().zip(c.d).map(|(n, v)| (*n, v as u64)).collect();
        regs.extend(A.iter().zip(c.a).map(|(n, v)| (*n, v as u64)));
        regs.extend([
            ("A7", c.a7() as u64),
            ("USP", c.usp as u64),
            ("SSP", c.ssp as u64),
            ("PC", c.pc as u64),
            ("SR", c.sr() as u64),
        ]);
        regs
    }

    fn render_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
        Some((
            SCREEN.0,
            SCREEN.1,
            vec![0; (SCREEN.0 * SCREEN.1 * 3) as usize],
        ))
    }

    fn read_memory(&self, addr: Addr) -> u8 {
        self.bus.peek(addr as u32)
    }

    fn load(&mut self, addr: Addr, bytes: &[u8]) -> usize {
        bytes
            .iter()
            .enumerate()
            .take_while(|(i, b)| self.bus.poke(addr as u32 + *i as u32, **b))
            .count()
    }

    fn set_pc(&mut self, addr: Addr) {
        let pc = addr as u32 & 0xFF_FFFE;
        self.cpu.pc = pc;
        let word = |a: u32| u16::from_be_bytes([self.bus.peek(a), self.bus.peek(a + 1)]);
        self.cpu.set_prefetch([word(pc), word(pc + 2)]);
    }

    fn save_state(&self) -> Vec<u8> {
        Vec::new()
    }

    fn load_state(&mut self, _bytes: &[u8]) -> Result<(), String> {
        Err("Atari ST save states arrive in Phase 2".into())
    }

    /// The reset button: CPU reset exception, RAM kept (spec section 14).
    fn reset(&mut self) {
        let clocks = self.cpu.reset(&mut self.bus);
        self.advance(clocks);
    }
}
