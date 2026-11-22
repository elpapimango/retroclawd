//! ANTIC display-list, playfield DMA and scan counters.
//! Source: Altirra Hardware Reference (2026-01-02), chapter 4 and section 14.6.
//! <https://www.virtualdub.org/downloads/Altirra%20Hardware%20Reference%20Manual.pdf>
//! Models normal playfield/PM DMA, displaced refresh, scrolling and interrupt
//! deadlines. Abnormal DMA rings and dynamic start/stop clocks remain incomplete.
//! It does not claim Acid800 conformance.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const WIDTH: usize = 384;
pub const HEIGHT: usize = 240;
const ROWS: [u8; 16] = [1, 1, 8, 10, 8, 16, 8, 16, 8, 4, 4, 2, 1, 2, 1, 1];
const BYTES: [u8; 16] = [0, 0, 40, 40, 40, 40, 20, 20, 10, 10, 20, 20, 20, 40, 40, 40];

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AnticTimingState {
    first_line: bool,
    refresh_pending: bool,
    vscroll: bool,
    ending_scroll: bool,
    end_row: u8,
    vscroll_latch: u8,
    dli: bool,
    nmi_enable: u8,
    nmi_enable_late: u8,
    dma_control: [u8; 2],
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AnticDmaState {
    start: i16,
    stop: i16,
    initialized: bool,
    pm_start: Option<u8>,
    clock: u8,
    cursor: u8,
    display_origin: Option<u8>,
}

impl Default for AnticDmaState {
    fn default() -> Self {
        Self {
            start: -1,
            stop: -1,
            initialized: false,
            pm_start: None,
            clock: 0,
            cursor: 0,
            display_origin: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Antic {
    registers: [u8; 16],
    lines: u16,
    pub x: u8,
    pub y: u16,
    pub cycles: u64,
    list: u16,
    scan: u16,
    address_low: u8,
    instruction: u8,
    row: u8,
    remaining: u8,
    wait_vblank: bool,
    names: Vec<u8>,
    data: Vec<u8>,
    line: Vec<u8>,
    status: u8,
    nmi: bool,
    wsync_until: u64,
    wsync_delay: u8,
    #[serde(skip)]
    timing: AnticTimingState,
    #[serde(skip)]
    pm_transfer: Option<(u8, u8)>,
    #[serde(skip)]
    dma: AnticDmaState,
}

impl Antic {
    pub fn new(pal: bool) -> Self {
        Self {
            registers: [0; 16],
            lines: if pal { 312 } else { 262 },
            x: 0,
            y: 0,
            cycles: 0,
            list: 0,
            scan: 0,
            address_low: 0,
            instruction: 0,
            row: 0,
            remaining: 0,
            wait_vblank: false,
            names: vec![0; 48],
            data: vec![0; 48],
            line: vec![0; WIDTH],
            status: 0x1f,
            nmi: false,
            wsync_until: 0,
            wsync_delay: 0,
            timing: AnticTimingState::default(),
            pm_transfer: None,
            dma: AnticDmaState::default(),
        }
    }

    pub fn pal(&self) -> bool {
        self.lines == 312
    }
    pub fn register(&self, index: usize) -> u8 {
        self.registers[index & 15]
    }
    pub fn save_timing(&self) -> AnticTimingState {
        self.timing.clone()
    }
    pub fn save_dma(&self) -> AnticDmaState {
        self.dma.clone()
    }
    pub fn restore_dma(&mut self, state: AnticDmaState) -> Result<(), String> {
        if !(-1..=33).contains(&state.start)
            || !(-1..=111).contains(&state.stop)
            || (state.stop >= 0 && state.start < 0)
            || state.pm_start.is_some_and(|clock| clock > 7)
            || state.cursor >= 63
            || state.display_origin.is_some_and(|cursor| cursor >= 63)
        {
            return Err("invalid ANTIC playfield deadline state".to_string());
        }
        self.dma = state;
        Ok(())
    }
    pub fn restore_legacy_dma(&mut self) {
        self.dma = AnticDmaState {
            initialized: true,
            ..AnticDmaState::default()
        };
        if let Some((start, stop)) = self.dma_deadlines()
            && i16::from(self.x) > start
        {
            self.dma.start = start + 2;
            if i16::from(self.x) > stop {
                self.dma.stop = stop;
            }
        }
        let period = if self.mode() >= 2 {
            80 / BYTES[self.mode()]
        } else {
            2
        };
        if self.dma.start >= 0 {
            for clock in 0..self.x {
                if i16::from(clock) == self.dma.start - 2 {
                    self.dma.clock |= 1;
                }
                if i16::from(clock) == self.dma.stop {
                    self.dma.clock &= !1;
                }
                self.dma.clock = (self.dma.clock << 1) | ((self.dma.clock >> (period - 1)) & 1);
            }
            let offset = if self.mode() >= 8 { 2 } else { 0 };
            self.dma.cursor = ((i16::from(self.x) - self.dma.start - offset).max(0)
                + i16::from(period)
                - 1) as u8
                / period;
            self.dma.display_origin = Some(0);
        }
    }
    pub fn restore_timing(&mut self, state: AnticTimingState) -> Result<(), String> {
        if state.end_row > 15
            || state.vscroll_latch > 15
            || (state.nmi_enable | state.nmi_enable_late) & !0xc0 != 0
        {
            return Err("invalid ANTIC timing extension".to_string());
        }
        self.timing = state;
        self.pm_transfer = None;
        Ok(())
    }
    pub fn restore_legacy_timing(&mut self) {
        self.timing = AnticTimingState {
            first_line: self.row == 0,
            end_row: self.row.wrapping_add(self.remaining.saturating_sub(1)) & 15,
            dma_control: [self.registers[0]; 2],
            ..AnticTimingState::default()
        };
        self.pm_transfer = None;
    }
    pub fn take_pm_transfer(&mut self) -> Option<(u8, u8)> {
        self.pm_transfer.take()
    }
    pub fn read(&self, offset: u8) -> u8 {
        match offset & 15 {
            11 => {
                let line = if self.x >= 111 { self.y + 1 } else { self.y };
                if line == self.lines && self.x > 111 {
                    0
                } else {
                    (line / 2) as u8
                }
            }
            12 | 13 => 0xff,
            15 => self.status,
            _ => 0xff,
        }
    }

    pub fn write(&mut self, offset: u8, value: u8) {
        let index = usize::from(offset & 15);
        match index {
            2 => self.list = (self.list & 0xff00) | u16::from(value),
            3 => self.list = (self.list & 0xff) | (u16::from(value) << 8),
            10 => {
                let base = self.cycles - u64::from(self.x);
                let until = base + 105 + if self.x >= 104 { 114 } else { 0 };
                // RDY reacts one clock after the write. A consecutive RMW
                // write cannot restart the same hold (AHR 4.9, pp.77-78).
                self.wsync_delay = if until == self.wsync_until && self.cycles < until {
                    0
                } else {
                    2
                };
                self.wsync_until = until;
            }
            15 => {
                self.status = 0x1f;
                // The status latch wins over a coincident bus write, including
                // the write phase of the clock in which the new value is readable.
                if self.x == 7 {
                    if self.y == 248 {
                        self.status = 0x5f;
                    } else if self.timing.dli {
                        self.status = 0x9f;
                    }
                }
            }
            _ => {}
        }
        self.registers[index] = value;
    }

    pub fn reset_nmi(&mut self) {
        self.registers[14] = 0;
        self.status = 0x1f;
        self.nmi = false;
    }

    /// The 400/800 Reset button reaches ANTIC's RNMI, not the CPU RESET pin.
    pub fn reset_button(&mut self) {
        self.status = 0x3f;
        self.nmi = true;
    }

    pub fn take_nmi(&mut self) -> bool {
        std::mem::take(&mut self.nmi)
    }
    pub fn line(&self) -> &[u8] {
        &self.line
    }
    pub fn hires(&self) -> bool {
        matches!(self.mode(), 2 | 3 | 15)
    }
    pub fn video_active(&self) -> bool {
        // AHR 6.2: a hires IR overrides vertical blank while PF DMA is on.
        self.active() || (self.hires() && self.registers[0] & 3 != 0)
    }

    fn active(&self) -> bool {
        (8..248).contains(&self.y)
    }
    fn dli(&self) -> bool {
        self.active()
            && self.instruction & 0x80 != 0
            && (self.wait_vblank
                || self.row
                    == if self.timing.ending_scroll {
                        self.registers[5] & 15
                    } else {
                        self.timing.end_row
                    })
    }
    fn cycle_status(&self) -> u8 {
        // The latch updates at the end of cycle 6. A write to NMIRES
        // on that cycle loses to the update (AHR 4.8, "Reading interrupt status").
        if self.x == 6 && self.y == 248 {
            (self.status & 0x20) | 0x5f
        } else if self.x == 6 && self.timing.dli {
            (self.status & 0x20) | 0x9f
        } else {
            self.status
        }
    }
    fn mode(&self) -> usize {
        usize::from(self.instruction & 15)
    }
    fn count(&self) -> u8 {
        match self.registers[0] & 3 {
            1 => BYTES[self.mode()] * 4 / 5,
            2 => BYTES[self.mode()],
            3 => BYTES[self.mode()] * 6 / 5,
            _ => 0,
        }
    }
    fn fetch_count(&self) -> u8 {
        let width = self.registers[0] & 3;
        let width = if self.instruction & 0x10 != 0 && width != 0 {
            (width + 1).min(3)
        } else {
            width
        };
        match width {
            1 => BYTES[self.mode()] * 4 / 5,
            2 => BYTES[self.mode()],
            3 => BYTES[self.mode()] * 6 / 5,
            _ => 0,
        }
    }
    fn dma_deadlines(&self) -> Option<(i16, i16)> {
        let width = self.registers[0] & 3;
        if width == 0 {
            return None;
        }
        let scroll = self.instruction & 0x10 != 0;
        let width = if scroll { (width + 1).min(3) } else { width };
        let delay = if scroll {
            i16::from((self.registers[4] & 15) / 2)
        } else {
            0
        };
        Some((
            32 - i16::from(width) * 8 + delay,
            80 + i16::from(width) * 8 + delay,
        ))
    }
    fn clock_dma_deadlines(&mut self) {
        // AHR 4.11: width changes select comparators, not a new fetch grid.
        // Once a comparator fires, later writes cannot move that edge.
        if self.x == 0 {
            self.dma = AnticDmaState {
                initialized: true,
                clock: self.dma.clock,
                ..AnticDmaState::default()
            };
        }
        if !self.active() || self.wait_vblank || self.mode() < 2 {
            self.dma.clock = 0;
        }
        if self.x == 7 {
            // Altirra 4.40 ANTIC timing cross-check: line-buffer pointers
            // reset with the high display-list address/NMIST clock.
            self.dma.cursor = 0;
            self.dma.display_origin = None;
        }
        if let Some((start, stop)) = self.dma_deadlines() {
            if i16::from(self.x) == start {
                self.dma.clock |= 1;
                if self.dma.start < 0 {
                    self.dma.start = start + 2;
                }
            }
            if i16::from(self.x) == stop && self.dma.start >= 0 {
                self.dma.clock &= !1;
                if self.dma.stop < 0 {
                    self.dma.stop = stop;
                }
            }
        }
    }
    fn fetch_slot(&self) -> Option<(usize, bool)> {
        if !self.active() || self.wait_vblank || self.mode() < 2 {
            return None;
        }
        let count = self.fetch_count();
        if self.dma.initialized {
            if self.mode() >= 8 {
                return (self.dma.clock & 0x10 != 0)
                    .then_some((usize::from(self.dma.cursor), false));
            }
            if self.dma.start < 0 {
                return None;
            }
            let step = i16::from(80 / BYTES[self.mode()]);
            let start = self.dma.start + if self.mode() <= 7 { 0 } else { 2 };
            let end = if self.dma.stop < 0 {
                113
            } else {
                self.dma.stop + if self.mode() <= 7 { 0 } else { 2 }
            };
            let position = i16::from(self.x) - start;
            if position >= 0 && position % step == 0 && i16::from(self.x) <= end {
                let index = usize::try_from(position / step).ok()?;
                if index < 48 && (self.timing.first_line || self.mode() >= 8) {
                    return Some((index, false));
                }
            }
            if self.mode() <= 7 {
                let position = position - 3;
                if position >= 0 && position % step == 0 && i16::from(self.x) <= end + 3 {
                    let index = usize::try_from(position / step).ok()?;
                    if index < 48 {
                        return Some((index, true));
                    }
                }
            }
            return None;
        }
        if count == 0 {
            return None;
        }
        let step = 80 / BYTES[self.mode()];
        // AHR 4.14: normal names at 18, bitmap data at 20; font +3.
        let start = if self.mode() <= 7 { 18 } else { 20 }
            + (40 - i16::from(count) * i16::from(step) / 2)
            + if self.instruction & 0x10 != 0 {
                i16::from((self.registers[4] & 15) / 2)
            } else {
                0
            };
        let position = i16::from(self.x) - start;
        if position >= 0 && position % i16::from(step) == 0 {
            let i = position / i16::from(step);
            if i < i16::from(count) && self.timing.first_line {
                return Some((i as usize, false));
            }
        }
        if self.mode() <= 7 {
            let position = position - 3;
            if position >= 0 && position % i16::from(step) == 0 {
                let i = position / i16::from(step);
                if i < i16::from(count) {
                    return Some((i as usize, true));
                }
            }
        }
        None
    }

    pub fn cpu_read_blocked(&self) -> bool {
        (self.wsync_delay == 0 && self.cycles < self.wsync_until) || self.cpu_dma_blocked()
    }

    /// DMA uses HALT for bus ownership, unlike WSYNC's read-only RDY hold.
    /// Altirra Hardware Reference, chapters 2 and 4.14.
    pub fn cpu_dma_blocked(&self) -> bool {
        self.refresh_slot() || self.timing.refresh_pending || self.other_dma()
    }
    fn refresh_slot(&self) -> bool {
        self.x >= 25 && self.x <= 57 && (self.x - 25).is_multiple_of(4)
    }
    fn pm_slot(&self) -> Option<u8> {
        let control = self.timing.dma_control[0];
        if !self.active() {
            return None;
        }
        match self.x {
            0 if control & 0x0c != 0 => Some(4),
            2..=5 if control & 8 != 0 => Some(self.x - 2),
            _ => None,
        }
    }
    fn other_dma(&self) -> bool {
        self.pm_slot().is_some()
            || (self.active()
                && !self.wait_vblank
                && self.registers[0] & 0x20 != 0
                && ((self.x == 1 && self.remaining == 0)
                    || ((self.x == 6 || self.x == 7)
                        && (self.mode() == 1
                            || (self.timing.first_line
                                && self.mode() >= 2
                                && self.instruction & 0x40 != 0)))))
            || (self.x > 0
                && self.x <= 105
                && self.registers[0] & 3 != 0
                && self
                    .fetch_slot()
                    .is_some_and(|(_, font)| font || self.timing.first_line))
    }

    fn list_byte(&mut self, read: &mut impl FnMut(u16) -> u8) -> u8 {
        let value = read(self.list);
        self.list = (self.list & 0xfc00) | (self.list.wrapping_add(1) & 0x3ff);
        value
    }

    fn char_address(&self, name: u8) -> u16 {
        let mode = self.mode();
        let mut row = self.row;
        if mode == 5 || mode == 7 {
            row /= 2;
        }
        row &= 7;
        if self.registers[1] & 4 != 0 {
            row ^= 7;
        }
        let mask = if mode <= 5 { 0x7f } else { 0x3f };
        let base = u16::from(self.registers[9] & if mode <= 5 { 0xfc } else { 0xfe }) << 8;
        base | (u16::from(name & mask) << 3) | u16::from(row)
    }

    /// Advance one system clock; return true after producing one scan line.
    pub fn tick(&mut self, mut read: impl FnMut(u16) -> u8) -> bool {
        self.tick_with_bus(&mut read, 0xff)
    }

    pub fn tick_with_bus(&mut self, mut read: impl FnMut(u16) -> u8, bus_value: u8) -> bool {
        self.clock_dma_deadlines();
        if self.x < 8 && self.other_dma() && self.dma.pm_start.is_none() {
            self.dma.pm_start = Some(self.x);
        }
        let mut data_bus = bus_value;
        let mut read = |address| {
            let value = read(address);
            data_bus = value;
            value
        };
        self.pm_transfer = None;
        self.timing.refresh_pending |= self.refresh_slot();
        if !self.other_dma() {
            self.timing.refresh_pending = false;
        }
        if let Some(player) = self.pm_slot() {
            let one_line = self.timing.dma_control[0] & 0x10 != 0;
            let shift = if one_line { 8 } else { 7 };
            let base = u16::from(self.registers[7] & if one_line { 0xf8 } else { 0xfc }) << 8;
            let section = if player == 4 {
                3
            } else {
                u16::from(player) + 4
            };
            let offset = if one_line {
                self.y & 255
            } else {
                (self.y / 2) & 127
            };
            self.pm_transfer = Some((player, read(base | (section << shift) | offset)));
        }
        if self.x == 0 {
            self.timing.vscroll_latch = self.registers[5] & 15;
            if self.active() && self.remaining == 0 && !self.wait_vblank {
                self.timing.first_line = true;
            }
        }
        if self.x == 0 && self.y == 8 {
            self.wait_vblank = false;
            self.remaining = 0;
            self.instruction &= !0x40;
        }
        if self.x == 1 && self.active() && !self.wait_vblank && self.remaining == 0 {
            self.row = 0;
            if self.registers[0] & 0x20 != 0 {
                self.instruction = self.list_byte(&mut read);
            }
            let scroll = self.mode() >= 2 && self.instruction & 0x20 != 0;
            self.timing.ending_scroll = self.timing.vscroll && !scroll;
            self.row = if scroll && !self.timing.vscroll {
                self.timing.vscroll_latch
            } else {
                0
            };
            self.timing.end_row = if self.timing.ending_scroll {
                self.registers[5] & 15
            } else if self.mode() == 0 {
                (self.instruction >> 4) & 7
            } else {
                ROWS[self.mode()] - 1
            };
            self.remaining = (self.timing.end_row.wrapping_sub(self.row) & 15) + 1;
            self.timing.vscroll = scroll;
            self.timing.first_line = true;
        }
        if self.active()
            && !self.wait_vblank
            && self.registers[0] & 0x20 != 0
            && (self.mode() == 1
                || (self.timing.first_line && self.mode() >= 2 && self.instruction & 0x40 != 0))
        {
            if self.x == 6 {
                self.address_low = self.list_byte(&mut read);
            }
            if self.x == 7 {
                let high = self.list_byte(&mut read);
                let address = u16::from_le_bytes([self.address_low, high]);
                if self.mode() == 1 {
                    self.list = address;
                    self.wait_vblank = self.instruction & 0x40 != 0;
                } else {
                    self.scan = address;
                }
            }
        }
        if let Some((i, font)) = self.fetch_slot() {
            if font {
                let name = self.names.get(i).copied().unwrap_or(0xff);
                let mut data = if self.x <= 105 && self.registers[0] & 3 != 0 {
                    read(self.char_address(name))
                } else {
                    bus_value
                };
                if (self.mode() == 3 && name & 0x60 == 0x60 && self.row < 2)
                    || (self.mode() <= 3 && name & 0x60 != 0x60 && (8..10).contains(&self.row))
                {
                    data = 0;
                }
                if self.mode() <= 3 && name & 0x80 != 0 {
                    if self.registers[1] & 1 != 0 {
                        data = 0;
                    }
                    if self.registers[1] & 2 != 0 {
                        data ^= 0xff;
                    }
                }
                if let Some(slot) = self.data.get_mut(i) {
                    *slot = data;
                }
            } else {
                if self.timing.first_line {
                    if self.registers[0] & 3 != 0 {
                        let byte = if self.x > 0 && self.x <= 105 {
                            read(self.scan)
                        } else {
                            bus_value
                        };
                        if let Some(slot) = self.names.get_mut(i) {
                            *slot = byte;
                        }
                    }
                    self.scan = (self.scan & 0xf000) | (self.scan.wrapping_add(1) & 0x0fff);
                }
                if let Some(slot) = self.data.get_mut(i) {
                    *slot = self.names[i];
                }
                if self.mode() >= 8 && self.dma.initialized {
                    if self.dma.display_origin.is_none()
                        && self.dma.start >= 0
                        && i16::from(self.x) >= self.dma.start + 2
                    {
                        self.dma.display_origin = Some(self.dma.cursor);
                    }
                    self.dma.cursor = (self.dma.cursor + 1) % 63;
                }
            }
        }
        // AHR 6.5: GTIA synchronizes its receive slots to the first HALT
        // in horizontal blank, even when ANTIC has no P/M DMA enabled.
        if self.pm_transfer.is_none()
            && self.active()
            && let Some(start) = self.dma.pm_start
        {
            let offset = self.x.saturating_sub(start);
            let player = match offset {
                0 => Some(4),
                2..=5 => Some(offset - 2),
                _ => None,
            };
            if let Some(player) = player {
                self.pm_transfer = Some((player, data_bus));
            }
        }
        if self.x == 5 {
            self.timing.dli = self.dli();
        }
        if self.x == 6 {
            self.timing.nmi_enable = self.registers[14] & 0xc0;
        }
        if self.x == 7 {
            self.timing.nmi_enable_late = self.registers[14] & !self.timing.nmi_enable & 0xc0;
        }
        let dli = self.timing.dli;
        self.status = self.cycle_status();
        // Altirra Hardware Reference 4.8, "DLI timing": NMI at cycle 8.
        // https://www.virtualdub.org/downloads/Altirra%20Hardware%20Reference%20Manual.pdf
        let enable = match self.x {
            8 => self.timing.nmi_enable,
            9 => self.timing.nmi_enable_late,
            _ => 0,
        };
        if (self.y == 248 && enable & 0x40 != 0) || (dli && enable & 0x80 != 0) {
            self.nmi = true;
        }
        let line_done = self.x == 113;
        if self.x == 108 && self.timing.ending_scroll {
            self.timing.end_row = self.registers[5] & 15;
        }
        if line_done {
            self.draw();
            if self.active() && self.remaining > 0 && !self.wait_vblank {
                self.remaining = self.timing.end_row.wrapping_sub(self.row) & 15;
                if self.remaining > 0 {
                    self.row = self.row.wrapping_add(1) & 15;
                }
            }
            self.timing.first_line = false;
        }
        if self.active() && !self.wait_vblank && self.mode() >= 2 {
            // AHR 4.12: eight physical bits, with a 2/4/8-cycle return tap.
            let period = 80 / BYTES[self.mode()];
            self.dma.clock = (self.dma.clock << 1) | ((self.dma.clock >> (period - 1)) & 1);
        } else {
            self.dma.clock = 0;
        }
        self.timing.dma_control = [self.timing.dma_control[1], self.registers[0]];
        self.wsync_delay = self.wsync_delay.saturating_sub(1);
        self.cycles += 1;
        self.x += 1;
        if self.x == 114 {
            self.x = 0;
            self.y += 1;
            if self.y == self.lines {
                self.y = 0;
            }
        }
        line_done
    }

    /// XL/XE reset pin, unlike the 400/800's separate RNMI button.
    /// AHR 4.1: preserve raster, WSYNC, pointers and other register latches.
    pub fn reset(&mut self) {
        self.registers[0] = 0;
        self.registers[14] = 0;
        self.dma.clock = 0;
        self.timing.dma_control = [0; 2];
        self.timing.nmi_enable = 0;
        self.timing.nmi_enable_late = 0;
    }

    fn draw(&mut self) {
        for x in 0..WIDTH {
            self.line[x] = self.pixel_at(x);
        }
    }

    /// Current buffered playfield at a high-resolution screen pixel.
    pub fn pixel_at(&self, x: usize) -> u8 {
        let mode = self.mode();
        let count = usize::from(self.count());
        if !self.video_active() || self.wait_vblank || mode < 2 || count == 0 {
            return 0;
        }
        let pixels_per_byte = match mode {
            6..=9 => 16 + usize::from(mode >= 8) * 16,
            10..=12 => 16,
            _ => 8,
        };
        let width = count * pixels_per_byte;
        let start = (WIDTH - width) / 2;
        if x < start || x >= start + width {
            return 0;
        }
        if !self.active() {
            return 5;
        }
        let fetch_width = usize::from(self.fetch_count()) * pixels_per_byte;
        let origin = (WIDTH - fetch_width) as isize / 2
            + if self.instruction & 0x10 != 0 {
                isize::from(self.registers[4] & 15) * 2
            } else {
                0
            };
        let source = x as isize - origin;
        if source < 0 || source >= fetch_width as isize {
            return 0;
        }
        let mut i = source as usize / pixels_per_byte;
        if mode >= 8
            && let Some(origin) = self.dma.display_origin
        {
            i = (i + usize::from(origin)) % 63;
        }
        let x = source as usize % pixels_per_byte;
        let byte = self.data.get(i).copied().unwrap_or(0xff);
        match mode {
            2 | 3 | 15 => 5 + ((byte >> (7 - x)) & 1),
            4 | 5 | 8 | 10 | 13 | 14 => {
                let pair = (byte >> (6 - (x * 4 / pixels_per_byte) * 2)) & 3;
                if pair == 3 && mode <= 5 && self.names[i] & 0x80 != 0 {
                    4
                } else {
                    pair
                }
            }
            6 | 7 => {
                if byte & (0x80 >> (x / 2)) != 0 {
                    (self.names[i] >> 6) + 1
                } else {
                    0
                }
            }
            _ => u8::from(byte & (0x80 >> (x * 8 / pixels_per_byte)) != 0),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if ![262, 312].contains(&self.lines)
            || self.x >= 114
            || self.y >= self.lines
            || self.row > 15
            || self.remaining > 16
            || self.names.len() != 48
            || self.data.len() != 48
            || self.line.len() != WIDTH
            || self.line.iter().any(|&x| x > 6)
            || self.wsync_delay > 2
            || self.cycles % (114 * u64::from(self.lines))
                != u64::from(self.y) * 114 + u64::from(self.x)
        {
            return Err("invalid ANTIC state".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn width_writes_preserve_start_and_stop_comparator_history() {
        for mode in [2, 14] {
            for (initial, clock, value, expected) in [
                (0x22, 16, 0x21, 32),
                (0x22, 17, 0x21, 36),
                (0x21, 88, 0x22, 36),
                (0x21, 89, 0x22, 32),
            ] {
                let mut a = Antic::new(true);
                a.y = 9;
                a.cycles = 9 * 114;
                a.instruction = mode;
                a.remaining = 8;
                a.timing.first_line = true;
                a.registers[0] = initial;
                a.scan = 0x3000;
                for x in 0..114 {
                    if x == clock {
                        a.write(0, value);
                    }
                    a.tick(|_| 0);
                }
                assert_eq!(
                    a.scan,
                    0x3000 + expected,
                    "mode={mode}, write={clock}, initial={initial:02x}"
                );
            }
        }
    }

    #[test]
    fn disabled_middle_dma_preserves_buffer_but_advances_scan_address() {
        let mut a = Antic::new(true);
        a.y = 9;
        a.cycles = 9 * 114;
        a.instruction = 8;
        a.remaining = 8;
        a.timing.first_line = true;
        a.registers[0] = 0x22;
        a.scan = 0x3000;
        a.names.fill(0x55);
        for x in 0..114 {
            if x == 38 {
                a.write(0, 0x20);
            }
            if x == 70 {
                a.write(0, 0x22);
            }
            a.tick(|_| 0xaa);
        }
        assert_eq!(a.scan, 0x300a);
        assert_eq!(
            &a.names[..10],
            &[0xaa, 0xaa, 0xaa, 0x55, 0x55, 0x55, 0x55, 0xaa, 0xaa, 0xaa]
        );
    }

    #[test]
    fn phantom_pm_slots_follow_first_halt_without_stealing_cpu_cycles() {
        let mut a = Antic::new(true);
        a.y = 9;
        a.cycles = 9 * 114;
        a.registers[0] = 0x20;
        let mut transfers = Vec::new();
        for x in 0..8 {
            if x >= 2 {
                assert!(!a.cpu_dma_blocked());
            }
            a.tick_with_bus(|_| 0x0f, 0xa0 | x);
            if let Some(transfer) = a.take_pm_transfer() {
                transfers.push((x, transfer));
            }
        }
        assert_eq!(
            transfers,
            vec![
                (1, (4, 0x0f)),
                (3, (0, 0xa3)),
                (4, (1, 0xa4)),
                (5, (2, 0xa5)),
                (6, (3, 0xa6))
            ]
        );
    }

    #[test]
    fn hires_ir_opens_vertical_border_only_with_playfield_enabled() {
        let mut a = Antic::new(true);
        a.y = 248;
        a.instruction = 15;
        assert!(!a.video_active());
        a.registers[0] = 0x22;
        assert!(a.video_active());
        assert_eq!(a.pixel_at(32), 5);
        a.instruction = 14;
        assert!(!a.video_active());
        assert_eq!(a.pixel_at(32), 0);
    }

    #[test]
    fn shifted_stop_leaves_dma_ring_running_across_horizontal_blank() {
        let mut a = Antic::new(true);
        a.y = 9;
        a.cycles = 9 * 114;
        a.instruction = 0x1e;
        a.remaining = 1;
        a.timing.first_line = true;
        a.registers[0] = 1;
        a.scan = 0x3000;
        let mut next_first_fetch = None;
        for clock in 0..228 {
            if clock == 78 {
                a.write(4, 2);
            }
            if clock == 108 {
                a.write(4, 0);
            }
            a.tick_with_bus(
                |address| {
                    if clock == 134 {
                        next_first_fetch = Some(address);
                    }
                    if address == 0x304b { 0xff } else { 0 }
                },
                0,
            );
        }
        // AHR 4.12 and Acid800's published HSCROL waveform: seven
        // additional clocks on the first line, ten before the next fetch.
        assert_eq!(next_first_fetch, Some(0x3039));
        assert_eq!(a.line()[176], 3);
        assert_eq!(a.scan, 0x3061);
    }

    #[test]
    fn descender_blanking_does_not_erase_mode_two_or_extended_rows() {
        for (mode, row, name, expected) in [
            (2, 1, 0x63, 6),
            (3, 1, 0x63, 5),
            (2, 8, 1, 5),
            (3, 10, 1, 6),
        ] {
            let mut a = Antic::new(true);
            a.y = 8;
            a.x = 21;
            a.row = row;
            a.instruction = mode;
            a.registers[0] = 0x22;
            a.names[0] = name;
            a.tick(|_| 0xf0);
            assert_eq!(a.pixel_at(32), expected);
        }
    }

    #[test]
    fn pm_dma_fetches_each_resolution_and_forces_missiles_with_players() {
        for (control, expected) in [
            (
                0x18,
                vec![
                    (0, 0x3308),
                    (2, 0x3408),
                    (3, 0x3508),
                    (4, 0x3608),
                    (5, 0x3708),
                ],
            ),
            (
                0x08,
                vec![
                    (0, 0x3584),
                    (2, 0x3604),
                    (3, 0x3684),
                    (4, 0x3704),
                    (5, 0x3784),
                ],
            ),
        ] {
            let mut a = Antic::new(true);
            a.registers[0] = control;
            a.registers[7] = 0x37;
            let mut accesses = Vec::new();
            for _ in 0..114 * 8 + 6 {
                let (x, y) = (a.x, a.y);
                a.tick(|address| {
                    if y == 8 {
                        accesses.push((x, address));
                    }
                    address as u8
                });
                if y == 8 && [0, 2, 3, 4, 5].contains(&x) {
                    assert!(a.take_pm_transfer().is_some());
                }
            }
            assert_eq!(accesses, expected);
        }
    }

    #[test]
    fn horizontal_scroll_expands_fetches_and_clips_to_original_width() {
        let mut a = Antic::new(true);
        a.y = 8;
        a.instruction = 0x1f;
        a.registers[0] = 0x22;
        a.data.fill(0x80);
        assert_eq!(a.fetch_count(), 48);
        assert_eq!(a.pixel_at(31), 0);
        assert_eq!(a.pixel_at(32), 6);
        a.registers[4] = 1;
        assert_eq!(a.pixel_at(32), 5);
        assert_eq!(a.pixel_at(34), 6);
        assert_eq!(a.pixel_at(352), 0);
    }

    #[test]
    fn display_list_address_and_character_fetches_use_documented_slots() {
        let mut a = Antic::new(true);
        a.registers[0] = 0x22;
        a.list = 0x2000;
        let mut ram = vec![0; 65536];
        ram[0x2000..0x2003].copy_from_slice(&[0x42, 0, 0x30]);
        let mut accesses = Vec::new();
        for _ in 0..114 * 9 {
            let (x, y) = (a.x, a.y);
            a.tick(|address| {
                if y == 8 {
                    accesses.push((x, address));
                }
                ram[usize::from(address)]
            });
        }
        assert_eq!(
            &accesses[..4],
            &[(1, 0x2000), (6, 0x2001), (7, 0x2002), (18, 0x3000)]
        );
        assert!(accesses.iter().any(|&(x, _)| x == 21));
    }

    #[test]
    fn refresh_waits_for_playfield_and_virtual_dma_does_not_halt_cpu() {
        let mut a = Antic::new(true);
        a.y = 8;
        a.cycles = 8 * 114;
        a.registers[0] = 0x23;
        a.list = 0x2000;
        for _ in 0..106 {
            a.tick(|address| if address == 0x2000 { 2 } else { 0 });
        }
        assert!(a.cpu_dma_blocked(), "last refresh is displaced to 106");
        a.tick(|_| 0);
        assert!(
            !a.cpu_dma_blocked(),
            "virtual character DMA cannot halt at 107"
        );
    }

    #[test]
    fn dma_between_rmw_writes_cannot_rearm_wsync_delay() {
        let mut a = Antic::new(true);
        a.x = 24;
        a.cycles = 24;
        a.write(10, 0xff);
        a.tick(|_| 0);
        assert!(a.cpu_dma_blocked());
        a.tick(|_| 0);
        a.write(10, 0);
        a.tick(|_| 0);
        assert!(a.cpu_read_blocked());
    }

    #[test]
    fn vertical_scroll_region_continues_across_vertical_blank() {
        let mut a = Antic::new(true);
        a.y = 8;
        a.timing.vscroll = true;
        a.registers[0] = 0x22;
        a.registers[5] = 12;
        a.tick(|_| 0xa2);
        a.tick(|_| 0xa2);
        assert_eq!(a.row, 0);
        assert_eq!(a.remaining, 8);
    }

    #[test]
    fn vertical_scroll_shortens_only_entry_and_exit_lines() {
        let mut a = Antic::new(true);
        a.registers[0] = 0x22;
        a.registers[5] = 3;
        a.list = 0x2000;
        let mut starts = Vec::new();
        for _ in 0..114 * 26 {
            let y = a.y;
            a.tick(|address| {
                if (0x2000..0x2004).contains(&address) {
                    starts.push((address, y));
                    [0x22, 0x22, 0x02, 0x41][usize::from(address - 0x2000)]
                } else {
                    0
                }
            });
        }
        assert_eq!(
            &starts[..4],
            &[(0x2000, 8), (0x2001, 13), (0x2002, 21), (0x2003, 25)]
        );
    }

    #[test]
    fn last_clock_nmi_enable_delays_only_a_newly_enabled_interrupt() {
        for (write_clock, initial, value, expected) in [
            (6, 0, 0x40, Some(8)),
            (7, 0, 0x40, Some(9)),
            (8, 0, 0x40, None),
            (6, 0x40, 0, None),
            (7, 0x40, 0, Some(8)),
        ] {
            let mut a = Antic::new(true);
            a.y = 248;
            a.registers[14] = initial;
            let mut pulse = None;
            for x in 0..11 {
                if x == write_clock {
                    a.write(14, value);
                }
                a.tick(|_| 0);
                if a.take_nmi() {
                    pulse = Some(x);
                }
            }
            assert_eq!(pulse, expected);
        }
    }

    #[test]
    fn nmist_sources_replace_each_other_and_nmires_loses_to_cycle_six() {
        let mut a = Antic::new(true);
        a.y = 39;
        a.cycles = 39 * 114;
        a.remaining = 1;
        a.instruction = 0x80;
        a.status = 0x5f;
        for _ in 0..6 {
            a.tick(|_| 0);
        }
        assert_eq!(a.read(15), 0x5f);
        a.write(15, 0);
        a.tick(|_| 0);
        assert_eq!(a.read(15), 0x9f);
        a.write(15, 0);
        assert_eq!(a.read(15), 0x9f);
        a.tick(|_| 0);
        a.write(15, 0);
        assert_eq!(a.read(15), 0x1f);
        a.y = 248;
        a.x = 6;
        a.status = 0xbf;
        a.tick(|_| 0);
        assert_eq!(a.read(15), 0x7f, "RNMI remains latched through VBI");
    }

    #[test]
    fn vcount_changes_at_111_with_one_cycle_end_of_frame_anomaly() {
        for pal in [false, true] {
            let mut a = Antic::new(pal);
            for y in [3, 4, a.lines - 1] {
                a.y = y;
                a.x = 110;
                assert_eq!(a.read(11), (y / 2) as u8);
                a.x = 111;
                assert_eq!(a.read(11), y.div_ceil(2) as u8);
                a.x = 112;
                assert_eq!(
                    a.read(11),
                    if y + 1 == a.lines {
                        0
                    } else {
                        y.div_ceil(2) as u8
                    }
                );
            }
        }
    }

    #[test]
    fn wsync_holds_after_the_following_cycle_not_the_write_cycle() {
        let mut a = Antic::new(false);
        a.write(10, 0);
        a.tick(|_| 0); // The WSYNC write itself.
        assert!(!a.cpu_read_blocked());
        a.tick(|_| 0); // One following CPU cycle is allowed.
        assert!(a.cpu_read_blocked());
    }

    #[test]
    fn display_list_wrap_lms_text_and_jvb_repeat_without_cpu_patching() {
        let mut a = Antic::new(true);
        let mut ram = vec![0; 65536];
        ram[0x23fe..0x2400].copy_from_slice(&[0x42, 0]);
        ram[0x2000..0x2005].copy_from_slice(&[0x30, 0x41, 0xfe, 0x23, 0]);
        ram[0x3000] = 1;
        ram[0x4008..0x4010].fill(0x80);
        a.write(2, 0xfe);
        a.write(3, 0x23);
        a.write(9, 0x40);
        a.write(0, 0x22);
        a.write(1, 2);
        for _ in 0..(114 * 9) {
            a.tick(|x| ram[usize::from(x)]);
        }
        assert_eq!(a.line()[32], 6);
        assert_eq!(a.line()[33], 5);
        assert_eq!(a.scan, 0x3028);
        for _ in 0..(114 * 312) {
            a.tick(|x| ram[usize::from(x)]);
        }
        assert_eq!(a.line()[32], 6);
        assert!(a.validate().is_ok());
    }
    #[test]
    fn frame_period_and_vblank_nmi_are_generated_by_chip_clocks() {
        for pal in [false, true] {
            let mut a = Antic::new(pal);
            a.write(14, 0x40);
            let mut interrupts = 0;
            for _ in 0..114 * if pal { 312 } else { 262 } {
                a.tick(|_| 0);
                if a.take_nmi() {
                    assert_eq!((a.x, a.y), (9, 248));
                    interrupts += 1;
                }
            }
            assert_eq!((a.x, a.y, interrupts), (0, 0, 1));
            assert_eq!(a.read(15), 0x5f);
        }
    }
    #[test]
    fn wsync_allows_one_cycle_then_waits_until_horizontal_blank() {
        let mut a = Antic::new(false);
        a.write(10, 0);
        assert!(!a.cpu_read_blocked());
        a.tick(|_| 0);
        assert!(!a.cpu_read_blocked());
        a.tick(|_| 0);
        assert!(a.cpu_read_blocked());
        for _ in 2..105 {
            a.tick(|_| 0);
        }
        assert!(!a.cpu_read_blocked());
    }
}
