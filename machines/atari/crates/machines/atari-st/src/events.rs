//! Machine time and the events that fall due in it, owned by the bus so that an `accurate` machine can let time
//! pass cycle by cycle (spec section 8): the scheduler, the frame's shape and the line being drawn.

use crate::bus::StBus;
use crate::dma::FDC_BYTE_CYCLES;
use crate::scheduler::{self, CLASS_ACIA, CLASS_FDC, CLASS_MFP, CLASS_VIDEO, Event};

impl StBus {
    /// Emulated time in CPU clocks.
    pub fn now(&self) -> u64 {
        self.now
    }

    /// Lines ended so far (each one an HBL request).
    pub fn hbl_count(&self) -> u64 {
        self.hbl_count
    }

    /// Frames ended so far (each one a VBL request).
    pub fn vbl_count(&self) -> u64 {
        self.vbl_count
    }

    /// The CPU clock in Hz.
    pub(crate) fn clock_hz(&self) -> u64 {
        (scheduler::crystal_hz(self.region) / scheduler::CPU_DIVIDER) as u64
    }

    /// Reschedules what a device access during the running instruction (fast mode) or cycle (accurate mode) moved,
    /// from `at`, before any event that fell due after it is processed, so none of them sees a device ahead of its
    /// own time.
    pub(crate) fn flush_dirty(&mut self, at: u64) {
        if std::mem::take(&mut self.fdc_dirty) {
            self.schedule_fdc(at);
        }
        if std::mem::take(&mut self.acia_dirty) {
            self.schedule_acia(0);
            self.schedule_acia(1);
        }
        if std::mem::take(&mut self.mfp_dirty) {
            self.schedule_mfp(at);
        }
    }

    /// Lets `clocks` pass (`fast` mode: a whole instruction) and processes the events due by then, each at its own
    /// time.
    pub(crate) fn advance(&mut self, clocks: u32) {
        // A device write during the instruction happened at its start (`now`).
        self.flush_dirty(self.now);
        self.now += clocks as u64;
        self.run_events(self.now);
    }

    /// Processes every event due at or before `upto`, each at its own time.
    pub(crate) fn run_events(&mut self, upto: u64) {
        while let Some((at, _, event)) = self.sched.pop_due(upto) {
            match event {
                Event::LineEnd => self.line_end(at),
                Event::Vbl => self.raise_vbl(),
                Event::MfpTimeout(n) if n == self.mfp_event => {
                    self.catch_up_mfp(at);
                    self.schedule_mfp(at);
                }
                Event::MfpTimeout(_) => {}
                Event::AciaService { port, generation }
                    if generation == self.acias.ports[port as usize].generation =>
                {
                    self.acias.ports[port as usize].service(at);
                    self.update_acia_irq(at);
                    self.mfp_dirty = false;
                    self.schedule_mfp(at);
                    self.schedule_acia(port as usize);
                }
                Event::AciaService { .. } => {}
                Event::FdcTick(g) if g == self.dma.generation => {
                    self.catch_up_fdc(at);
                    if std::mem::take(&mut self.mfp_dirty) {
                        self.schedule_mfp(at);
                    }
                    self.schedule_fdc(at);
                }
                Event::FdcTick(_) => {}
            }
        }
    }

    // --- accurate mode: time passes with the bus cycles (spec sections 8 and 11)

    /// `accurate`: the cycle that starts now is about to put its address on the bus. Returns the time the device
    /// sees it: the events due by then have been processed and `now` is that time.
    pub(crate) fn begin_cycle(&mut self, wait: u32, write: bool) -> u64 {
        // The cycle waits for its slot; time passes (and events fall due) meanwhile.
        if wait > 0 {
            self.pass(wait);
        }
        let start = self.now;
        let seen = start
            + if write {
                scheduler::WRITE_POINT
            } else {
                scheduler::READ_POINT
            };
        self.run_events(seen);
        // A DMA burst that came due holds the cycle back.
        let held = std::mem::take(&mut self.stall);
        self.now = seen + held;
        start + held
    }

    /// `accurate`: the cycle that started at `start` ends (4 clocks and the wait states `extra` after it began);
    /// what the access moved is rescheduled from the time it was seen, then the events the cycle crossed run.
    pub(crate) fn end_cycle(&mut self, start: u64, extra: u32) {
        self.flush_dirty(self.now);
        self.now = start + 4 + extra as u64;
        self.run_events(self.now);
        self.now += std::mem::take(&mut self.stall);
    }

    /// `accurate`: `clocks` internal clocks with no bus cycle.
    pub(crate) fn pass(&mut self, clocks: u32) {
        self.flush_dirty(self.now);
        self.now += clocks as u64;
        self.run_events(self.now);
        self.now += std::mem::take(&mut self.stall);
    }

    /// The end of a line: the HBL; after the frame's last line the VBL, the picture, and the next frame's shape
    /// from the sync mode then in force.
    fn line_end(&mut self, at: u64) {
        self.hbl_count += 1;
        self.raise_hbl();
        self.line += 1;
        let wrapped = self.line == self.geometry.lines;
        if wrapped {
            self.line = 0;
        }
        self.shifter.line_boundary(self.line as u16);
        if wrapped {
            self.vbl_count += 1;
            self.last_vbl_at = at;
            // `fast` mode raises it with the line-end HBL (its picture and its Hatari I/O trace were made so); `accurate`
            // mode puts it a little into the first line, where the hardware has it.
            if self.accurate {
                self.sched
                    .schedule(at + crate::waits::VBL_DELAY, CLASS_VIDEO, Event::Vbl);
            } else {
                self.raise_vbl();
            }
            self.catch_up_mfp(at);
            self.psg.catch_up(at);
            self.catch_up_fdc(at);
            let cps = self.clock_hz();
            if let Some(k) = self.acias.ikbd() {
                k.tick_clock(at, cps);
            }
            self.render();
            // Both monitors have a frame for every sync mode, so this cannot fail.
            if let Ok(g) = scheduler::geometry(self.shifter.sync_50hz(), self.monitor) {
                self.geometry = g;
            }
            self.shifter.begin_frame();
        }
        self.shifter.set_line(self.line as u16);
        self.shifter.set_line_start(at);
        self.sched.schedule(
            at + self.geometry.clocks_per_line as u64,
            CLASS_VIDEO,
            Event::LineEnd,
        );
    }

    /// One MFP timeout event at the next timer timeout after `at`; any older one is now stale.
    pub(crate) fn schedule_mfp(&mut self, at: u64) {
        self.catch_up_mfp(at);
        self.mfp_event += 1;
        if let Some(ticks) = self.mfp().next_timeout() {
            let clock = self.mfp_clock();
            let when = clock.cycle_for(self.mfp_ticks() + ticks).max(at);
            self.sched
                .schedule(when, CLASS_MFP, Event::MfpTimeout(self.mfp_event));
        }
    }

    /// The controller's next byte time while it works; any older event is now stale.
    pub(crate) fn schedule_fdc(&mut self, at: u64) {
        self.dma.generation += 1;
        if self.dma.busy() {
            let generation = self.dma.generation;
            self.sched
                .schedule(at + FDC_BYTE_CYCLES, CLASS_FDC, Event::FdcTick(generation));
        }
    }

    /// One service event at ACIA `port`'s next due action; any older one is now stale.
    pub(crate) fn schedule_acia(&mut self, port: usize) {
        let p = &mut self.acias.ports[port];
        p.generation += 1;
        if let Some(at) = p.next_due() {
            let generation = p.generation;
            self.sched.schedule(
                at.max(self.now),
                CLASS_ACIA,
                Event::AciaService {
                    port: port as u8,
                    generation,
                },
            );
        }
    }

    pub(crate) fn render(&mut self) {
        let mut frame = std::mem::take(&mut self.frame);
        let first = scheduler::first_display_line(self.geometry.lines);
        if self.accurate && self.borders {
            self.shifter.render_overscan(
                self.monitor,
                crate::waits::PIXEL_START,
                &|a| self.peek(a),
                &mut frame,
            );
        } else if self.accurate {
            self.shifter.render_exact(
                self.monitor,
                first,
                crate::waits::PIXEL_START,
                &|a| self.peek(a),
                &mut frame,
            );
        } else {
            self.shifter
                .render_raster(self.monitor, first, &|a| self.peek(a), &mut frame);
        }
        self.frame = frame;
    }
}
