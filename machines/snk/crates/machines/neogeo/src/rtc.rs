//! The calendar chip (uPD4990A) as the BIOS drives it through REG_RTCCTRL and REG_STATUS_A (fact N48): a command of four bits is
//! clocked in (data on bit 0, clock on bit 1, strobe on bit 2 of the control register), the time register is loaded by the command
//! "time read" and its bits come out one clock at a time on the data-out line.

/// Seconds, minutes, hours, days, weekday, month and a two-digit year, all as the chip holds them: binary coded decimal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: u8,
    pub month: u8,
    pub day: u8,
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

fn from_bcd(v: u8) -> u8 {
    (v >> 4) * 10 + (v & 0xF)
}

fn bcd(v: u32) -> u8 {
    (((v / 10) << 4) | (v % 10)) as u8
}

fn days_in(month: u32, year: u32) -> u32 {
    match month {
        2 => {
            if year.is_multiple_of(4) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A date and time that is a number of whole seconds after `start` (years 2000 to 2099, a two-digit year).
pub fn add_seconds(start: (u32, u32, u32, u32, u32, u32), weekday0: u32, seconds: u64) -> DateTime {
    let (mut y, mut mo, mut d, mut h, mut mi, mut s) = start;
    let total = u64::from(s) + u64::from(mi) * 60 + u64::from(h) * 3600 + seconds;
    let days = total / 86_400;
    let rest = total % 86_400;
    s = (rest % 60) as u32;
    mi = ((rest / 60) % 60) as u32;
    h = (rest / 3600) as u32;
    for _ in 0..days {
        d += 1;
        if d > days_in(mo, 2000 + y) {
            d = 1;
            mo += 1;
            if mo > 12 {
                mo = 1;
                y = (y + 1) % 100;
            }
        }
    }
    DateTime {
        year: bcd(y),
        month: bcd(mo),
        day: bcd(d),
        weekday: ((weekday0 + (days % 7) as u32) % 7) as u8,
        hour: bcd(h),
        minute: bcd(mi),
        second: bcd(s),
    }
}

/// The chip's serial side.
#[derive(Default)]
pub struct Rtc {
    command: u8,
    bits_in: u8,
    shifting: bool,
    register: u64,
    last_clock: bool,
    last_strobe: bool,
    /// The level on the data-out line.
    pub data_out: bool,
}

impl Rtc {
    /// A write to REG_RTCCTRL; `now` is the time to load for a "time read" command.
    pub fn write(&mut self, value: u8, now: &DateTime) {
        let (data, clock, strobe) = (value & 1 != 0, value & 2 != 0, value & 4 != 0);
        if clock && !self.last_clock {
            if self.shifting {
                // the register moves one bit toward the data-out line (fact N48)
                self.register >>= 1;
                self.data_out = self.register & 1 != 0;
            } else {
                self.command = (self.command >> 1) | u8::from(data) << 3;
                self.bits_in = (self.bits_in + 1).min(4);
            }
        }
        if strobe && !self.last_strobe {
            self.execute(now);
        }
        self.last_clock = clock;
        self.last_strobe = strobe;
    }

    fn execute(&mut self, now: &DateTime) {
        match self.command & 0xF {
            // register hold
            0 => self.shifting = false,
            // register shift: the clock now moves the time register
            1 => {
                self.shifting = true;
                self.data_out = self.register & 1 != 0;
            }
            // time read: the current time is loaded; the register stops shifting until told to
            3 => {
                self.register = pack(now);
                self.shifting = false;
                self.data_out = self.register & 1 != 0;
            }
            // the time pulse and interval commands change nothing that is modelled
            _ => {}
        }
        self.bits_in = 0;
        self.command = 0;
    }
}

/// The 48 bits of the time register, the first to come out in bit 0: second, minute, hour, day (each a byte of two BCD digits, the
/// units first), the weekday, the month and the year.
fn pack(t: &DateTime) -> u64 {
    u64::from(t.second)
        | u64::from(t.minute) << 8
        | u64::from(t.hour) << 16
        | u64::from(t.day) << 24
        | u64::from(t.weekday & 0xF) << 32
        | u64::from(from_bcd(t.month)) << 36
        | u64::from(t.year) << 40
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime {
        DateTime {
            year: 0x26,
            month: 0x10,
            day: 0x31,
            weekday: 6,
            hour: 0x12,
            minute: 0x34,
            second: 0x56,
        }
    }

    fn command(rtc: &mut Rtc, c: u8) {
        for i in 0..4 {
            let d = (c >> i) & 1;
            rtc.write(d, &now());
            rtc.write(d | 2, &now());
            rtc.write(d, &now());
        }
        rtc.write(4, &now());
        rtc.write(0, &now());
    }

    fn clock(rtc: &mut Rtc) {
        rtc.write(2, &now());
        rtc.write(0, &now());
    }

    #[test]
    fn a_time_read_then_a_shift_clocks_the_register_out_second_first() {
        let mut rtc = Rtc::default();
        command(&mut rtc, 3);
        command(&mut rtc, 1);
        let mut bits = Vec::new();
        for _ in 0..48 {
            bits.push(rtc.data_out);
            clock(&mut rtc);
        }
        let byte = |from: usize| (0..8).fold(0u8, |v, i| v | u8::from(bits[from + i]) << i);
        assert_eq!(
            [byte(0), byte(8), byte(16), byte(24)],
            [0x56, 0x34, 0x12, 0x31]
        );
        let nibble = |from: usize| (0..4).fold(0u8, |v, i| v | u8::from(bits[from + i]) << i);
        // the month is a plain number in four bits (the chip's way: 10 is 0xA)
        assert_eq!([nibble(32), nibble(36), byte(40)], [6, 0xA, 0x26]);
    }

    #[test]
    fn seconds_added_roll_the_date_over() {
        // 2000-12-31 23:59:59 plus a second is the first of January 2001; 1 January 2000 was a Saturday (6)
        let t = add_seconds((0, 12, 31, 23, 59, 59), 0, 1);
        assert_eq!(
            (t.year, t.month, t.day, t.hour, t.minute, t.second),
            (1, 1, 1, 0, 0, 0)
        );
        assert_eq!(t.weekday, 1);
        // 2000 is a leap year: the 28th of February plus a day is the 29th
        let t = add_seconds((0, 2, 28, 0, 0, 0), 0, 86_400);
        assert_eq!((t.month, t.day), (2, 0x29));
    }
}
