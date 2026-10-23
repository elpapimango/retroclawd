//! The Mega ST's real-time clock, an RP5C15 at `$FFFC20-$FFFC3F` (`docs/hardware-facts.md`, "Mega ST real-time clock").
//!
//! The sixteen four-bit registers sit at the odd addresses `$FFFC21 + 2n`. [O] Hatari 2.6.1 (probe P32 and a scan of the
//! area): the even addresses read `$FF`, nothing bus-errors; in bank 0 registers 0-12 are the time (seconds, minutes, hours,
//! weekday, day, month, year since 1980, ones digit then tens digit) and read with the upper nibble 0; the mode register 13
//! keeps all four bits and reads with the upper nibble `$F`; the test and reset registers 14 and 15 keep four bits. [H?] The
//! rest is the chip's documented layout as remembered (no data sheet to hand): bit 0 of the mode register selects bank 1, the
//! alarm registers (minutes 2-3, hours 4-5, weekday 6, day 7-8) and the 12/24 hour select (register 10, bit 0 set = 24 hours)
//! live there and read with the upper nibble `$F` as Hatari's registers 2 and 3 do; the alarm output is not wired on the ST,
//! so alarms are stored and do nothing. Hatari's clock is the host's; this one counts machine time from the time the
//! plugin option `rtc_time` gives (or stands still, `rtc_mode = stopped`) and never reads a wall clock.

use serde::{Deserialize, Serialize};

const SECS_PER_DAY: u64 = 86_400;

/// Days from 1970-01-01 to a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count from 1970-01-01: (year, month, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Days from 1970-01-01 to 1980-01-01, the chip's year zero.
const DAYS_1980: i64 = 3652;

/// The clock fields: seconds, minutes, hours (24), day, month, year since 1980.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fields {
    sec: u64,
    min: u64,
    hour: u64,
    day: i64,
    month: i64,
    year: i64,
}

fn fields(secs: u64) -> Fields {
    let (days, rest) = ((secs / SECS_PER_DAY) as i64, secs % SECS_PER_DAY);
    let (y, m, d) = civil_from_days(days + DAYS_1980);
    Fields {
        sec: rest % 60,
        min: rest / 60 % 60,
        hour: rest / 3600,
        day: d,
        month: m,
        year: y - 1980,
    }
}

fn secs_of(f: Fields) -> u64 {
    let days = days_from_civil(f.year + 1980, f.month, f.day) - DAYS_1980;
    days as u64 * SECS_PER_DAY + f.hour * 3600 + f.min * 60 + f.sec
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year + 1980) % 4 == 0 && ((year + 1980) % 100 != 0 || (year + 1980) % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The last second the chip counts to: the end of 2079 (the tens digit of the year is four bits but the year goes to 99).
const MAX_SECS: u64 = 100 * 366 * SECS_PER_DAY;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rtc {
    /// The seconds since 1980-01-01 00:00:00 at CPU cycle `base_cycle`.
    base_secs: u64,
    base_cycle: u64,
    running: bool,
    mode: u8,
    test: u8,
    reset: u8,
    /// The alarm registers (bank 1), by register number.
    alarm: [u8; 13],
    hour24: bool,
    leap: u8,
}

impl Default for Rtc {
    fn default() -> Self {
        Self {
            base_secs: 0,
            base_cycle: 0,
            running: true,
            mode: 0,
            test: 0,
            reset: 0,
            alarm: [0; 13],
            hour24: true,
            leap: 0,
        }
    }
}

impl Rtc {
    /// Whether a state (untrusted) can have come from this chip.
    pub fn is_consistent(&self) -> bool {
        self.mode < 16
            && self.test < 16
            && self.reset < 16
            && self.leap < 4
            && self.alarm.iter().all(|&a| a < 16)
            && self.base_secs < MAX_SECS
    }

    /// Sets the clock to `secs` seconds after 1980-01-01 00:00:00 at cycle `now`; `running` false freezes it there.
    pub fn set_time(&mut self, secs: u64, now: u64, running: bool) {
        self.base_secs = secs.min(MAX_SECS - 1);
        self.base_cycle = now;
        self.running = running;
    }

    /// The seconds since 1980-01-01 00:00:00 at cycle `now`.
    pub fn seconds(&self, now: u64, clock_hz: u64) -> u64 {
        if !self.running {
            return self.base_secs;
        }
        let elapsed = now.saturating_sub(self.base_cycle) / clock_hz.max(1);
        (self.base_secs + elapsed) % MAX_SECS
    }

    fn bank1(&self) -> bool {
        self.mode & 1 != 0
    }

    /// The hour registers' value: 24 hours, or 12 with the PM flag in bit 1 of the tens digit.
    fn hour_digits(&self, hour: u64) -> (u8, u8) {
        if self.hour24 {
            ((hour % 10) as u8, (hour / 10) as u8)
        } else {
            let pm = u8::from(hour >= 12);
            let h = if hour.is_multiple_of(12) {
                12
            } else {
                hour % 12
            };
            ((h % 10) as u8, (h / 10) as u8 | pm << 1)
        }
    }

    /// Register `n` (0-15) as the byte the bus shows.
    fn read_reg(&self, n: usize, now: u64, clock_hz: u64) -> u8 {
        let f = fields(self.seconds(now, clock_hz));
        let weekday = ((secs_of(f) / SECS_PER_DAY + 2) % 7) as u8;
        let time = |n: usize| -> u8 {
            let (h1, h10) = self.hour_digits(f.hour);
            match n {
                0 => (f.sec % 10) as u8,
                1 => (f.sec / 10) as u8,
                2 => (f.min % 10) as u8,
                3 => (f.min / 10) as u8,
                4 => h1,
                5 => h10,
                6 => weekday,
                7 => (f.day % 10) as u8,
                8 => (f.day / 10) as u8,
                9 => (f.month % 10) as u8,
                10 => (f.month / 10) as u8,
                11 => (f.year % 10) as u8,
                _ => (f.year / 10) as u8,
            }
        };
        match n {
            13 => 0xF0 | self.mode,
            14 => self.test,
            15 => self.reset,
            0 | 1 | 9 | 12 if self.bank1() => time(n),
            2..=8 if self.bank1() => 0xF0 | self.alarm[n],
            10 if self.bank1() => 0xF0 | u8::from(self.hour24),
            11 if self.bank1() => 0xF0 | self.leap,
            _ => time(n),
        }
    }

    fn write_reg(&mut self, n: usize, v: u8, now: u64, clock_hz: u64) {
        let v = v & 0x0F;
        match n {
            13 => self.mode = v,
            14 => self.test = v,
            15 => self.reset = v,
            2..=8 if self.bank1() => self.alarm[n] = v,
            10 if self.bank1() => self.hour24 = v & 1 != 0,
            11 if self.bank1() => self.leap = v & 3,
            0 | 1 | 9 | 12 if self.bank1() => {}
            _ => self.write_time(n, v, now, clock_hz),
        }
    }

    /// A time digit written in bank 0: the field takes it when the result is a valid value, else the write is ignored
    /// ([H?]); the weekday is derived from the date and ignores writes.
    fn write_time(&mut self, n: usize, v: u8, now: u64, clock_hz: u64) {
        let mut f = fields(self.seconds(now, clock_hz));
        let v = i64::from(v);
        let digit = |old: i64, ones: bool| {
            if ones {
                old / 10 * 10 + v
            } else {
                old % 10 + v * 10
            }
        };
        match n {
            0 => f.sec = digit(f.sec as i64, true) as u64,
            1 => f.sec = digit(f.sec as i64, false) as u64,
            2 => f.min = digit(f.min as i64, true) as u64,
            3 => f.min = digit(f.min as i64, false) as u64,
            4 | 5 => {
                let (h1, h10) = self.hour_digits(f.hour);
                let (h1, h10) = if n == 4 {
                    (v as u8, h10)
                } else {
                    (h1, v as u8)
                };
                f.hour = if self.hour24 {
                    u64::from(h10) * 10 + u64::from(h1)
                } else {
                    let pm = u64::from(h10 >> 1 & 1);
                    (u64::from(h10 & 1) * 10 + u64::from(h1)) % 12 + 12 * pm
                };
            }
            7 => f.day = digit(f.day, true),
            8 => f.day = digit(f.day, false),
            9 => f.month = digit(f.month, true),
            10 => f.month = digit(f.month, false),
            11 => f.year = digit(f.year, true),
            12 => f.year = digit(f.year, false),
            _ => return,
        }
        let valid = f.sec < 60
            && f.min < 60
            && f.hour < 24
            && (1..=12).contains(&f.month)
            && (0..100).contains(&f.year)
            && f.day >= 1
            && f.day <= days_in_month(f.year, f.month);
        if valid {
            self.base_secs = secs_of(f);
            self.base_cycle = now;
        }
    }

    /// An access at `addr` (`$FFFC20-$FFFC3F`) as a byte or a word; a write returns the value written.
    pub fn access(
        &mut self,
        addr: u32,
        word: bool,
        write: Option<u32>,
        now: u64,
        clock_hz: u64,
    ) -> u32 {
        let reg_at =
            |a: u32| -> Option<usize> { (a & 1 == 1).then(|| ((a - 0xFF_FC21) / 2) as usize) };
        if word {
            // The even byte reads open; the odd byte is the register.
            let odd = addr + 1;
            return match write {
                Some(v) => {
                    if let Some(n) = reg_at(odd) {
                        self.write_reg(n, v as u8, now, clock_hz);
                    }
                    v
                }
                None => {
                    let lo = reg_at(odd).map_or(0xFF, |n| self.read_reg(n, now, clock_hz));
                    0xFF00 | u32::from(lo)
                }
            };
        }
        match (reg_at(addr), write) {
            (Some(n), Some(v)) => {
                self.write_reg(n, v as u8, now, clock_hz);
                v
            }
            (Some(n), None) => u32::from(self.read_reg(n, now, clock_hz)),
            (None, Some(v)) => v,
            (None, None) => 0xFF,
        }
    }
}

/// Parses `YYYY-MM-DD HH:MM:SS` (1980-2079) into seconds after 1980-01-01 00:00:00.
pub fn parse_time(text: &str) -> Result<u64, String> {
    let bad = || format!("rtc_time must be like 2026-10-23 08:43:55 (1980 to 2079), not {text:?}");
    let (date, time) = text.trim().split_once([' ', 'T']).ok_or_else(bad)?;
    let num = |s: &str| s.parse::<i64>().map_err(|_| bad());
    let d: Vec<i64> = date.split('-').map(num).collect::<Result<_, _>>()?;
    let t: Vec<i64> = time.split(':').map(num).collect::<Result<_, _>>()?;
    let [y, mo, da] = d[..] else {
        return Err(bad());
    };
    let [h, mi, s] = t[..] else { return Err(bad()) };
    let f = Fields {
        sec: u64::try_from(s).map_err(|_| bad())?,
        min: u64::try_from(mi).map_err(|_| bad())?,
        hour: u64::try_from(h).map_err(|_| bad())?,
        day: da,
        month: mo,
        year: y - 1980,
    };
    let ok = (1980..2080).contains(&y)
        && (1..=12).contains(&mo)
        && da >= 1
        && da <= days_in_month(f.year, mo)
        && f.hour < 24
        && f.min < 60
        && f.sec < 60;
    if ok { Ok(secs_of(f)) } else { Err(bad()) }
}
