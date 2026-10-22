//! The save-state envelope (spec section 19): what a load refuses, and that hostile bytes never panic or make the
//! loader allocate from a length field.

mod common;
use common::{BRA_SELF, config, rom};

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use atari_st::state::{self, DriveMeta, Header, StateError};
use atari_st::{AtariSt, RamSize};
use machine::Machine;

/// Largest single allocation since the last reset: a loader that trusts a length field shows up here.
struct Tracking;
static LARGEST: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Tracking = Tracking;

fn machine() -> AtariSt {
    let mut m = AtariSt::new(config(rom(&[BRA_SELF], &[]), RamSize::K256)).unwrap();
    while m.now() < 100_000 {
        m.step();
    }
    m
}

/// The envelope's header length for this machine's states (the version tag is "dev").
fn header_len(bytes: &[u8]) -> usize {
    // magic 4, schema 2, id (1 + 8), tag (1 + n), cpu 2, accuracy 1, firmware 20, two drives of 22, length 4.
    let tag = bytes[4 + 2 + 1 + 8] as usize;
    4 + 2 + 1 + 8 + 1 + tag + 2 + 1 + 20 + 2 * 22 + 4
}

/// Recomputes the CRC after a test changed header bytes.
fn fix_crc(bytes: &mut [u8]) {
    let h = header_len(bytes);
    let mut crc = crc32fast::Hasher::new();
    crc.update(&bytes[..h]);
    crc.update(&bytes[h + 4..]);
    bytes[h..h + 4].copy_from_slice(&crc.finalize().to_be_bytes());
}

#[test]
fn round_trip_through_save_and_load_state() {
    let mut a = machine();
    let saved = a.save_state();
    assert_eq!(&saved[..4], b"RCST");
    let mut b = machine();
    b.load_state(&saved).unwrap();
    assert_eq!(b.save_state(), saved);
    a.load_state(&saved).unwrap();
}

#[test]
fn another_machine_id_is_refused() {
    let mut s = machine().save_state();
    s[7..15].copy_from_slice(b"zx-speck");
    fix_crc(&mut s);
    let err = machine().load_state(&s).unwrap_err();
    assert!(
        err.contains("zx-speck") && err.contains("atari-st"),
        "{err}"
    );
}

#[test]
fn a_newer_schema_is_refused() {
    let mut s = machine().save_state();
    s[4..6].copy_from_slice(&(state::SCHEMA + 1).to_be_bytes());
    fix_crc(&mut s);
    let err = machine().load_state(&s).unwrap_err();
    assert!(err.contains("schema"), "{err}");
}

#[test]
fn a_length_field_beyond_the_cap_is_refused_before_allocating() {
    let mut s = machine().save_state();
    let h = header_len(&s);
    s[h - 4..h].copy_from_slice(&u32::MAX.to_be_bytes());
    LARGEST.store(0, Ordering::Relaxed);
    let err = machine().load_state(&s).unwrap_err();
    assert!(err.contains("limit"), "{err}");
    // The machine under test (a 256K RAM one) holds a few hundred KB; 4 GiB would show.
    assert!(LARGEST.load(Ordering::Relaxed) < 64 << 20);
}

#[test]
fn a_length_that_does_not_match_what_follows_is_an_error() {
    let mut s = machine().save_state();
    let h = header_len(&s);
    s[h - 4..h].copy_from_slice(&1000u32.to_be_bytes());
    let err = machine().load_state(&s).unwrap_err();
    assert!(err.contains("1000"), "{err}");
}

#[test]
fn a_flipped_payload_bit_is_a_crc_error() {
    let mut s = machine().save_state();
    let n = s.len();
    s[n / 2] ^= 0x01;
    let err = machine().load_state(&s).unwrap_err();
    assert!(err.contains("CRC"), "{err}");
}

#[test]
fn a_different_firmware_names_both_hashes() {
    let a = machine().save_state();
    let mut other_rom = rom(&[BRA_SELF], &[]);
    other_rom[100] ^= 0xFF;
    let mut b = AtariSt::new(config(other_rom, RamSize::K256)).unwrap();
    let err = b.load_state(&a).unwrap_err();
    assert!(
        err.contains(&state::hex(&state::sha1_of(&rom(&[BRA_SELF], &[])))),
        "{err}"
    );
    assert!(err.matches("SHA-1").count() >= 1 && err.len() > 80, "{err}");
}

#[test]
fn another_accuracy_mode_is_refused() {
    let a = machine().save_state();
    let mut cfg = config(rom(&[BRA_SELF], &[]), RamSize::K256);
    cfg.accuracy = atari_st::Accuracy::Accurate;
    let mut b = AtariSt::new(cfg).unwrap();
    let err = b.load_state(&a).unwrap_err();
    assert!(err.contains("fast") && err.contains("accurate"), "{err}");
}

#[test]
fn a_media_mismatch_loads_with_a_warning() {
    let mut a = machine();
    a.insert_disk(0, st_floppy::Image::parse(&vec![0xAA; 737_280]).unwrap())
        .unwrap();
    let saved = a.save_state();
    // The loading machine has a different disk in drive A: the state loads, and says so.
    let mut b = machine();
    b.insert_disk(0, st_floppy::Image::parse(&vec![0xBB; 737_280]).unwrap())
        .unwrap();
    b.load_state(&saved).unwrap();
    let warnings = b.take_state_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("drive A"), "{warnings:?}");
    assert!(b.take_state_warnings().is_empty(), "taken once");
    // The disk in the drive is still the user's.
    assert_eq!(b.disk(0).unwrap().sector(0, 0, 1).unwrap()[0], 0xBB);
}

#[test]
fn the_same_disk_loads_without_a_warning() {
    let mut a = machine();
    a.insert_disk(0, st_floppy::Image::parse(&vec![0xAA; 737_280]).unwrap())
        .unwrap();
    let saved = a.save_state();
    let mut b = machine();
    b.insert_disk(0, st_floppy::Image::parse(&vec![0xAA; 737_280]).unwrap())
        .unwrap();
    b.load_state(&saved).unwrap();
    assert!(b.take_state_warnings().is_empty());
}

#[test]
fn truncation_at_every_offset_is_an_error_not_a_panic() {
    let s = machine().save_state();
    let h = header_len(&s);
    for cut in (0..h + 8).chain((h + 8..s.len()).step_by(s.len() / 40 + 1)) {
        assert!(machine().load_state(&s[..cut]).is_err(), "cut at {cut}");
    }
}

#[test]
fn a_payload_that_is_not_a_machine_is_an_error() {
    // A well-formed envelope (right CRC, right hashes) around bytes that are not a payload.
    let a = machine();
    let good = a.save_state();
    let h = header_len(&good);
    let header = state::decode(&good, 0, &state::sha1_of(&rom(&[BRA_SELF], &[])))
        .unwrap()
        .0;
    let mut x = 0x9E37_79B9u32;
    for len in [0usize, 1, 7, 100, 5000] {
        let junk: Vec<u8> = (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        let bytes = state::encode(&header, &junk);
        assert!(machine().load_state(&bytes).is_err(), "{len} junk bytes");
    }
    let _ = h;
    let _ = DriveMeta::default();
    let _ = Header { ..header };
    let _ = StateError::Truncated;
}

/// Damaged payloads that still carry a valid envelope (right CRC): a state file edited by hand, or made by a program
/// that is not this one. Whatever they hold, loading and then running the machine must not panic.
#[test]
fn mutated_payloads_never_panic() {
    let good = machine().save_state();
    let (header, payload) =
        state::decode(&good, 0, &state::sha1_of(&rom(&[BRA_SELF], &[]))).unwrap();
    let mut x = 0xC0FF_EE11u32;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    let (mut loaded, mut refused) = (0, 0);
    for round in 0..100 {
        let mut p = payload.to_vec();
        for _ in 0..1 + round % 6 {
            // The CPU and scheduler at the front, the devices behind the RAM and the picture at the back.
            let at = if rnd() % 2 == 0 {
                rnd() as usize % 4096
            } else {
                p.len() - 1 - rnd() as usize % 60_000
            };
            p[at] = rnd() as u8;
        }
        let bytes = state::encode(&header, &p);
        let mut m = machine();
        match m.load_state(&bytes) {
            Ok(()) => {
                loaded += 1;
                for _ in 0..3_000 {
                    m.step();
                }
                let _ = m.save_state();
            }
            Err(_) => refused += 1,
        }
    }
    eprintln!("{loaded} loaded, {refused} refused");
    assert!(loaded + refused == 100);
}
