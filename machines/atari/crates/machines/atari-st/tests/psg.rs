//! The YM2149 at `$FF8800` (facts: `docs/hardware-facts.md`, "PSG"): register select and data, port A, audio.

mod common;
use common::{BRA_SELF, RAM_SIZES, config, move_b, rom};

use atari_st::{AtariSt, RamSize, StBus};
use machine::Machine;
use mc68000::{BusFault, FunctionCode, M68kBus, Size};

const SUPER: FunctionCode = FunctionCode::SUPERVISOR_DATA;
const USER: FunctionCode = FunctionCode::USER_DATA;

fn bus() -> StBus {
    StBus::new(&config(rom(&[BRA_SELF], &[]), RamSize::M1)).unwrap()
}

fn psg_write(b: &mut StBus, reg: u8, value: u8) {
    b.write(0xFF8800, Size::Byte, reg as u32, SUPER).unwrap();
    b.write(0xFF8802, Size::Byte, value as u32, SUPER).unwrap();
}

#[test]
fn select_write_read_back() {
    let mut b = bus();
    psg_write(&mut b, 0, 0x34);
    assert_eq!(b.read(0xFF8800, Size::Byte, SUPER), Ok(0x34));
    assert!(b.stop().is_none());
}

/// YM2149 catalog Table 3: R1 (coarse tone A) has four bits.
#[test]
fn r1_reads_four_bits() {
    let mut b = bus();
    psg_write(&mut b, 1, 0xFF);
    assert_eq!(b.read(0xFF8800, Size::Byte, SUPER), Ok(0x0F));
}

/// R7 bits 7-6 set both ports to output; port A then reads back its latch, and drives the ST's control lines.
#[test]
fn port_a_latch_and_direction() {
    let mut b = bus();
    psg_write(&mut b, 7, 0xC0);
    psg_write(&mut b, 14, 0x07);
    assert_eq!(b.psg().port_a(), 0x07);
    assert_eq!(b.read(0xFF8800, Size::Byte, SUPER), Ok(0x07));
    psg_write(&mut b, 7, 0x00);
    assert_eq!(
        b.psg().port_a(),
        0xFF,
        "an input port drives nothing: the lines float high"
    );
}

/// Internals p.60: "Attempting to read this address after writing to it will give you $FF only".
#[test]
fn ff8802_reads_ff() {
    let mut b = bus();
    psg_write(&mut b, 0, 0x12);
    assert_eq!(b.read(0xFF8802, Size::Byte, SUPER), Ok(0xFF));
}

#[test]
fn user_mode_is_bus_error() {
    let mut b = bus();
    assert_eq!(b.read(0xFF8800, Size::Byte, USER), Err(BusFault));
}

/// The S4 shape: tone A at period 284, full level, then STOP. Over 50 frames the DC-free output crosses zero twice
/// per tone period: the YM's clock is the CPU clock / 4 and `fT = fMaster / (16 TP)`.
#[test]
fn tone_a_from_bare_metal() {
    let mut program = Vec::new();
    for (reg, value) in [(0u8, 284u16 as u8), (1, 1), (7, 0x3E), (8, 0x0F)] {
        program.extend(move_b(reg, 0x8800));
        program.extend(move_b(value, 0x8802));
    }
    program.extend([0x4E72, 0x2700, 0x60FA]); // STOP #$2700; BRA.S back to the STOP
    let mut m = AtariSt::new(config(rom(&program, &[]), RamSize::M1)).unwrap();
    let mut audio = Vec::new();
    while m.vbl_count() < 50 {
        m.step();
        if m.vbl_count().is_multiple_of(10) {
            audio.extend(m.drain_audio());
        }
    }
    audio.extend(m.drain_audio());
    let mean = audio.iter().sum::<f32>() / audio.len() as f32;
    let crossings = audio
        .windows(2)
        .filter(|w| (w[0] - mean).signum() != (w[1] - mean).signum())
        .count() as i64;
    let expected = (2 * m.now() / (4 * 16 * 284)) as i64;
    assert!(
        (crossings - expected).abs() <= 2,
        "{crossings} crossings, expected {expected}"
    );
}

/// Samples come from cycles: after any run, the samples drained are `floor(now × 44 100 / clock)`.
#[test]
fn audio_sample_count_is_exact() {
    let mut m = AtariSt::new(config(rom(&[BRA_SELF], &[]), RamSize::M1)).unwrap();
    let mut total = 0u64;
    while m.vbl_count() < 7 {
        m.step();
        if m.now().is_multiple_of(3) {
            total += m.drain_audio().len() as u64;
        }
    }
    total += m.drain_audio().len() as u64;
    assert_eq!(
        total,
        (m.now() as u128 * machine::AUDIO_SAMPLE_RATE as u128 / m.clock_hz() as u128) as u64
    );
}

/// Slice 3's end: EmuTOS's accesses up to its first keyboard ACIA write equal Hatari's, for every RAM size.
#[test]
fn emutos_matches_hatari_up_to_the_keyboard_acia() {
    let Some(rom) = common::emutos() else { return };
    for (name, ram) in RAM_SIZES {
        let a = common::run_until_stop(rom.clone(), ram);
        let ours = common::io_lines(&a);
        assert_eq!(
            ours[109], "IO write.b $fffffc00 = $03 pc=fc5160",
            "--ram {name}"
        );
        if let Some(reference) = common::hatari_io(name) {
            common::assert_prefix(&ours[..110], &reference, name);
            eprintln!("--ram {name}: 110 I/O accesses compared with Hatari");
        }
    }
}
