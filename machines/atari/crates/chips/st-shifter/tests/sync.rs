//! The sync-mode and resolution write log: where in the frame the program touched `$FF820A` and `$FF8260`. A log only:
//! the picture ignores it (no overscan border is opened, Phase 3).

use st_shifter::{MAX_SYNC_LOG, Monitor, Shifter, SyncRegister, SyncWrite};

fn log(s: &Shifter) -> Vec<SyncWrite> {
    s.sync_log().copied().collect()
}

#[test]
fn sync_writes_are_logged_with_line_and_cycle() {
    let mut s = Shifter::new(true);
    s.begin_frame();
    s.set_line(100);
    s.set_line_start(1_000_000);
    s.write_at(0xFF820A, false, 0x00, 1_000_372);
    s.write_at(0xFF8260, false, 0x01, 1_000_400);
    s.set_line(101);
    s.set_line_start(1_000_512);
    s.write_at(0xFF820A, false, 0x02, 1_000_520);
    assert_eq!(
        log(&s),
        vec![
            SyncWrite {
                line: 100,
                cycle: 372,
                register: SyncRegister::Sync,
                value: 0
            },
            SyncWrite {
                line: 100,
                cycle: 400,
                register: SyncRegister::Resolution,
                value: 1
            },
            SyncWrite {
                line: 101,
                cycle: 8,
                register: SyncRegister::Sync,
                value: 2
            },
        ]
    );
    // The write still takes effect.
    assert!(s.sync_50hz());
}

#[test]
fn other_registers_are_not_logged() {
    let mut s = Shifter::new(true);
    s.write_at(0xFF8240, true, 0x0777, 10);
    s.write_at(0xFF8201, false, 0x10, 10);
    s.write_at(0xFF820B, false, 0x10, 10);
    s.write_at(0xFF8261, false, 0x10, 10);
    assert_eq!(log(&s), vec![]);
}

#[test]
fn the_log_is_bounded() {
    let mut s = Shifter::new(true);
    for i in 0..(MAX_SYNC_LOG as u64 + 44) {
        s.write_at(0xFF820A, false, (i & 3) as u16, i);
    }
    let l = log(&s);
    assert_eq!(l.len(), MAX_SYNC_LOG);
    assert_eq!(l[0].cycle, 44, "the oldest 44 were dropped");
    assert_eq!(l.last().unwrap().cycle, MAX_SYNC_LOG as u32 + 43);
}

#[test]
fn a_new_frame_empties_the_log() {
    let mut s = Shifter::new(true);
    s.write_at(0xFF820A, false, 0, 5);
    s.begin_frame();
    assert_eq!(log(&s), vec![]);
}

/// The picture is the same with or without the log: a program that flips the sync mode and the resolution "at the
/// right cycle" and flips them back gets the picture of the registers as they end the frame.
#[test]
fn the_picture_ignores_the_log() {
    let ram = |a: u32| (a.wrapping_mul(131) >> 2) as u8;
    let mut plain = Shifter::new(true);
    plain.write(0xFF8240, true, 0x0123);
    let mut logged = plain.clone();
    logged.set_line(200);
    logged.set_line_start(0);
    for _ in 0..3 {
        logged.write_at(0xFF820A, false, 0x00, 400);
        logged.write_at(0xFF820A, false, 0x02, 410);
        logged.write_at(0xFF8260, false, 0x02, 420);
        logged.write_at(0xFF8260, false, 0x00, 430);
    }
    assert!(logged.sync_log().count() > 0);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    plain.render(Monitor::Colour, &ram, &mut a);
    logged.render(Monitor::Colour, &ram, &mut b);
    assert_eq!(a, b);
}
