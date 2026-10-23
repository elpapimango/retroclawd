//! The machine: CPU, bus and scheduler, stepped one instruction at a time.
//!
//! Fast mode (spec section 11): the CPU runs a whole instruction, then every event due by the new time is
//! processed at its own timestamp before the next instruction; the 68000 samples interrupts only between
//! instructions (UM 6.3.2), so no event is visible earlier than it would be on the hardware.

use machine::{Addr, FrameRate, Machine, RunStats};
use mc68000::{Cpu, RunState};
use serde::{Deserialize, Serialize};

use crate::acia::SerialPeer;
use crate::bus::{IoStop, StBus};
use crate::config::{Accuracy, Config, Region};
use crate::diag::Diag;
use crate::scheduler::{self, CLASS_VIDEO, Event, Scheduler, SchedulerState};
use crate::state::{self, DriveMeta, Header, StateError};

/// The machine as data (the payload of a save state; see `state.rs` for the envelope around it).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatePayload {
    cpu: Cpu,
    now: u64,
    vbl_count: u64,
    last_vbl_at: u64,
    hbl_count: u64,
    clocks_per_line: u32,
    lines: u32,
    line: u32,
    mfp_event: u64,
    sched: SchedulerState,
    frame: Vec<u8>,
    bus: crate::bus::BusState,
}

pub struct AtariSt {
    cpu: Cpu,
    bus: StBus,
    diag: Diag,
    accuracy: Accuracy,
    /// The plugin's version tag, recorded in the states this machine saves.
    version_tag: String,
    state_warnings: Vec<String>,
}

impl AtariSt {
    pub fn new(cfg: Config) -> Result<Self, String> {
        let bus = StBus::new(&cfg)?;
        let mut m = Self {
            cpu: Cpu::new(),
            bus,
            diag: Diag::new(cfg.trace_limit),
            accuracy: cfg.accuracy,
            version_tag: "dev".to_string(),
            state_warnings: Vec::new(),
        };
        m.power_on();
        Ok(m)
    }

    fn power_on(&mut self) {
        self.bus.sched = Scheduler::new();
        self.bus.sched.schedule(
            self.bus.geometry.clocks_per_line as u64,
            CLASS_VIDEO,
            Event::LineEnd,
        );
        // The IKBD powers up with the machine: its version code goes out after the self-test.
        let cps = self.clock_hz();
        if let Some(k) = self.bus.acias.ikbd() {
            k.tick_clock(0, cps);
        }
        self.bus.acias.ports[0].pull_rx(0);
        self.bus.schedule_acia(0);
        let start = self.bus.now;
        let clocks = self.cpu.reset(&mut self.bus);
        self.after_cpu(start, clocks);
    }

    /// The CPU ran for `clocks` from `start`: in `fast` mode that time passes now, in `accurate` mode the bus
    /// cycles let it pass as they happened (and waited for their slots), and only the interrupt level is left to hand
    /// to the CPU. Returns the clocks that went by.
    fn after_cpu(&mut self, start: u64, clocks: u32) -> u32 {
        if self.bus.accurate {
            debug_assert!(
                self.bus.now >= start + clocks as u64,
                "the bus cycles and the internal clocks cover the instruction's clocks"
            );
            self.cpu.set_ipl(self.bus.ipl());
            (self.bus.now - start) as u32
        } else {
            self.advance(clocks);
            clocks
        }
    }

    /// Lets `clocks` pass (events due run at their own times) and shows the CPU the interrupt level that results.
    fn advance(&mut self, clocks: u32) {
        self.bus.advance(clocks);
        self.cpu.set_ipl(self.bus.ipl());
    }

    /// The current frame's length in CPU clocks.
    pub fn cycles_per_frame(&self) -> u64 {
        self.bus.geometry.clocks()
    }

    /// Lines ended so far (each one an HBL request).
    pub fn hbl_count(&self) -> u64 {
        self.bus.hbl_count
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
        self.bus.line
    }

    /// The time of the next scheduled event.
    pub fn next_event_at(&self) -> Option<u64> {
        self.bus.sched.next_at()
    }

    /// Vertical blanks so far, and the time of the last one.
    pub fn vbl_count(&self) -> u64 {
        self.bus.vbl_count
    }

    pub fn last_vbl_at(&self) -> u64 {
        self.bus.last_vbl_at
    }

    /// Emulated time in CPU clocks.
    pub fn now(&self) -> u64 {
        self.bus.now
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

    /// The I/O accesses logged since the last call, removed: a long run can be logged in pieces.
    pub fn take_io_log(&mut self) -> Vec<crate::iolog::IoAccess> {
        self.bus.take_io_log()
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
                Diag::halt_headline(self.cpu.halt_reason(), self.bus.now, clock)
            }
            None => return None,
        };
        // The instruction that touched the device, not the one after it.
        let pc = self.diag.last_pc().unwrap_or(self.cpu.pc);
        let bytes: Vec<u8> = (0..10).map(|i| self.bus.peek(pc.wrapping_add(i))).collect();
        let mut text = format!(
            "${pc:06X} {}",
            mc68000::disassemble::disassemble(&bytes, pc).0
        );
        let mut sync: Vec<String> = self
            .bus
            .shifter
            .sync_log()
            .map(|w| {
                let register = match w.register {
                    st_shifter::SyncRegister::Sync => "sync",
                    st_shifter::SyncRegister::Resolution => "resolution",
                };
                format!("line {} cycle {}: {register}={}", w.line, w.cycle, w.value)
            })
            .collect();
        if !sync.is_empty() {
            let skipped = sync.len().saturating_sub(8);
            sync.drain(..skipped);
            text.push_str(&format!(
                "\nvideo: sync and resolution writes this frame, last {}: {}",
                sync.len(),
                sync.join("; ")
            ));
        }
        self.diag.take_report(&self.cpu, &headline, &text)
    }

    /// Puts `image` in drive `drive` (0 = A, 1 = B).
    pub fn insert_disk(&mut self, drive: u8, image: st_floppy::Image) -> Result<(), String> {
        self.bus.catch_up_fdc(self.bus.now);
        self.bus.dma.fdc.floppy.insert(drive as usize, image)
    }

    /// Takes the disk out of drive `drive`.
    pub fn eject_disk(&mut self, drive: u8) -> Option<st_floppy::Image> {
        self.bus.catch_up_fdc(self.bus.now);
        self.bus.dma.fdc.floppy.eject(drive as usize)
    }

    /// The disk in drive `drive`, if any.
    ///
    /// A track the controller is still writing is not in the image until it is committed (the controller goes idle,
    /// the head moves, the disk is ejected or [`AtariSt::mark_disk_saved`]); `commit_disks` takes it in now.
    pub fn disk(&self, drive: u8) -> Option<&st_floppy::Image> {
        self.bus.dma.fdc.floppy.drive(drive as usize).image()
    }

    /// Takes any track the controller has written into the images.
    pub fn commit_disks(&mut self) {
        self.bus.catch_up_fdc(self.bus.now);
        self.bus.dma.fdc.floppy.commit_pending();
    }

    /// Floppy drives fitted.
    pub fn drives(&self) -> u8 {
        self.bus.dma.fdc.floppy.fitted()
    }

    /// Whether the controller has written to the disk in `drive` since it was inserted or last marked saved.
    pub fn disk_modified(&self, drive: u8) -> bool {
        self.bus.dma.fdc.floppy.drive(drive as usize).is_modified()
    }

    /// The disk in `drive` has been saved: it is no longer modified.
    pub fn mark_disk_saved(&mut self, drive: u8) {
        self.bus.catch_up_fdc(self.bus.now);
        self.bus
            .dma
            .fdc
            .floppy
            .drive_mut(drive as usize)
            .mark_saved();
    }

    /// Sets the write-protect tab of the disk in `drive`.
    pub fn set_write_protect(&mut self, drive: u8, on: bool) {
        self.bus
            .dma
            .fdc
            .floppy
            .drive_mut(drive as usize)
            .set_write_protected(on);
    }

    /// Names the build in the states this machine saves (the plugin passes its version tag).
    pub fn set_version_tag(&mut self, tag: &str) {
        self.version_tag = tag.to_string();
    }

    /// Notes made by the last loaded state (a different disk in a drive than the state was saved with), once.
    pub fn take_state_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.state_warnings)
    }

    /// The accuracy in the state header: bit 0 accurate, bit 1 borders, bit 2 mono monitor (a state is only loaded by the
    /// same canvas and monitor).
    fn accuracy_code(&self) -> u8 {
        let accuracy = match self.accuracy {
            Accuracy::Fast => 0,
            Accuracy::Accurate => 1,
        };
        accuracy
            | if self.bus.borders { 2 } else { 0 }
            | if self.bus.monitor == st_shifter::Monitor::Mono {
                4
            } else {
                0
            }
    }

    /// Draws the picture with its borders (`borders = on`), which needs `Accuracy::Accurate`.
    pub fn set_borders(&mut self, on: bool) -> Result<(), String> {
        if on && self.accuracy != Accuracy::Accurate {
            return Err("borders need accuracy = accurate".to_string());
        }
        self.bus.borders = on;
        let (w, h) = self.canvas();
        self.bus.frame = vec![0; (w * h * 3) as usize];
        Ok(())
    }

    /// The size of the picture `render_frame` returns.
    pub fn canvas(&self) -> (u32, u32) {
        if self.bus.borders {
            (st_shifter::OVERSCAN_WIDTH, st_shifter::OVERSCAN_HEIGHT)
        } else {
            (st_shifter::WIDTH, st_shifter::HEIGHT)
        }
    }

    /// What the machine is as data, or `Stopped` when it has stopped at an unemulated device.
    pub fn snapshot(&self) -> Result<StatePayload, StateError> {
        Ok(StatePayload {
            cpu: self.cpu.clone(),
            now: self.bus.now,
            vbl_count: self.bus.vbl_count,
            last_vbl_at: self.bus.last_vbl_at,
            hbl_count: self.bus.hbl_count,
            clocks_per_line: self.bus.geometry.clocks_per_line,
            lines: self.bus.geometry.lines,
            line: self.bus.line,
            mfp_event: self.bus.mfp_event,
            sched: self.bus.sched.snapshot(),
            frame: self.bus.frame.clone(),
            bus: self.bus.snapshot().ok_or(StateError::Stopped)?,
        })
    }

    /// Becomes the machine `p` describes. The payload is untrusted: it is checked against this machine first, and a
    /// refused payload changes nothing.
    pub fn restore(&mut self, p: StatePayload) -> Result<(), StateError> {
        let bad = |why: &str| StateError::Payload(why.to_string());
        let frames = [
            scheduler::geometry(true, self.bus.monitor),
            scheduler::geometry(false, self.bus.monitor),
        ];
        let geometry = frames
            .into_iter()
            .flatten()
            .find(|g| g.clocks_per_line == p.clocks_per_line && g.lines == p.lines)
            .ok_or_else(|| bad("the frame shape is not one this machine draws"))?;
        if p.line >= geometry.lines {
            return Err(bad("the raster line is outside the frame"));
        }
        if p.frame.len() != self.bus.frame.len() {
            return Err(bad("the picture is not the size this machine draws"));
        }
        if p.sched.has_bad_port() {
            return Err(bad(
                "the event queue names an ACIA this machine does not have",
            ));
        }
        self.bus.restore(p.bus).map_err(StateError::Payload)?;
        self.cpu = p.cpu;
        self.bus.now = p.now;
        self.bus.vbl_count = p.vbl_count;
        self.bus.last_vbl_at = p.last_vbl_at;
        self.bus.hbl_count = p.hbl_count;
        self.bus.geometry = geometry;
        self.bus.line = p.line;
        self.bus.mfp_event = p.mfp_event;
        self.bus.sched = Scheduler::restore(p.sched);
        self.bus.frame = p.frame;
        Ok(())
    }

    fn drive_meta(&self) -> [DriveMeta; 2] {
        let mut out = [DriveMeta::default(); 2];
        for (n, slot) in out.iter_mut().enumerate().take(self.drives() as usize) {
            if let Some(image) = self.disk(n as u8) {
                *slot = DriveMeta {
                    present: true,
                    sha1: state::sha1_of(&image.to_bytes()),
                    dirty: self.disk_modified(n as u8),
                };
            }
        }
        out
    }

    /// The state in the `RCST` envelope.
    pub fn save_state_checked(&self) -> Result<Vec<u8>, StateError> {
        let payload = state::encode_payload(&self.snapshot()?);
        let header = Header {
            version_tag: self.version_tag.clone(),
            accuracy: self.accuracy_code(),
            firmware_sha1: self.bus.firmware_sha1(),
            drives: self.drive_meta(),
        };
        Ok(state::encode(&header, &payload))
    }

    /// Loads a state saved by `save_state_checked`; see `state.rs` for what is refused. A state loaded with a
    /// different disk in a drive than it was saved with succeeds and leaves a note for `take_state_warnings`.
    pub fn load_state_checked(&mut self, bytes: &[u8]) -> Result<(), StateError> {
        let (header, payload) =
            state::decode(bytes, self.accuracy_code(), &self.bus.firmware_sha1())?;
        let p: StatePayload = state::decode_payload(payload)?;
        self.restore(p)?;
        self.state_warnings = self.media_notes(&header);
        Ok(())
    }

    /// What differs between the disks the state was saved with and the ones in the drives now.
    fn media_notes(&self, header: &Header) -> Vec<String> {
        let now = self.drive_meta();
        let mut notes = Vec::new();
        for (n, (saved, current)) in header.drives.iter().zip(&now).enumerate() {
            let name = ["drive A", "drive B"][n];
            if saved.dirty {
                continue; // the state carries its content
            }
            match (saved.present, current.present) {
                (true, false) => notes.push(format!(
                    "{name} held a disk when the state was saved; no disk is inserted now"
                )),
                (false, true) => notes.push(format!(
                    "{name} was empty when the state was saved; the disk inserted now stays"
                )),
                (true, true) if saved.sha1 != current.sha1 => notes.push(format!(
                    "the disk in {name} is not the one the state was saved with ({} then, {} now)",
                    &state::hex(&saved.sha1)[..8],
                    &state::hex(&current.sha1)[..8]
                )),
                _ => {}
            }
        }
        notes
    }

    /// The number of floppy drives (1 or 2, default 2), before the machine runs; any disks are removed.
    pub fn set_drives(&mut self, n: u8) {
        self.bus.dma.fdc.floppy = crate::drive::Drives::new(n);
    }

    /// Puts a raw disk image behind ACSI target `id` (0-7); a guest write changes this copy only.
    pub fn attach_hard_disk(&mut self, id: u8, image: Vec<u8>) -> Result<(), String> {
        let disk = crate::acsi::MemoryDisk::from_image(image)?;
        self.bus.dma.acsi.attach(id, Box::new(disk))
    }

    pub fn detach_hard_disk(&mut self, id: u8) {
        self.bus.dma.acsi.detach(id);
    }

    /// The image behind target `id`, with the guest's writes in it.
    pub fn hard_disk_image(&self, id: u8) -> Option<&[u8]> {
        self.bus.dma.acsi.disk(id)?.image()
    }

    pub fn hard_disk_modified(&self, id: u8) -> bool {
        self.bus.dma.acsi.disk(id).is_some_and(|d| d.is_modified())
    }

    pub fn mark_hard_disk_saved(&mut self, id: u8) {
        if let Some(d) = self.bus.dma.acsi.disk_mut(id) {
            d.mark_saved();
        }
    }

    /// Puts a ROM cartridge in the slot (`None` takes it out); see [`StBus::set_cartridge`].
    pub fn set_cartridge(&mut self, image: Option<Vec<u8>>) -> Result<(), String> {
        self.bus.set_cartridge(image)
    }

    pub fn cartridge(&self) -> Option<&[u8]> {
        self.bus.cartridge()
    }

    /// Connects a Centronics printer whose output goes to `sink`.
    pub fn attach_printer(&mut self, sink: crate::serial::FileSink) {
        self.bus.attach_printer(sink);
    }

    /// Connects `peer` to the MFP's serial line (the RS-232 port).
    pub fn attach_rs232_peer(&mut self, peer: Box<dyn SerialPeer + Send>) {
        self.bus.attach_rs232_peer(peer);
    }

    /// Connects `peer` to ACIA `port` (0 keyboard, 1 MIDI).
    pub fn attach_serial_peer(&mut self, port: usize, peer: Box<dyn SerialPeer + Send>) {
        let now = self.bus.now;
        let p = &mut self.bus.acias.ports[port];
        p.peer = crate::acia::Peer::Other(peer);
        p.drop_rx();
        p.pull_rx(now);
        self.bus.schedule_acia(port);
    }

    /// Runs `f` on the IKBD at the current time, then lets the keyboard ACIA collect what it queued.
    fn with_ikbd(&mut self, f: impl FnOnce(&mut ikbd::Ikbd, u64)) {
        let now = self.bus.now;
        if let Some(k) = self.bus.acias.ikbd() {
            f(k, now);
        }
        self.bus.acias.ports[0].pull_rx(now);
        self.bus.schedule_acia(0);
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
}

impl Machine for AtariSt {
    fn clock_hz(&self) -> u64 {
        (scheduler::crystal_hz(self.bus.region) / scheduler::CPU_DIVIDER) as u64
    }

    fn frame_hz(&self) -> u64 {
        match (self.bus.monitor, self.bus.region) {
            (st_shifter::Monitor::Mono, _) => 71,
            (_, Region::Pal) => 50,
            (_, Region::Ntsc) => 60,
        }
    }

    /// The power-on frame (the host paces by it; TOS may switch 50/60 Hz later).
    fn frame_rate(&self) -> FrameRate {
        FrameRate {
            num: scheduler::crystal_hz(self.bus.region),
            den: scheduler::CPU_DIVIDER
                * match self.bus.monitor {
                    st_shifter::Monitor::Mono => 224 * 501,
                    _ => scheduler::cycles_per_frame(self.bus.region),
                } as u32,
        }
    }

    fn run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> RunStats {
        machine::Clock::new(self.frame_hz()).run(self, running)
    }

    fn step(&mut self) -> u32 {
        let start = self.bus.now;
        if self.bus.stop().is_some() {
            self.advance(4);
            return 4;
        }
        if self.cpu.run_state() == RunState::Running {
            self.diag.instruction(self.cpu.pc, self.cpu.prefetch()[0]);
            self.bus.pc = self.cpu.pc;
        }
        let clocks = self.cpu.step(&mut self.bus);
        if let Some(e) = self.cpu.take_exception() {
            self.diag.exception(e);
        }
        self.after_cpu(start, clocks)
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
        self.bus.psg.catch_up(self.bus.now);
        self.bus.psg.drain()
    }

    fn render_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
        let (w, h) = self.canvas();
        Some((w, h, self.bus.frame.clone()))
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

    /// The state in the `RCST` envelope; empty when the machine has stopped (`save_state_checked` says why).
    fn save_state(&self) -> Vec<u8> {
        self.save_state_checked().unwrap_or_default()
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.load_state_checked(bytes).map_err(|e| e.to_string())
    }

    /// The reset button: the RESET line resets the CPU and the MFP ([D] MC68901 §2.2.3); RAM kept (spec section 9).
    fn reset(&mut self) {
        mc68000::M68kBus::reset_devices(&mut self.bus);
        let start = self.bus.now;
        let clocks = self.cpu.reset(&mut self.bus);
        self.after_cpu(start, clocks);
    }
}
