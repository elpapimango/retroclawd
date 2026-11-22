//! The Mega ST's clock: BCD digits as the registers show them, counting machine time, writes, banks and the plugin options.
//! (The registers Hatari keeps are compared in `timing_probes.rs`, P32; its time registers follow the host.)

use atari_st::rtc::{Rtc, parse_time};
mod common;

const HZ: u64 = 8_021_247;

fn reg(addr_reg: u32) -> u32 {
    0xFF_FC21 + 2 * addr_reg
}

fn digits(rtc: &mut Rtc, now: u64) -> Vec<u8> {
    (0..13)
        .map(|r| rtc.access(reg(r), false, None, now, HZ) as u8)
        .collect()
}

fn at(rtc: &mut Rtc, text: &str) -> Rtc {
    rtc.set_time(parse_time(text).unwrap(), 0, true);
    rtc.clone()
}

#[test]
fn the_registers_show_the_time_in_digits_as_hatari_showed_the_host_s() {
    let mut rtc = Rtc::default();
    at(&mut rtc, "2026-10-23 08:43:55");
    // seconds 5 5, minutes 3 4, hours 8 0, a Friday, day 3 2, month 0 1, year 6 4: the bytes of the scan of Hatari's clock
    assert_eq!(digits(&mut rtc, 0), [5, 5, 3, 4, 8, 0, 5, 3, 2, 0, 1, 6, 4]);
}

#[test]
fn the_clock_counts_machine_time_and_carries_through_the_calendar() {
    let mut rtc = Rtc::default();
    at(&mut rtc, "2028-02-28 23:59:59");
    assert_eq!(digits(&mut rtc, HZ)[..2], [0, 0], "one second on: 00");
    // a leap year: the 29th exists, then March
    let d = digits(&mut rtc, HZ);
    assert_eq!((d[7], d[8], d[9], d[10]), (9, 2, 2, 0), "2028-02-29");
    let d = digits(&mut rtc, HZ + 86_400 * HZ);
    assert_eq!((d[7], d[8], d[9], d[10]), (1, 0, 3, 0), "2028-03-01");
    let mut rtc = Rtc::default();
    at(&mut rtc, "2026-12-31 23:59:59");
    let d = digits(&mut rtc, HZ);
    assert_eq!(
        (d[11], d[12], d[9], d[10], d[7], d[8]),
        (7, 4, 1, 0, 1, 0),
        "2027-01-01"
    );
    // not a leap year
    let mut rtc = Rtc::default();
    at(&mut rtc, "2026-02-28 23:59:59");
    let d = digits(&mut rtc, HZ);
    assert_eq!((d[7], d[8], d[9], d[10]), (1, 0, 3, 0), "2026-03-01");
}

#[test]
fn a_stopped_clock_stands_still() {
    let mut rtc = Rtc::default();
    rtc.set_time(parse_time("2001-02-03 04:05:06").unwrap(), 0, false);
    assert_eq!(digits(&mut rtc, 0), digits(&mut rtc, 1000 * HZ));
}

#[test]
fn even_addresses_read_open_and_words_read_the_odd_register() {
    let mut rtc = Rtc::default();
    at(&mut rtc, "2026-10-23 08:43:55");
    assert_eq!(rtc.access(0xFF_FC20, false, None, 0, HZ), 0xFF);
    assert_eq!(rtc.access(0xFF_FC20, true, None, 0, HZ), 0xFF05);
    assert_eq!(
        rtc.access(0xFF_FC3A, true, None, 0, HZ),
        0xFFF0,
        "the mode register with its upper nibble set"
    );
}

#[test]
fn the_mode_test_and_reset_registers_keep_four_bits_and_the_alarm_bank_shows_with_bit_one_set() {
    let mut rtc = Rtc::default();
    for v in 0..16u32 {
        rtc.access(reg(13), false, Some(v), 0, HZ);
        // [D] bit 1 of the mode register is not implemented: it reads 0 (Hatari keeps it)
        assert_eq!(rtc.access(reg(13), false, None, 0, HZ), 0xF0 | v & !2);
    }
    rtc.access(reg(14), false, Some(0xFF), 0, HZ);
    assert_eq!(rtc.access(reg(14), false, None, 0, HZ), 0x0F);
    // bank 1: the alarm minutes read back with the upper nibble set, as Hatari's registers 2 and 3 do
    rtc.access(reg(13), false, Some(1), 0, HZ);
    for (v, want) in [(0x0F, 0xFF), (0, 0xF0), (5, 0xF5)] {
        rtc.access(reg(2), false, Some(v), 0, HZ);
        assert_eq!(rtc.access(reg(2), false, None, 0, HZ), want);
    }
    // the time is untouched by alarm writes
    rtc.access(reg(13), false, Some(0), 0, HZ);
    assert_eq!(rtc.access(reg(2), false, None, 0, HZ), 0);
}

#[test]
fn writing_the_time_changes_it_and_an_invalid_digit_is_ignored() {
    let mut rtc = Rtc::default();
    at(&mut rtc, "2026-10-23 08:43:55");
    // set the minutes to 59 and then try 6x: ignored
    rtc.access(reg(2), false, Some(9), 0, HZ);
    rtc.access(reg(3), false, Some(5), 0, HZ);
    assert_eq!(digits(&mut rtc, 0)[2..4], [9, 5]);
    rtc.access(reg(3), false, Some(6), 0, HZ);
    assert_eq!(
        digits(&mut rtc, 0)[2..4],
        [9, 5],
        "a minute of 69 is refused"
    );
    // the day cannot be 31 in a 30-day month
    rtc.access(reg(9), false, Some(9), 0, HZ);
    assert_eq!(
        digits(&mut rtc, 0)[7..9],
        [3, 2],
        "month 9 with day 23 is fine"
    );
    rtc.access(reg(8), false, Some(3), 0, HZ);
    assert_eq!(digits(&mut rtc, 0)[7..9], [3, 2], "day 33 refused");
    // the clock runs on from what was written: 55 s and ten more is 05
    assert_eq!(digits(&mut rtc, 10 * HZ)[..2], [5, 0]);
}

#[test]
fn twelve_hour_mode_shows_the_pm_flag() {
    let mut rtc = Rtc::default();
    at(&mut rtc, "2026-10-23 15:00:00");
    rtc.access(reg(13), false, Some(1), 0, HZ);
    rtc.access(reg(10), false, Some(0), 0, HZ); // 12 hours
    rtc.access(reg(13), false, Some(0), 0, HZ);
    let d = digits(&mut rtc, 0);
    assert_eq!(
        (d[4], d[5]),
        (3, 2),
        "3 PM: ones digit 3, tens digit 0 with the PM flag"
    );
}

#[test]
fn rtc_time_is_parsed_or_refused_with_the_reason() {
    assert_eq!(parse_time("1980-01-01 00:00:00"), Ok(0));
    assert_eq!(parse_time("1980-01-02T00:00:01"), Ok(86_401));
    for bad in [
        "",
        "1979-12-31 23:59:59",
        "2080-01-01 00:00:00",
        "2026-02-30 00:00:00",
        "2026-10-23 24:00:00",
        "yesterday",
        "2026-10-23",
    ] {
        assert!(parse_time(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_hostile_state_is_recognised() {
    let mut rtc = Rtc::default();
    assert!(rtc.is_consistent());
    rtc.set_time(u64::MAX, 0, true);
    assert!(rtc.is_consistent(), "the time is clamped");
}

/// EmuTOS on a Mega ST (needs `RC_EMUTOS`, 192K): it finds the clock, reads it and boots to the desktop with no stop, and the
/// time it reads is the one the machine was given.
#[test]
fn emutos_reads_the_clock_it_was_given() {
    use atari_st::{AtariSt, Chipset, RamSize};
    use machine::Machine;
    let Some(rom) = common::emutos() else {
        return;
    };
    let mut m = AtariSt::with_chipset(common::config(rom, RamSize::M1), Chipset::MegaSt).unwrap();
    m.set_rtc(parse_time("2001-02-03 04:05:06").unwrap(), false)
        .unwrap();
    while m.vbl_count() < 300 && m.io_stop().is_none() {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
    let reads: Vec<_> = m
        .io_log()
        .iter()
        .filter(|a| (0xFF_FC20..0xFF_FC40).contains(&a.addr) && !a.write)
        .collect();
    assert!(!reads.is_empty(), "EmuTOS never read the clock");
    // EmuTOS probes the alarm bank first (it writes and reads back registers 2 and 3 there); the last reads of the time
    // registers are those of the time given (seconds 6, minutes 5, hours 4, a Saturday)
    let value = |reg: u32| {
        reads
            .iter()
            .rev()
            .find(|a| a.addr == 0xFF_FC21 + 2 * reg)
            .map(|a| a.value & 0xF)
    };
    for (r, want) in [(0, 6), (2, 5), (4, 4), (6, 6), (11, 1), (12, 2)] {
        assert_eq!(value(r), Some(want), "register {r}");
    }
}

fn write(rtc: &mut Rtc, r: u32, v: u32, now: u64) {
    rtc.access(reg(r), false, Some(v), now, HZ);
}

fn read(rtc: &mut Rtc, r: u32, now: u64) -> u32 {
    rtc.access(reg(r), false, None, now, HZ)
}

#[test]
fn clearing_timer_en_holds_the_clock_and_setting_it_carries_on_from_the_same_part_of_the_second() {
    let mut rtc = Rtc::default();
    at(&mut rtc, "2026-10-23 08:43:55");
    write(&mut rtc, 13, 8, 0);
    // 0.75 s in: still second 55; clear Timer EN, wait ten seconds, set it again
    let t0 = HZ * 3 / 4;
    write(&mut rtc, 13, 0, t0);
    let t1 = t0 + 10 * HZ;
    assert_eq!(read(&mut rtc, 0, t1), 5, "held at 55");
    write(&mut rtc, 13, 8, t1);
    // a quarter of a second after the restart the held three quarters are complete: second 56
    assert_eq!(read(&mut rtc, 0, t1 + (HZ - t0) - 1) & 0xF, 5);
    assert_eq!(read(&mut rtc, 0, t1 + (HZ - t0)) & 0xF, 6);
}

#[test]
fn a_clock_the_option_stopped_starts_when_timer_en_is_set() {
    let mut rtc = Rtc::default();
    rtc.set_time(parse_time("2001-02-03 04:05:06").unwrap(), 0, false);
    write(&mut rtc, 13, 8, 100);
    assert_eq!(read(&mut rtc, 0, 100 + HZ) & 0xF, 7);
}

#[test]
fn adjust_rounds_the_seconds_to_the_minute() {
    for (from, to) in [
        ("2026-10-23 08:43:29", "2026-10-23 08:43:00"),
        ("2026-10-23 08:43:30", "2026-10-23 08:44:00"),
        ("2026-10-23 23:59:45", "2026-10-24 00:00:00"),
    ] {
        let mut rtc = Rtc::default();
        at(&mut rtc, from);
        write(&mut rtc, 13, 9, 0); // Timer EN and bank 1
        write(&mut rtc, 1, 1, 100);
        write(&mut rtc, 13, 8, 100);
        let mut want = Rtc::default();
        at(&mut want, to);
        assert_eq!(digits(&mut rtc, 100), digits(&mut want, 0), "{from}");
    }
}

#[test]
fn the_reset_register_clears_the_alarm_registers_and_restarts_the_divider() {
    let mut rtc = Rtc::default();
    at(&mut rtc, "2026-10-23 08:43:55");
    write(&mut rtc, 13, 9, 0);
    write(&mut rtc, 2, 0x0A, 0);
    write(&mut rtc, 7, 0x03, 0);
    write(&mut rtc, 15, 1, 0);
    assert_eq!(read(&mut rtc, 2, 0), 0xF0);
    assert_eq!(read(&mut rtc, 7, 0), 0xF0);
    // three quarters of a second in, the divider restarts: the next second is a whole second later
    write(&mut rtc, 13, 8, 0);
    write(&mut rtc, 15, 2, HZ * 3 / 4);
    assert_eq!(read(&mut rtc, 0, HZ * 3 / 4 + HZ - 1) & 0xF, 5);
    assert_eq!(read(&mut rtc, 0, HZ * 3 / 4 + HZ) & 0xF, 6);
}

#[test]
fn the_leap_year_counter_follows_the_year_and_can_be_set() {
    let mut rtc = Rtc::default();
    at(&mut rtc, "2028-02-28 12:00:00");
    write(&mut rtc, 13, 9, 0);
    assert_eq!(
        read(&mut rtc, 11, 0),
        0xF0,
        "2028 is a leap year: counter 0"
    );
    // a year later the counter is 1
    assert_eq!(read(&mut rtc, 11, 366 * 86_400 * HZ), 0xF1);
    write(&mut rtc, 11, 3, 0);
    assert_eq!(read(&mut rtc, 11, 0), 0xF3);
    assert_eq!(read(&mut rtc, 11, 366 * 86_400 * HZ), 0xF0);
}

#[test]
fn bank_1_registers_without_a_register_read_zero_and_the_digit_registers_keep_their_bits() {
    let mut rtc = Rtc::default();
    at(&mut rtc, "2026-10-23 08:43:55");
    write(&mut rtc, 13, 9, 0);
    for r in [1, 9, 12] {
        write(&mut rtc, r, 0, 0);
        assert_eq!(read(&mut rtc, r, 0), 0xF0, "register {r}");
    }
    write(&mut rtc, 0, 0xF, 0);
    assert_eq!(
        read(&mut rtc, 0, 0),
        0xF7,
        "the clock output select has three bits"
    );
    for (r, bits) in [(2, 4), (3, 3), (4, 4), (5, 2), (6, 3), (7, 4), (8, 2)] {
        write(&mut rtc, r, 0xF, 0);
        assert_eq!(
            read(&mut rtc, r, 0),
            0xF0 | ((1 << bits) - 1),
            "register {r}"
        );
    }
}
