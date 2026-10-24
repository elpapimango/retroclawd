//! The Yamaha YM2612 (the Mega Drive's FM chip) and its CMOS successor the YM3438, emulated one chip cycle at a time.
//!
//! The chip works on 24 slots (6 channels of 4 operators) in a fixed 24-cycle sequence, one cycle per `clock`; one sample of the
//! output is the sum of 24 cycles' outputs. A chip cycle is 6 clocks of the chip's input clock (the 68000 clock), that is 42
//! master clocks, and the output sample rate is the input clock divided by 144.
//!
//! The stages of the pipeline, their offsets from one another and the behaviour of the registers follow the die analysis of the
//! YM3438 published by the Nuked OPN2 project (Nuke.YKT, with the silicon images of Silicon Pr0n; LGPL-2.1+): the order in
//! which a register write, a key-on, an envelope step and a phase step meet is what decides the samples, and it is only known
//! from the silicon. This crate is written from that understanding, in its own structure, and is checked cycle for cycle
//! against that implementation (`examples/ymscript.rs`, `scenarios/ym_diff.py`, `docs/ym2612-validation.md`). The sine and
//! exponential ROMs are generated from their formulas (`tables.rs`).
//!
//! Facts: S1-S3 and the YM rows in `docs/hardware-facts.md`.
#![forbid(unsafe_code)]

mod tables;

use serde::{Deserialize, Serialize};
use tables::{exp_rom, logsin_rom};

/// Master clocks of one chip cycle (6 input clocks of 7 master clocks each).
pub const CYCLE_MASTER_CLOCKS: u64 = 42;
/// Chip cycles of one output sample.
pub const CYCLES_PER_SAMPLE: usize = 24;

/// Which chip is modelled: the discrete YM2612 of the early consoles (with its DAC peculiarities) or the YM3438 core of the
/// integrated ASIC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Chip {
    Ym2612,
    Ym3438,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum State {
    Attack = 0,
    Decay = 1,
    Sustain = 2,
    Release = 3,
}

/// The note (low two key code bits) from the top four bits of the 11-bit frequency number: documented as
/// N4 = F11, N3 = F11 (F10 + F9 + F8) + !F11 F10 F9 F8.
fn note(fnum: u16) -> u8 {
    let f11 = fnum >> 10 & 1;
    let f10 = fnum >> 9 & 1;
    let f9 = fnum >> 8 & 1;
    let f8 = fnum >> 7 & 1;
    let n4 = f11;
    let n3 = (f11 & (f10 | f9 | f8)) | (!f11 & 1 & f10 & f9 & f8);
    (n4 << 1 | n3) as u8
}

/// Envelope step pattern for the fastest rates (rate 48 and up): by the low two bits of the rate and the timer phase.
const EG_STEP_HIGH: [[u8; 4]; 4] = [[0, 0, 0, 0], [1, 0, 0, 0], [1, 0, 1, 0], [1, 1, 1, 0]];
/// How far the LFO amplitude is shifted down for each amplitude modulation sensitivity.
const AM_SHIFT: [u8; 4] = [7, 3, 1, 0];
/// Detune base values, indexed by `(sum & 1) << 2 | note`.
const DETUNE: [u32; 8] = [16, 17, 19, 20, 22, 24, 27, 29];
/// Vibrato shifts of the phase generator by sensitivity (0-7) and the LFO position (0-7).
const PM_SHIFT_1: [[u8; 8]; 8] = [
    [7, 7, 7, 7, 7, 7, 7, 7],
    [7, 7, 7, 7, 7, 7, 7, 7],
    [7, 7, 7, 7, 7, 7, 1, 1],
    [7, 7, 7, 7, 1, 1, 1, 1],
    [7, 7, 7, 1, 1, 1, 1, 0],
    [7, 7, 1, 1, 0, 0, 0, 0],
    [7, 7, 1, 1, 0, 0, 0, 0],
    [7, 7, 1, 1, 0, 0, 0, 0],
];
const PM_SHIFT_2: [[u8; 8]; 8] = [
    [7, 7, 7, 7, 7, 7, 7, 7],
    [7, 7, 7, 7, 2, 2, 2, 2],
    [7, 7, 7, 2, 2, 2, 7, 7],
    [7, 7, 2, 2, 7, 7, 2, 2],
    [7, 7, 2, 7, 7, 7, 2, 7],
    [7, 7, 7, 2, 7, 7, 2, 1],
    [7, 7, 7, 2, 7, 7, 2, 1],
    [7, 7, 7, 2, 7, 7, 2, 1],
];
/// The slot register address of each of the 12 positions of the address decoder (bit 8 selects the second register bank).
const SLOT_ADDRESS: [u16; 12] = [
    0x000, 0x001, 0x002, 0x100, 0x101, 0x102, 0x004, 0x005, 0x006, 0x104, 0x105, 0x106,
];
const CHANNEL_ADDRESS: [u16; 6] = [0x000, 0x001, 0x002, 0x100, 0x101, 0x102];
/// LFO divider thresholds by frequency setting.
const LFO_CYCLES: [u8; 8] = [108, 77, 71, 67, 62, 44, 8, 5];
/// Which operator outputs feed which inputs, by the position in the evaluation order (OP1, OP3, OP2, OP4) and the algorithm.
/// Rows: OP1 delayed once, OP1 delayed twice, OP2's previous output, the operator just before in the order (two kinds), the
/// output mix.
const ALGORITHM: [[[bool; 8]; 6]; 4] = {
    const fn row(a: [u8; 8]) -> [bool; 8] {
        [
            a[0] != 0,
            a[1] != 0,
            a[2] != 0,
            a[3] != 0,
            a[4] != 0,
            a[5] != 0,
            a[6] != 0,
            a[7] != 0,
        ]
    }
    [
        [
            row([1, 1, 1, 1, 1, 1, 1, 1]),
            row([1, 1, 1, 1, 1, 1, 1, 1]),
            row([0, 0, 0, 0, 0, 0, 0, 0]),
            row([0, 0, 0, 0, 0, 0, 0, 0]),
            row([0, 0, 0, 0, 0, 0, 0, 0]),
            row([0, 0, 0, 0, 0, 0, 0, 1]),
        ],
        [
            row([0, 1, 0, 0, 0, 1, 0, 0]),
            row([0, 0, 0, 0, 0, 0, 0, 0]),
            row([1, 1, 1, 0, 0, 0, 0, 0]),
            row([0, 0, 0, 0, 0, 0, 0, 0]),
            row([0, 0, 0, 0, 0, 0, 0, 0]),
            row([0, 0, 0, 0, 0, 1, 1, 1]),
        ],
        [
            row([0, 0, 0, 0, 0, 0, 0, 0]),
            row([0, 0, 0, 0, 0, 0, 0, 0]),
            row([0, 0, 0, 0, 0, 0, 0, 0]),
            row([1, 0, 0, 1, 1, 1, 1, 0]),
            row([0, 0, 0, 0, 0, 0, 0, 0]),
            row([0, 0, 0, 0, 1, 1, 1, 1]),
        ],
        [
            row([0, 0, 1, 0, 0, 1, 0, 0]),
            row([0, 0, 0, 0, 0, 0, 0, 0]),
            row([0, 0, 0, 1, 0, 0, 0, 0]),
            row([1, 1, 0, 1, 1, 0, 0, 0]),
            row([0, 0, 1, 0, 0, 0, 0, 0]),
            row([1, 1, 1, 1, 1, 1, 1, 1]),
        ],
    ]
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ym2612 {
    kind: Chip,
    /// The cycle of the 24 now running and the channel (cycle mod 6) it belongs to.
    cycles: usize,
    channel: usize,
    mol: i16,
    mor: i16,
    // bus interface
    write_data: u16,
    write_a: u8,
    write_d: u8,
    write_a_en: bool,
    write_d_en: bool,
    write_busy: bool,
    write_busy_cnt: u8,
    write_fm_address: bool,
    write_fm_data: bool,
    write_fm_mode_a: u16,
    address: u16,
    data: u8,
    busy: bool,
    // LFO
    lfo_en: u8,
    lfo_freq: u8,
    lfo_pm: u8,
    lfo_am: u8,
    lfo_cnt: u8,
    lfo_inc: u8,
    lfo_quotient: u8,
    // phase generator
    pg_fnum: u16,
    pg_block: u8,
    pg_kcode: u8,
    pg_inc: [u32; 24],
    pg_phase: [u32; 24],
    pg_reset: [bool; 24],
    // envelope generator
    eg_cycle: u8,
    eg_cycle_stop: bool,
    eg_shift: u8,
    eg_shift_lock: u8,
    eg_timer_low_lock: u8,
    eg_timer: u16,
    eg_timer_inc: u8,
    eg_quotient: u16,
    eg_rate: u8,
    eg_ksv: u8,
    eg_inc: u8,
    eg_ratemax: bool,
    eg_sl: [u8; 2],
    eg_lfo_am: u16,
    eg_tl: [u8; 2],
    eg_state: [State; 24],
    eg_level: [u16; 24],
    eg_out: [u16; 24],
    eg_kon: [bool; 24],
    eg_kon_csm: [bool; 24],
    eg_kon_latch: [bool; 24],
    eg_ssg_enable: [bool; 24],
    eg_ssg_pgrst_latch: [bool; 24],
    eg_ssg_repeat_latch: [bool; 24],
    eg_ssg_hold_up_latch: [bool; 24],
    eg_ssg_dir: [bool; 24],
    eg_ssg_inv: [bool; 24],
    // operators and channels
    fm_op1: [[i16; 2]; 6],
    fm_op2: [i16; 6],
    fm_out: [i16; 24],
    fm_mod: [u16; 24],
    ch_acc: [i16; 6],
    ch_out: [i16; 6],
    ch_lock: i16,
    ch_lock_l: bool,
    ch_lock_r: bool,
    // timers
    timer_a_cnt: u16,
    timer_a_reg: u16,
    timer_a_load_lock: bool,
    timer_a_load: bool,
    timer_a_enable: bool,
    timer_a_reset: bool,
    timer_a_load_latch: bool,
    timer_a_overflow_flag: bool,
    timer_a_overflow: bool,
    timer_b_cnt: u16,
    timer_b_subcnt: u8,
    timer_b_reg: u16,
    timer_b_load_lock: bool,
    timer_b_load: bool,
    timer_b_enable: bool,
    timer_b_reset: bool,
    timer_b_load_latch: bool,
    timer_b_overflow_flag: bool,
    timer_b_overflow: bool,
    // register set
    mode_test_21: [bool; 8],
    mode_test_2c: [bool; 8],
    mode_ch3: u8,
    mode_kon_channel: u8,
    mode_kon_operator: [bool; 4],
    mode_kon: [bool; 24],
    mode_csm: bool,
    mode_kon_csm: bool,
    dacen: bool,
    dacdata: i16,
    ks: [u8; 24],
    ar: [u8; 24],
    sr: [u8; 24],
    dt: [u8; 24],
    multi: [u8; 24],
    sl: [u8; 24],
    rr: [u8; 24],
    dr: [u8; 24],
    am: [bool; 24],
    tl: [u8; 24],
    ssg_eg: [u8; 24],
    fnum: [u16; 6],
    block: [u8; 6],
    kcode: [u8; 6],
    fnum_3ch: [u16; 6],
    block_3ch: [u8; 6],
    kcode_3ch: [u8; 6],
    reg_a4: u8,
    reg_ac: u8,
    connect: [u8; 6],
    fb: [u8; 6],
    pan_l: [bool; 6],
    pan_r: [bool; 6],
    ams: [u8; 6],
    pms: [u8; 6],
    status: u8,
    status_time: u32,
}

impl Ym2612 {
    pub fn new(kind: Chip) -> Self {
        Self {
            kind,
            cycles: 0,
            channel: 0,
            mol: 0,
            mor: 0,
            write_data: 0,
            write_a: 0,
            write_d: 0,
            write_a_en: false,
            write_d_en: false,
            write_busy: false,
            write_busy_cnt: 0,
            write_fm_address: false,
            write_fm_data: false,
            write_fm_mode_a: 0,
            address: 0,
            data: 0,
            busy: false,
            lfo_en: 0,
            lfo_freq: 0,
            lfo_pm: 0,
            lfo_am: 0,
            lfo_cnt: 0,
            lfo_inc: 0,
            lfo_quotient: 0,
            pg_fnum: 0,
            pg_block: 0,
            pg_kcode: 0,
            pg_inc: [0; 24],
            pg_phase: [0; 24],
            pg_reset: [false; 24],
            eg_cycle: 0,
            eg_cycle_stop: false,
            eg_shift: 0,
            eg_shift_lock: 0,
            eg_timer_low_lock: 0,
            eg_timer: 0,
            eg_timer_inc: 0,
            eg_quotient: 0,
            eg_rate: 0,
            eg_ksv: 0,
            eg_inc: 0,
            eg_ratemax: false,
            eg_sl: [0; 2],
            eg_lfo_am: 0,
            eg_tl: [0; 2],
            eg_state: [State::Release; 24],
            eg_level: [0x3FF; 24],
            eg_out: [0x3FF; 24],
            eg_kon: [false; 24],
            eg_kon_csm: [false; 24],
            eg_kon_latch: [false; 24],
            eg_ssg_enable: [false; 24],
            eg_ssg_pgrst_latch: [false; 24],
            eg_ssg_repeat_latch: [false; 24],
            eg_ssg_hold_up_latch: [false; 24],
            eg_ssg_dir: [false; 24],
            eg_ssg_inv: [false; 24],
            fm_op1: [[0; 2]; 6],
            fm_op2: [0; 6],
            fm_out: [0; 24],
            fm_mod: [0; 24],
            ch_acc: [0; 6],
            ch_out: [0; 6],
            ch_lock: 0,
            ch_lock_l: false,
            ch_lock_r: false,
            timer_a_cnt: 0,
            timer_a_reg: 0,
            timer_a_load_lock: false,
            timer_a_load: false,
            timer_a_enable: false,
            timer_a_reset: false,
            timer_a_load_latch: false,
            timer_a_overflow_flag: false,
            timer_a_overflow: false,
            timer_b_cnt: 0,
            timer_b_subcnt: 0,
            timer_b_reg: 0,
            timer_b_load_lock: false,
            timer_b_load: false,
            timer_b_enable: false,
            timer_b_reset: false,
            timer_b_load_latch: false,
            timer_b_overflow_flag: false,
            timer_b_overflow: false,
            mode_test_21: [false; 8],
            mode_test_2c: [false; 8],
            mode_ch3: 0,
            mode_kon_channel: 0,
            mode_kon_operator: [false; 4],
            mode_kon: [false; 24],
            mode_csm: false,
            mode_kon_csm: false,
            dacen: false,
            dacdata: 0,
            ks: [0; 24],
            ar: [0; 24],
            sr: [0; 24],
            dt: [0; 24],
            multi: [1; 24],
            sl: [0; 24],
            rr: [0; 24],
            dr: [0; 24],
            am: [false; 24],
            tl: [0; 24],
            ssg_eg: [0; 24],
            fnum: [0; 6],
            block: [0; 6],
            kcode: [0; 6],
            fnum_3ch: [0; 6],
            block_3ch: [0; 6],
            kcode_3ch: [0; 6],
            reg_a4: 0,
            reg_ac: 0,
            connect: [0; 6],
            fb: [0; 6],
            pan_l: [true; 6],
            pan_r: [true; 6],
            ams: [0; 6],
            pms: [0; 6],
            status: 0,
            status_time: 0,
        }
    }

    /// Whether a value read back from a saved state is usable: every field the code uses as an index or a shift is in range.
    pub fn validate(&self) -> Result<(), String> {
        let bad = |what: &str| Err(format!("YM2612 {what} is out of range"));
        if self.cycles >= 24 || self.channel >= 6 || self.channel != self.cycles % 6 {
            return bad("cycle");
        }
        if self.lfo_freq > 7
            || self.eg_cycle > 15
            || self.eg_timer_low_lock > 3
            || self.eg_shift > 15
            || self.eg_shift_lock > 16
        {
            return bad("counter");
        }
        if self.eg_quotient > 2 || self.eg_timer > 0xFFF || self.timer_b_subcnt > 0x10 {
            return bad("timer");
        }
        if self.connect.iter().any(|v| *v > 7)
            || self.fb.iter().any(|v| *v > 7)
            || self.pms.iter().any(|v| *v > 7)
            || self.ams.iter().any(|v| *v > 3)
            || self.ks.iter().any(|v| *v > 3)
            || self.dt.iter().any(|v| *v > 7)
            || self.block.iter().any(|v| *v > 7)
            || self.block_3ch.iter().any(|v| *v > 7)
            || self.eg_level.iter().any(|v| *v > 0x3FF)
            || self.eg_out.iter().any(|v| *v > 0x3FF)
            || self.pg_phase.iter().any(|v| *v > 0xF_FFFF)
            || self.pg_inc.iter().any(|v| *v > 0xF_FFFF)
            || self.multi.iter().any(|v| *v > 30)
            || self.sl.iter().any(|v| *v > 31)
            || self.eg_sl.iter().any(|v| *v > 31)
        {
            return bad("register");
        }
        if self.pg_block > 7
            || self.pg_fnum > 0x7FF
            || self.eg_lfo_am > 0xFF
            || self.eg_rate > 0x3F
            || self.eg_ksv > 0x1F
        {
            return bad("generator value");
        }
        Ok(())
    }

    pub fn kind(&self) -> Chip {
        self.kind
    }

    /// A write to one of the four ports (0 and 2 select a register of bank 0 or 1, 1 and 3 write its data).
    pub fn write(&mut self, port: u32, data: u8) {
        let port = port & 3;
        self.write_data = ((port << 7) & 0x100) as u16 | u16::from(data);
        if port & 1 != 0 {
            self.write_d |= 1;
        } else {
            self.write_a |= 1;
        }
    }

    /// The status register: busy in bit 7, timer B and timer A overflow flags in bits 1 and 0. The value a read returns
    /// stays for a long while on the YM2612 (reads of the other ports see the same register).
    pub fn read(&mut self, port: u32) -> u8 {
        // A YM2612 answers a status read on port 0 only (the others repeat the last value for a while); the YM3438 on any port.
        if port & 3 == 0 || self.kind == Chip::Ym3438 {
            self.status = u8::from(self.busy) << 7
                | u8::from(self.timer_b_overflow_flag) << 1
                | u8::from(self.timer_a_overflow_flag);
            self.status_time = match self.kind {
                Chip::Ym2612 => 300_000,
                Chip::Ym3438 => 40_000_000,
            };
        }
        if self.status_time != 0 {
            self.status
        } else {
            0
        }
    }

    /// The interrupt line: either timer flag.
    pub fn irq(&self) -> bool {
        self.timer_a_overflow_flag | self.timer_b_overflow_flag
    }

    fn do_io(&mut self) {
        self.write_a_en = self.write_a & 3 == 1;
        self.write_d_en = self.write_d & 3 == 1;
        self.write_a <<= 1;
        self.write_d <<= 1;
        self.busy = self.write_busy;
        self.write_busy_cnt = self.write_busy_cnt.wrapping_add(u8::from(self.write_busy));
        self.write_busy = (self.write_busy && (self.write_busy_cnt >> 5) == 0) || self.write_d_en;
        self.write_busy_cnt &= 0x1F;
    }

    fn do_reg_write(&mut self) {
        let mut slot = self.cycles % 12;
        let channel = self.channel;
        if self.write_fm_data {
            if SLOT_ADDRESS[slot] == self.address & 0x107 {
                if self.address & 0x08 != 0 {
                    slot += 12;
                }
                let data = self.data;
                match self.address & 0xF0 {
                    0x30 => {
                        self.multi[slot] = if data & 0x0F == 0 {
                            1
                        } else {
                            (data & 0x0F) << 1
                        };
                        self.dt[slot] = (data >> 4) & 7;
                    }
                    0x40 => self.tl[slot] = data & 0x7F,
                    0x50 => {
                        self.ar[slot] = data & 0x1F;
                        self.ks[slot] = (data >> 6) & 3;
                    }
                    0x60 => {
                        self.dr[slot] = data & 0x1F;
                        self.am[slot] = data >> 7 & 1 != 0;
                    }
                    0x70 => self.sr[slot] = data & 0x1F,
                    0x80 => {
                        self.rr[slot] = data & 0x0F;
                        let sl = (data >> 4) & 0x0F;
                        self.sl[slot] = sl | (sl + 1) & 0x10;
                    }
                    0x90 => self.ssg_eg[slot] = data & 0x0F,
                    _ => {}
                }
            }
            if CHANNEL_ADDRESS[channel] == self.address & 0x103 {
                let data = self.data;
                match self.address & 0xFC {
                    0xA0 => {
                        self.fnum[channel] = u16::from(data) | u16::from(self.reg_a4 & 7) << 8;
                        self.block[channel] = (self.reg_a4 >> 3) & 7;
                        self.kcode[channel] = self.block[channel] << 2 | note(self.fnum[channel]);
                    }
                    0xA4 => self.reg_a4 = data,
                    0xA8 => {
                        self.fnum_3ch[channel] = u16::from(data) | u16::from(self.reg_ac & 7) << 8;
                        self.block_3ch[channel] = (self.reg_ac >> 3) & 7;
                        self.kcode_3ch[channel] =
                            self.block_3ch[channel] << 2 | note(self.fnum_3ch[channel]);
                    }
                    0xAC => self.reg_ac = data,
                    0xB0 => {
                        self.connect[channel] = data & 7;
                        self.fb[channel] = (data >> 3) & 7;
                    }
                    0xB4 => {
                        self.pms[channel] = data & 7;
                        self.ams[channel] = (data >> 4) & 3;
                        self.pan_l[channel] = data >> 7 & 1 != 0;
                        self.pan_r[channel] = data >> 6 & 1 != 0;
                    }
                    _ => {}
                }
            }
        }

        if self.write_a_en || self.write_d_en {
            if self.write_a_en {
                self.write_fm_data = false;
            }
            if self.write_fm_address && self.write_d_en {
                self.write_fm_data = true;
            }
            if self.write_a_en {
                if self.write_data & 0xF0 != 0 {
                    self.address = self.write_data;
                    self.write_fm_address = true;
                } else {
                    self.write_fm_address = false;
                }
            }
            if self.write_d_en && self.write_data & 0x100 == 0 {
                let d = self.write_data;
                let bit = |n: u32| d >> n & 1 != 0;
                match self.write_fm_mode_a {
                    0x21 => {
                        for i in 0..8 {
                            self.mode_test_21[i] = bit(i as u32);
                        }
                    }
                    0x22 => {
                        self.lfo_en = if bit(3) { 0x7F } else { 0 };
                        self.lfo_freq = (d & 7) as u8;
                    }
                    0x24 => {
                        self.timer_a_reg &= 0x03;
                        self.timer_a_reg |= (d & 0xFF) << 2;
                    }
                    0x25 => {
                        self.timer_a_reg &= 0x3FC;
                        self.timer_a_reg |= d & 3;
                    }
                    0x26 => self.timer_b_reg = d & 0xFF,
                    0x27 => {
                        self.mode_ch3 = ((d & 0xC0) >> 6) as u8;
                        self.mode_csm = self.mode_ch3 == 2;
                        self.timer_a_load = bit(0);
                        self.timer_a_enable = bit(2);
                        self.timer_a_reset = bit(4);
                        self.timer_b_load = bit(1);
                        self.timer_b_enable = bit(3);
                        self.timer_b_reset = bit(5);
                    }
                    0x28 => {
                        for i in 0..4 {
                            self.mode_kon_operator[i] = bit(4 + i as u32);
                        }
                        self.mode_kon_channel = if d & 3 == 3 {
                            0xFF
                        } else {
                            ((d & 3) + ((d >> 2) & 1) * 3) as u8
                        };
                    }
                    0x2A => {
                        self.dacdata &= 1;
                        self.dacdata |= ((d as u8 ^ 0x80) as i16) << 1;
                    }
                    0x2B => self.dacen = d >> 7 & 1 != 0,
                    0x2C => {
                        for i in 0..8 {
                            self.mode_test_2c[i] = bit(i as u32);
                        }
                        self.dacdata &= 0x1FE;
                        self.dacdata |= i16::from(self.mode_test_2c[3]);
                    }
                    _ => {}
                }
            }
            if self.write_a_en {
                self.write_fm_mode_a = self.write_data & 0x1FF;
            }
        }

        if self.write_fm_data {
            self.data = self.write_data as u8;
        }
    }

    fn phase_calc_increment(&mut self) {
        let chan = self.channel;
        let slot = self.cycles;
        let mut fnum = u32::from(self.pg_fnum);
        let fnum_h = fnum >> 4;
        let lfo = self.lfo_pm;
        let mut lfo_l = lfo & 0x0F;
        let pms = usize::from(self.pms[chan]);
        let dt = self.dt[slot];
        let dt_l = dt & 3;
        let mut detune = 0u32;
        let mut kcode = self.pg_kcode;
        fnum <<= 1;
        if lfo_l & 0x08 != 0 {
            lfo_l ^= 0x0F;
        }
        let mut fm = (fnum_h >> PM_SHIFT_1[pms][usize::from(lfo_l)])
            + (fnum_h >> PM_SHIFT_2[pms][usize::from(lfo_l)]);
        if pms > 5 {
            fm <<= pms - 5;
        }
        fm >>= 2;
        if lfo & 0x10 != 0 {
            fnum = fnum.wrapping_sub(fm);
        } else {
            fnum += fm;
        }
        fnum &= 0xFFF;
        let mut basefreq = (fnum << self.pg_block) >> 2;
        if dt_l != 0 {
            if kcode > 0x1C {
                kcode = 0x1C;
            }
            let block = kcode >> 2;
            let note = kcode & 3;
            let sum = block + 9 + (u8::from(dt_l == 3) | (dt_l & 2));
            let sum_h = sum >> 1;
            let sum_l = sum & 1;
            detune = DETUNE[usize::from(sum_l << 2 | note)] >> (9 - sum_h);
        }
        if dt & 4 != 0 {
            basefreq = basefreq.wrapping_sub(detune);
        } else {
            basefreq += detune;
        }
        basefreq &= 0x1_FFFF;
        self.pg_inc[slot] = (basefreq * u32::from(self.multi[slot])) >> 1 & 0xF_FFFF;
    }

    fn phase_generate(&mut self) {
        let slot = (self.cycles + 20) % 24;
        if self.pg_reset[slot] {
            self.pg_inc[slot] = 0;
        }
        let slot = (self.cycles + 19) % 24;
        if self.pg_reset[slot] || self.mode_test_21[3] {
            self.pg_phase[slot] = 0;
        }
        self.pg_phase[slot] = (self.pg_phase[slot] + self.pg_inc[slot]) & 0xF_FFFF;
    }

    fn envelope_ssg_eg(&mut self) {
        let slot = self.cycles;
        let mut direction = false;
        self.eg_ssg_pgrst_latch[slot] = false;
        self.eg_ssg_repeat_latch[slot] = false;
        self.eg_ssg_hold_up_latch[slot] = false;
        let ssg = self.ssg_eg[slot];
        if ssg & 0x08 != 0 {
            direction = self.eg_ssg_dir[slot];
            if self.eg_level[slot] & 0x200 != 0 {
                if ssg & 3 == 0 {
                    self.eg_ssg_pgrst_latch[slot] = true;
                }
                if ssg & 1 == 0 {
                    self.eg_ssg_repeat_latch[slot] = true;
                }
                if ssg & 3 == 2 {
                    direction = !direction;
                }
                if ssg & 3 == 3 {
                    direction = true;
                }
            }
            if self.eg_kon_latch[slot] && (ssg & 7 == 5 || ssg & 7 == 3) {
                self.eg_ssg_hold_up_latch[slot] = true;
            }
            direction &= self.eg_kon[slot];
        }
        self.eg_ssg_dir[slot] = direction;
        self.eg_ssg_enable[slot] = ssg >> 3 & 1 != 0;
        self.eg_ssg_inv[slot] =
            (direction ^ ((ssg >> 2 & 1 != 0) && (ssg >> 3 & 1 != 0))) && self.eg_kon[slot];
    }

    fn envelope_adsr(&mut self) {
        let slot = (self.cycles + 22) % 24;
        let nkon = self.eg_kon_latch[slot];
        let okon = self.eg_kon[slot];
        let mut nextstate = self.eg_state[slot];
        let mut inc: i32 = 0;

        // Reset the phase generator.
        self.pg_reset[slot] = (nkon && !okon) || self.eg_ssg_pgrst_latch[slot];

        let kon_event = (nkon && !okon) || (okon && self.eg_ssg_repeat_latch[slot]);
        let koff_event = okon && !nkon;

        let mut level = i32::from(self.eg_level[slot]);
        let mut ssg_level = level;
        if self.eg_ssg_inv[slot] {
            ssg_level = (512 - level) & 0x3FF;
        }
        if koff_event {
            level = ssg_level;
        }
        let eg_off = if self.eg_ssg_enable[slot] {
            level >> 9 != 0
        } else {
            level & 0x3F0 == 0x3F0
        };
        let mut nextlevel = level;
        if kon_event {
            nextstate = State::Attack;
            if self.eg_ratemax {
                nextlevel = 0;
            } else if self.eg_state[slot] == State::Attack && level != 0 && self.eg_inc != 0 && nkon
            {
                inc = (!level << self.eg_inc) >> 5;
            }
        } else {
            match self.eg_state[slot] {
                State::Attack => {
                    if level == 0 {
                        nextstate = State::Decay;
                    } else if self.eg_inc != 0 && !self.eg_ratemax && nkon {
                        inc = (!level << self.eg_inc) >> 5;
                    }
                }
                State::Decay => {
                    if level >> 4 == i32::from(self.eg_sl[1]) << 1 {
                        nextstate = State::Sustain;
                    } else if !eg_off && self.eg_inc != 0 {
                        inc = 1 << (self.eg_inc - 1);
                        if self.eg_ssg_enable[slot] {
                            inc <<= 2;
                        }
                    }
                }
                State::Sustain | State::Release => {
                    if !eg_off && self.eg_inc != 0 {
                        inc = 1 << (self.eg_inc - 1);
                        if self.eg_ssg_enable[slot] {
                            inc <<= 2;
                        }
                    }
                }
            }
            if !nkon {
                nextstate = State::Release;
            }
        }
        if self.eg_kon_csm[slot] {
            nextlevel |= i32::from(self.eg_tl[1]) << 3;
        }
        // The envelope is off.
        if !kon_event
            && !self.eg_ssg_hold_up_latch[slot]
            && self.eg_state[slot] != State::Attack
            && eg_off
        {
            nextstate = State::Release;
            nextlevel = 0x3FF;
        }
        nextlevel += inc;
        self.eg_kon[slot] = self.eg_kon_latch[slot];
        self.eg_level[slot] = (nextlevel & 0x3FF) as u16;
        self.eg_state[slot] = nextstate;
    }

    fn envelope_prepare(&mut self) {
        let slot = self.cycles;
        let mut inc = 0u8;
        let mut rate = (self.eg_rate << 1).wrapping_add(self.eg_ksv);
        if rate > 0x3F {
            rate = 0x3F;
        }
        let sum = ((rate >> 2) + self.eg_shift_lock) & 0x0F;
        if self.eg_rate != 0 && self.eg_quotient == 2 {
            if rate < 48 {
                inc = match sum {
                    12 => 1,
                    13 => (rate >> 1) & 1,
                    14 => rate & 1,
                    _ => 0,
                };
            } else {
                let step = EG_STEP_HIGH[usize::from(rate & 3)][usize::from(self.eg_timer_low_lock)];
                inc = (step + (rate >> 2)).wrapping_sub(11);
                if inc > 4 {
                    inc = 4;
                }
            }
        }
        self.eg_inc = inc;
        self.eg_ratemax = rate >> 1 == 0x1F;

        let mut rate_sel = self.eg_state[slot];
        if (self.eg_kon[slot] && self.eg_ssg_repeat_latch[slot])
            || (!self.eg_kon[slot] && self.eg_kon_latch[slot])
        {
            rate_sel = State::Attack;
        }
        self.eg_rate = match rate_sel {
            State::Attack => self.ar[slot],
            State::Decay => self.dr[slot],
            State::Sustain => self.sr[slot],
            State::Release => self.rr[slot] << 1 | 1,
        };
        self.eg_ksv = self.pg_kcode >> (self.ks[slot] ^ 3);
        self.eg_lfo_am = if self.am[slot] {
            u16::from(self.lfo_am >> AM_SHIFT[usize::from(self.ams[self.channel])])
        } else {
            0
        };
        self.eg_tl[1] = self.eg_tl[0];
        self.eg_tl[0] = self.tl[slot];
        self.eg_sl[1] = self.eg_sl[0];
        self.eg_sl[0] = self.sl[slot];
    }

    fn envelope_generate(&mut self) {
        let slot = (self.cycles + 23) % 24;
        let mut level = i32::from(self.eg_level[slot]);
        if self.eg_ssg_inv[slot] {
            level = 512 - level;
        }
        if self.mode_test_21[5] {
            level = 0;
        }
        level &= 0x3FF;
        level += i32::from(self.eg_lfo_am);
        if !(self.mode_csm && self.channel == 2 + 1) {
            level += i32::from(self.eg_tl[0]) << 3;
        }
        if level > 0x3FF {
            level = 0x3FF;
        }
        self.eg_out[slot] = level as u16;
    }

    fn update_lfo(&mut self) {
        let threshold = LFO_CYCLES[usize::from(self.lfo_freq)];
        if self.lfo_quotient & threshold == threshold {
            self.lfo_quotient = 0;
            self.lfo_cnt = self.lfo_cnt.wrapping_add(1);
        } else {
            self.lfo_quotient = self.lfo_quotient.wrapping_add(self.lfo_inc);
        }
        self.lfo_cnt &= self.lfo_en;
    }

    fn fm_prepare(&mut self) {
        let slot = (self.cycles + 6) % 24;
        let channel = self.channel;
        let op = slot / 6;
        let connect = usize::from(self.connect[channel]);
        let prevslot = (self.cycles + 18) % 24;
        let mut mod1: i16 = 0;
        let mut mod2: i16 = 0;
        if ALGORITHM[op][0][connect] {
            mod2 |= self.fm_op1[channel][0];
        }
        if ALGORITHM[op][1][connect] {
            mod1 |= self.fm_op1[channel][1];
        }
        if ALGORITHM[op][2][connect] {
            mod1 |= self.fm_op2[channel];
        }
        if ALGORITHM[op][3][connect] {
            mod2 |= self.fm_out[prevslot];
        }
        if ALGORITHM[op][4][connect] {
            mod1 |= self.fm_out[prevslot];
        }
        let mut m = mod1.wrapping_add(mod2);
        if op == 0 {
            // feedback
            m >>= 10 - self.fb[channel];
            if self.fb[channel] == 0 {
                m = 0;
            }
        } else {
            m >>= 1;
        }
        self.fm_mod[slot] = m as u16;

        let slot = (self.cycles + 18) % 24;
        if slot / 6 == 0 {
            self.fm_op1[channel][1] = self.fm_op1[channel][0];
            self.fm_op1[channel][0] = self.fm_out[slot];
        }
        if slot / 6 == 2 {
            self.fm_op2[channel] = self.fm_out[slot];
        }
    }

    fn ch_generate(&mut self) {
        let slot = (self.cycles + 18) % 24;
        let channel = self.channel;
        let op = slot / 6;
        let test_dac = self.mode_test_2c[5];
        let mut acc = self.ch_acc[channel];
        let mut add = i16::from(test_dac);
        if op == 0 && !test_dac {
            acc = 0;
        }
        if ALGORITHM[op][5][usize::from(self.connect[channel])] && !test_dac {
            add += self.fm_out[slot] >> 5;
        }
        let sum = (acc + add).clamp(-256, 255);
        if op == 0 || test_dac {
            self.ch_out[channel] = self.ch_acc[channel];
        }
        self.ch_acc[channel] = sum;
    }

    fn ch_output(&mut self) {
        let cycles = self.cycles;
        let mut channel = self.channel;
        let test_dac = self.mode_test_2c[5];
        if cycles < 12 {
            channel += 1;
        }
        if cycles & 3 == 0 {
            if !test_dac {
                self.ch_lock = self.ch_out[channel];
            }
            self.ch_lock_l = self.pan_l[channel];
            self.ch_lock_r = self.pan_r[channel];
        }
        let out: i16 = if ((cycles >> 2) == 1 && self.dacen) || test_dac {
            // the DAC replaces channel 6: a signed 9-bit value
            sign_extend(self.dacdata, 8)
        } else {
            self.ch_lock
        };
        self.mol = 0;
        self.mor = 0;
        match self.kind {
            Chip::Ym2612 => {
                let out_en = cycles & 3 == 3 || test_dac;
                let mut out = out;
                let mut sign = out >> 8;
                if out >= 0 {
                    out += 1;
                    sign += 1;
                }
                self.mol = if self.ch_lock_l && out_en { out } else { sign };
                self.mor = if self.ch_lock_r && out_en { out } else { sign };
                self.mol = self.mol.wrapping_mul(3);
                self.mor = self.mor.wrapping_mul(3);
            }
            Chip::Ym3438 => {
                let out_en = cycles & 3 != 0 || test_dac;
                if self.ch_lock_l && out_en {
                    self.mol = out;
                }
                if self.ch_lock_r && out_en {
                    self.mor = out;
                }
            }
        }
    }

    fn fm_generate(&mut self) {
        let slot = (self.cycles + 19) % 24;
        let phase = (self.fm_mod[slot].wrapping_add((self.pg_phase[slot] >> 10) as u16)) & 0x3FF;
        let quarter = if phase & 0x100 != 0 {
            (phase ^ 0xFF) & 0xFF
        } else {
            phase & 0xFF
        };
        let mut level = u32::from(logsin_rom(usize::from(quarter)));
        level += u32::from(self.eg_out[slot]) << 2;
        if level > 0x1FFF {
            level = 0x1FFF;
        }
        let mut output =
            ((u32::from(exp_rom(((level & 0xFF) ^ 0xFF) as usize)) | 0x400) << 2) >> (level >> 8);
        let flip = u32::from(self.mode_test_21[4]) << 13;
        output = if phase & 0x200 != 0 {
            ((!output) ^ flip).wrapping_add(1)
        } else {
            output ^ flip
        };
        self.fm_out[slot] = sign_extend(output as i16, 13);
    }

    fn do_timer_a(&mut self) {
        let mut load = self.timer_a_overflow;
        if self.cycles == 2 {
            load |= !self.timer_a_load_lock && self.timer_a_load;
            self.timer_a_load_lock = self.timer_a_load;
            self.mode_kon_csm = self.mode_csm && load;
        }
        let mut time = if self.timer_a_load_latch {
            self.timer_a_reg
        } else {
            self.timer_a_cnt
        };
        self.timer_a_load_latch = load;
        if (self.cycles == 1 && self.timer_a_load_lock) || self.mode_test_21[2] {
            time += 1;
        }
        if self.timer_a_reset {
            self.timer_a_reset = false;
            self.timer_a_overflow_flag = false;
        } else {
            self.timer_a_overflow_flag |= self.timer_a_overflow & self.timer_a_enable;
        }
        self.timer_a_overflow = time >> 10 != 0;
        self.timer_a_cnt = time & 0x3FF;
    }

    fn do_timer_b(&mut self) {
        let mut load = self.timer_b_overflow;
        if self.cycles == 2 {
            load |= !self.timer_b_load_lock && self.timer_b_load;
            self.timer_b_load_lock = self.timer_b_load;
        }
        let mut time = if self.timer_b_load_latch {
            self.timer_b_reg
        } else {
            self.timer_b_cnt
        };
        self.timer_b_load_latch = load;
        if self.cycles == 1 {
            self.timer_b_subcnt = self.timer_b_subcnt.wrapping_add(1);
        }
        if (self.timer_b_subcnt == 0x10 && self.timer_b_load_lock) || self.mode_test_21[2] {
            time += 1;
        }
        self.timer_b_subcnt &= 0x0F;
        if self.timer_b_reset {
            self.timer_b_reset = false;
            self.timer_b_overflow_flag = false;
        } else {
            self.timer_b_overflow_flag |= self.timer_b_overflow & self.timer_b_enable;
        }
        self.timer_b_overflow = time >> 8 != 0;
        self.timer_b_cnt = time & 0xFF;
    }

    fn key_on(&mut self) {
        let slot = self.cycles;
        let chan = self.channel;
        self.eg_kon_latch[slot] = self.mode_kon[slot];
        self.eg_kon_csm[slot] = false;
        if self.channel == 2 && self.mode_kon_csm {
            self.eg_kon_latch[slot] = true;
            self.eg_kon_csm[slot] = true;
        }
        if self.cycles == usize::from(self.mode_kon_channel) {
            self.mode_kon[chan] = self.mode_kon_operator[0];
            self.mode_kon[chan + 12] = self.mode_kon_operator[1];
            self.mode_kon[chan + 6] = self.mode_kon_operator[2];
            self.mode_kon[chan + 18] = self.mode_kon_operator[3];
        }
    }

    /// Runs one chip cycle; returns the left and right outputs of this cycle (one sample is the sum of 24 cycles).
    pub fn clock(&mut self) -> (i16, i16) {
        let slot = self.cycles;
        self.lfo_inc = u8::from(self.mode_test_21[1]);
        self.eg_cycle = self.eg_cycle.wrapping_add(1);
        if self.cycles == 1 && self.eg_quotient == 2 {
            self.eg_shift_lock = if self.eg_cycle_stop {
                0
            } else {
                self.eg_shift + 1
            };
            self.eg_timer_low_lock = (self.eg_timer & 3) as u8;
        }
        match self.cycles {
            0 => {
                self.lfo_pm = self.lfo_cnt >> 2;
                self.lfo_am = if self.lfo_cnt & 0x40 != 0 {
                    self.lfo_cnt & 0x3F
                } else {
                    self.lfo_cnt ^ 0x3F
                };
                self.lfo_am <<= 1;
            }
            1 => {
                self.eg_quotient = (self.eg_quotient + 1) % 3;
                self.eg_cycle = 0;
                self.eg_cycle_stop = true;
                self.eg_shift = 0;
                self.eg_timer_inc |= (self.eg_quotient >> 1) as u8;
                self.eg_timer += u16::from(self.eg_timer_inc);
                self.eg_timer_inc = (self.eg_timer >> 12) as u8;
                self.eg_timer &= 0xFFF;
            }
            13 => {
                self.eg_cycle = 0;
                self.eg_cycle_stop = true;
                self.eg_shift = 0;
                self.eg_timer += u16::from(self.eg_timer_inc);
                self.eg_timer_inc = (self.eg_timer >> 12) as u8;
                self.eg_timer &= 0xFFF;
            }
            23 => self.lfo_inc |= 1,
            _ => {}
        }
        if self.mode_test_21[5] {
            self.eg_timer &= !(1u16 << self.eg_cycle);
        }
        if (self.eg_timer >> self.eg_cycle) & 1 != 0 && self.eg_cycle_stop {
            self.eg_shift = self.eg_cycle;
            self.eg_cycle_stop = false;
        }

        self.do_io();
        self.do_timer_a();
        self.do_timer_b();
        self.key_on();

        self.ch_output();
        self.ch_generate();

        self.fm_prepare();
        self.fm_generate();

        self.phase_generate();
        self.phase_calc_increment();

        self.envelope_adsr();
        self.envelope_generate();
        self.envelope_ssg_eg();
        self.envelope_prepare();

        // The frequency the phase generator works with next: channel 3's operators have their own in the special mode.
        let next = (self.channel + 1) % 6;
        let (fnum, block, kcode) = if self.mode_ch3 != 0 {
            match slot {
                1 => (self.fnum_3ch[1], self.block_3ch[1], self.kcode_3ch[1]),
                7 => (self.fnum_3ch[0], self.block_3ch[0], self.kcode_3ch[0]),
                13 => (self.fnum_3ch[2], self.block_3ch[2], self.kcode_3ch[2]),
                _ => (self.fnum[next], self.block[next], self.kcode[next]),
            }
        } else {
            (self.fnum[next], self.block[next], self.kcode[next])
        };
        self.pg_fnum = fnum;
        self.pg_block = block;
        self.pg_kcode = kcode;

        self.update_lfo();
        self.do_reg_write();
        self.cycles = (self.cycles + 1) % 24;
        self.channel = self.cycles % 6;

        self.status_time = self.status_time.saturating_sub(1);
        (self.mol, self.mor)
    }
}

/// Sign-extends the low `bits + 1` bits of `value`.
fn sign_extend(value: i16, bits: u32) -> i16 {
    let v = i32::from(value);
    let mask = (1i32 << bits) - 1;
    ((v & mask) - (v & (1 << bits))) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_samples(chip: &mut Ym2612, samples: usize) -> Vec<(i32, i32)> {
        (0..samples)
            .map(|_| {
                let (mut l, mut r) = (0i32, 0i32);
                for _ in 0..CYCLES_PER_SAMPLE {
                    let (a, b) = chip.clock();
                    l += i32::from(a);
                    r += i32::from(b);
                }
                (l, r)
            })
            .collect()
    }

    fn write(chip: &mut Ym2612, bank: u32, reg: u8, value: u8) {
        chip.write(bank * 2, reg);
        for _ in 0..40 {
            chip.clock();
        }
        chip.write(bank * 2 + 1, value);
        for _ in 0..80 {
            chip.clock();
        }
    }

    #[test]
    fn a_silent_chip_outputs_a_steady_value() {
        let mut chip = Ym2612::new(Chip::Ym3438);
        let s = run_samples(&mut chip, 4);
        assert!(s.iter().all(|x| *x == s[0]));
    }

    #[test]
    fn the_status_register_reports_timer_a_after_its_period() {
        let mut chip = Ym2612::new(Chip::Ym3438);
        // timer A: 1023 (the shortest period of one count), enabled and loaded
        write(&mut chip, 0, 0x24, 0xFF);
        write(&mut chip, 0, 0x25, 0x03);
        write(&mut chip, 0, 0x27, 0x05);
        run_samples(&mut chip, 4);
        assert_eq!(chip.read(0) & 1, 1, "timer A overflowed");
        assert!(chip.irq());
        // stop the timer and reset the flag
        write(&mut chip, 0, 0x27, 0x10);
        run_samples(&mut chip, 2);
        assert_eq!(chip.read(0) & 1, 0);
        assert!(!chip.irq());
    }

    #[test]
    fn a_key_on_note_makes_sound_and_key_off_ends_it() {
        let mut chip = Ym2612::new(Chip::Ym3438);
        // channel 1: algorithm 7, operator 1 audible: TL 0, AR 31, RR 15, frequency about 440 Hz
        write(&mut chip, 0, 0x30, 0x01);
        write(&mut chip, 0, 0x40, 0x00);
        write(&mut chip, 0, 0x50, 0x1F);
        write(&mut chip, 0, 0x60, 0x00);
        write(&mut chip, 0, 0x70, 0x00);
        write(&mut chip, 0, 0x80, 0x0F);
        write(&mut chip, 0, 0xB0, 0x07);
        write(&mut chip, 0, 0xB4, 0xC0);
        write(&mut chip, 0, 0xA4, 0x22);
        write(&mut chip, 0, 0xA0, 0x69);
        write(&mut chip, 0, 0x28, 0xF0);
        let s = run_samples(&mut chip, 400);
        let peak = s.iter().map(|x| x.0.abs()).max().unwrap();
        assert!(peak > 500, "audible: {peak}");
        write(&mut chip, 0, 0x28, 0x00);
        run_samples(&mut chip, 2000);
        let tail = run_samples(&mut chip, 100);
        let quiet = tail.iter().map(|x| x.0.abs()).max().unwrap();
        assert!(quiet < peak / 20, "released: {quiet} against {peak}");
    }

    #[test]
    fn the_note_of_a_frequency_number_follows_the_documented_logic() {
        let notes: Vec<u8> = (0..16).map(|i| note(i << 7)).collect();
        assert_eq!(notes, vec![0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 3, 3, 3, 3, 3, 3]);
    }
}
