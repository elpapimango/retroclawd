//! The machine: CPU, bus and scheduler, stepped one instruction at a time.
//!
//! Fast mode (spec section 11): the CPU runs a whole instruction, then every event due by the new time is
//! processed at its own timestamp before the next instruction; the 68000 samples interrupts only between
//! instructions (UM 6.3.2), so no event is visible earlier than it would be on the hardware.

use machine::{Addr, FrameRate, Machine, RunStats};
use mc68000::{Cpu, RunState};

use crate::acia::SerialPeer;
use crate::bus::{IoStop, StBus};
use crate::config::{Config, Region};
use crate::diag::Diag;
use crate::dma::FDC_BYTE_CYCLES;
use crate::scheduler::{
    self, CLASS_ACIA, CLASS_FDC, CLASS_MFP, CLASS_VIDEO, Event, FrameGeometry, Scheduler,
};

pub struct AtariSt {
    cpu: Cpu,
    bus: StBus,
    sched: Scheduler,
    region: Region,
    now: u64,
    vbl_count: u64,
    last_vbl_at: u64,
    hbl_count: u64,
    /// This frame's shape (chosen at its start) and the lines done in it.
    geometry: FrameGeometry,
    line: u32,
    /// The number of the one MFP timeout event that is current.
    mfp_event: u64,
    /// The picture drawn at the last VBL (640 x 400 RGB).
    frame: Vec<u8>,
    diag: Diag,
}

impl AtariSt {
    pub fn new(cfg: Config) -> Result<Self, String> {
        let bus = StBus::new(&cfg)?;
        let geometry = scheduler::geometry(bus.shifter.sync_50hz(), cfg.monitor)?;
        let mut m = Self {
            cpu: Cpu::new(),
            bus,
            sched: Scheduler::new(),
            region: cfg.region,
            now: 0,
            vbl_count: 0,
            last_vbl_at: 0,
            hbl_count: 0,
            geometry,
            line: 0,
            mfp_event: 0,
            frame: vec![0; (st_shifter::WIDTH * st_shifter::HEIGHT * 3) as usize],
            diag: Diag::new(cfg.trace_limit),
        };
        m.power_on();
        Ok(m)
    }

    fn power_on(&mut self) {
        self.sched = Scheduler::new();
        self.sched.schedule(
            self.geometry.clocks_per_line as u64,
            CLASS_VIDEO,
            Event::LineEnd,
        );
        // The IKBD powers up with the machine: its version code goes out after the self-test.
        let cps = self.clock_hz();
        if let Some(k) = self.bus.acias.ikbd() {
            k.tick_clock(0, cps);
        }
        self.bus.acias.ports[0].pull_rx(0);
        self.schedule_acia(0);
        let clocks = self.cpu.reset(&mut self.bus);
        self.advance(clocks);
    }

    /// The current frame's length in CPU clocks.
    pub fn cycles_per_frame(&self) -> u64 {
        self.geometry.clocks()
    }

    /// Lines ended so far (each one an HBL request).
    pub fn hbl_count(&self) -> u64 {
        self.hbl_count
    }

    /// MFP timer clocks so far.
    pub fn mfp_ticks(&self) -> u64 {
        self.bus.mfp_ticks()
    }

    pub fn bus(&self) -> &StBus {
        &self.bus
    }

    /// The line of the current frame.
    pub fn line(&self) -> u32 {
        self.line
    }

    /// The time of the next scheduled event.
    pub fn next_event_at(&self) -> Option<u64> {
        self.sched.next_at()
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

    /// Lets `clocks` pass and processes the events due by then, each at its own time.
    fn advance(&mut self, clocks: u32) {
        // An MFP write during the instruction happened at its start (fast mode, `bus.now`): reschedule from there,
        // before the events the instruction crossed, so none of them sees the MFP ahead of its own time.
        if std::mem::take(&mut self.bus.fdc_dirty) {
            self.schedule_fdc(self.bus.now);
        }
        if std::mem::take(&mut self.bus.acia_dirty) {
            self.schedule_acia(0);
            self.schedule_acia(1);
        }
        if std::mem::take(&mut self.bus.mfp_dirty) {
            self.schedule_mfp(self.now);
        }
        self.now += clocks as u64;
        self.bus.now = self.now;
        while let Some((at, _, event)) = self.sched.pop_due(self.now) {
            match event {
                Event::LineEnd => self.line_end(at),
                Event::MfpTimeout(n) if n == self.mfp_event => {
                    self.bus.catch_up_mfp(at);
                    self.schedule_mfp(at);
                }
                Event::MfpTimeout(_) => {}
                Event::AciaService { port, generation }
                    if generation == self.bus.acias.ports[port as usize].generation =>
                {
                    self.bus.acias.ports[port as usize].service(at);
                    self.bus.update_acia_irq(at);
                    self.bus.mfp_dirty = false;
                    self.schedule_mfp(at);
                    self.schedule_acia(port as usize);
                }
                Event::AciaService { .. } => {}
                Event::FdcTick(g) if g == self.bus.dma.generation => {
                    self.bus.catch_up_fdc(at);
                    if std::mem::take(&mut self.bus.mfp_dirty) {
                        self.schedule_mfp(at);
                    }
                    self.schedule_fdc(at);
                }
                Event::FdcTick(_) => {}
            }
        }
        self.cpu.set_ipl(self.bus.ipl());
    }

    /// The end of a line: the HBL; after the frame's last line the VBL, the picture, and the next frame's shape
    /// from the sync mode then in force.
    fn line_end(&mut self, at: u64) {
        self.hbl_count += 1;
        self.bus.raise_hbl();
        self.line += 1;
        if self.line == self.geometry.lines {
            self.line = 0;
            self.vbl_count += 1;
            self.last_vbl_at = at;
            self.bus.raise_vbl();
            self.bus.catch_up_mfp(at);
            self.bus.psg.catch_up(at);
            self.bus.catch_up_fdc(at);
            let cps = self.clock_hz();
            if let Some(k) = self.bus.acias.ikbd() {
                k.tick_clock(at, cps);
            }
            self.render();
            // A mono monitor is refused at creation, so this cannot fail later.
            if let Ok(g) = scheduler::geometry(self.bus.shifter.sync_50hz(), self.bus.monitor) {
                self.geometry = g;
            }
        }
        self.sched.schedule(
            at + self.geometry.clocks_per_line as u64,
            CLASS_VIDEO,
            Event::LineEnd,
        );
    }

    /// One MFP timeout event at the next timer timeout after `at`; any older one is now stale.
    fn schedule_mfp(&mut self, at: u64) {
        self.bus.catch_up_mfp(at);
        self.mfp_event += 1;
        if let Some(ticks) = self.bus.mfp().next_timeout() {
            let clock = self.bus.mfp_clock();
            let when = clock.cycle_for(self.bus.mfp_ticks() + ticks).max(at);
            self.sched
                .schedule(when, CLASS_MFP, Event::MfpTimeout(self.mfp_event));
        }
    }

    /// The controller's next byte time while it works; any older event is now stale.
    fn schedule_fdc(&mut self, at: u64) {
        self.bus.dma.generation += 1;
        if self.bus.dma.busy() {
            let generation = self.bus.dma.generation;
            self.sched
                .schedule(at + FDC_BYTE_CYCLES, CLASS_FDC, Event::FdcTick(generation));
        }
    }

    /// Puts `image` in drive `drive` (0 = A, 1 = B).
    pub fn insert_disk(&mut self, drive: u8, image: st_floppy::Image) -> Result<(), String> {
        self.bus.catch_up_fdc(self.now);
        self.bus.dma.fdc.floppy.insert(drive as usize, image)
    }

    /// Takes the disk out of drive `drive`.
    pub fn eject_disk(&mut self, drive: u8) -> Option<st_floppy::Image> {
        self.bus.catch_up_fdc(self.now);
        self.bus.dma.fdc.floppy.drive_mut(drive as usize).eject()
    }

    /// The disk in drive `drive`, if any.
    pub fn disk(&self, drive: u8) -> Option<&st_floppy::Image> {
        self.bus.dma.fdc.floppy.drive(drive as usize).image()
    }

    /// Floppy drives fitted.
    pub fn drives(&self) -> u8 {
        self.bus.dma.fdc.floppy.fitted()
    }

    /// Phase 1 never writes to a disk.
    pub fn disk_modified(&self, _drive: u8) -> bool {
        false
    }

    /// The number of floppy drives (1 or 2, default 2), before the machine runs; any disks are removed.
    pub fn set_drives(&mut self, n: u8) {
        self.bus.dma.fdc.floppy = crate::drive::Drives::new(n);
    }

    /// One service event at ACIA `port`'s next due action; any older one is now stale.
    fn schedule_acia(&mut self, port: usize) {
        let p = &mut self.bus.acias.ports[port];
        p.generation += 1;
        if let Some(at) = p.next_due() {
            let generation = p.generation;
            self.sched.schedule(
                at.max(self.bus.now),
                CLASS_ACIA,
                Event::AciaService {
                    port: port as u8,
                    generation,
                },
            );
        }
    }

    /// Connects `peer` to ACIA `port` (0 keyboard, 1 MIDI).
    pub fn attach_serial_peer(&mut self, port: usize, peer: Box<dyn SerialPeer + Send>) {
        let now = self.now;
        let p = &mut self.bus.acias.ports[port];
        p.peer = crate::acia::Peer::Other(peer);
        p.drop_rx();
        p.pull_rx(now);
        self.schedule_acia(port);
    }

    /// Runs `f` on the IKBD at the current time, then lets the keyboard ACIA collect what it queued.
    fn with_ikbd(&mut self, f: impl FnOnce(&mut ikbd::Ikbd, u64)) {
        let now = self.now;
        if let Some(k) = self.bus.acias.ikbd() {
            f(k, now);
        }
        self.bus.acias.ports[0].pull_rx(now);
        self.schedule_acia(0);
    }

    /// A key of the ST keyboard went down or up; `name` is one of `ikbd::SCANCODES`.
    pub fn key_event(&mut self, name: &str, down: bool) -> Result<(), String> {
        let code = ikbd::scancode(name)
            .ok_or_else(|| format!("the Atari ST has no key named {name:?}"))?;
        self.with_ikbd(|k, now| k.key(code, down, now));
        Ok(())
    }

    /// Mouse motion in mouse clicks (X right, Y down positive).
    pub fn mouse_motion(&mut self, dx: i32, dy: i32) {
        self.with_ikbd(|k, now| k.mouse_move(dx, dy, now));
    }

    /// A mouse button (`left` false = right) went down or up.
    pub fn mouse_button(&mut self, left: bool, down: bool) {
        self.with_ikbd(|k, now| k.mouse_button(left, down, now));
    }

    /// Joystick `port` (0 or 1) now reads `bits` (bit 7 fire, bits 0-3 the stick; see `ikbd::Ikbd::joystick`).
    pub fn joystick(&mut self, port: u8, bits: u8) {
        self.with_ikbd(|k, now| k.joystick(port, bits, now));
    }

    /// When ACIA `port`'s transmitter last took a character into its shift register.
    pub fn serial_tx_started_at(&self, port: usize) -> Option<u64> {
        self.bus.acias.tx_started_at(port)
    }

    /// The bus, for tests that continue with bus cycles of their own.
    pub fn into_bus(self) -> StBus {
        self.bus
    }

    fn render(&mut self) {
        let bus = &self.bus;
        bus.shifter
            .render(bus.monitor, &|a| bus.peek(a), &mut self.frame);
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

    /// The power-on frame (the host paces by it; TOS may switch 50/60 Hz later).
    fn frame_rate(&self) -> FrameRate {
        FrameRate {
            num: scheduler::crystal_hz(self.region),
            den: scheduler::CPU_DIVIDER * scheduler::cycles_per_frame(self.region) as u32,
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

    /// The YM2149's output, mono at 44 100 Hz, up to now.
    fn drain_audio(&mut self) -> Vec<f32> {
        self.bus.psg.catch_up(self.now);
        self.bus.psg.drain()
    }

    fn render_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
        Some((st_shifter::WIDTH, st_shifter::HEIGHT, self.frame.clone()))
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

    /// The reset button: the RESET line resets the CPU and the MFP ([D] MC68901 §2.2.3); RAM kept (spec section 9).
    fn reset(&mut self) {
        mc68000::M68kBus::reset_devices(&mut self.bus);
        let clocks = self.cpu.reset(&mut self.bus);
        self.advance(clocks);
    }
}
