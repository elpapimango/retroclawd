//! The chipsets: ROM size and base, what decodes where, states that belong to one chipset.

mod common;
use common::{config, rom};

use atari_st::{AtariSt, Chipset, RamSize};
use machine::Machine;

/// A 256 KB image for the STE: SSP `$8000`, reset PC `$E00010`, `BRA.S *` there.
fn ste_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 262_144];
    rom[0..4].copy_from_slice(&0x8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x00E0_0010u32.to_be_bytes());
    rom[0x10..0x12].copy_from_slice(&0x60FEu16.to_be_bytes());
    rom
}

fn st_rom() -> Vec<u8> {
    rom(&[0x60FE], &[])
}

#[test]
fn ste_rom_is_mapped_at_e00000_with_the_reset_vector_mirrored() {
    let m = AtariSt::with_chipset(config(ste_rom(), RamSize::M1), Chipset::Ste).unwrap();
    assert_eq!(m.read_memory(0xE0_0010), 0x60);
    assert_eq!(m.read_memory(0xE3_FFFF), 0, "the last byte of the 256 KB");
    // `$0-$7` mirror the first 8 bytes of the ROM: SSP and the reset PC.
    assert_eq!(
        (0..8).map(|i| m.read_memory(i)).collect::<Vec<_>>(),
        vec![0, 0, 0x80, 0x00, 0, 0xE0, 0, 0x10]
    );
    assert_eq!(m.cpu().pc, 0xE0_0010, "the CPU starts in it");
    assert_eq!(m.read_memory(0xE4_0000), 0xFF, "nothing above the image");
    assert_eq!(
        m.read_memory(0xFC_0010),
        0xFF,
        "no ROM at $FC0000 on an STE"
    );
}

#[test]
fn an_st_and_a_mega_st_keep_the_192_kb_rom_at_fc0000() {
    for chipset in [Chipset::St, Chipset::MegaSt] {
        let m = AtariSt::with_chipset(config(st_rom(), RamSize::M1), chipset).unwrap();
        assert_eq!(m.read_memory(0xFC_0010), 0x60, "{chipset:?}");
        assert_eq!(
            m.read_memory(0xE0_0010),
            0xFF,
            "{chipset:?}: no ROM at $E00000"
        );
    }
}

#[test]
fn an_image_of_the_wrong_size_is_refused_naming_the_chipset() {
    let err = AtariSt::with_chipset(config(st_rom(), RamSize::M1), Chipset::Ste)
        .err()
        .unwrap();
    assert!(err.contains("atari-ste") && err.contains("262144"), "{err}");
    let err = AtariSt::with_chipset(config(ste_rom(), RamSize::M1), Chipset::MegaSt)
        .err()
        .unwrap();
    assert!(
        err.contains("atari-mega-st") && err.contains("196608"),
        "{err}"
    );
}

#[test]
fn a_state_loads_only_into_the_same_chipset() {
    let st = AtariSt::with_chipset(config(st_rom(), RamSize::M1), Chipset::St).unwrap();
    let state = st.save_state_checked().unwrap();
    let mut mega = AtariSt::with_chipset(config(st_rom(), RamSize::M1), Chipset::MegaSt).unwrap();
    let err = mega.load_state_checked(&state).unwrap_err().to_string();
    assert!(err.contains("Mega ST"), "{err}");
    let mut same = AtariSt::with_chipset(config(st_rom(), RamSize::M1), Chipset::St).unwrap();
    same.load_state_checked(&state).unwrap();
}

#[test]
fn the_chipset_ids_are_the_machine_ids() {
    for (chipset, id) in [
        (Chipset::St, "atari-st"),
        (Chipset::MegaSt, "atari-mega-st"),
        (Chipset::Ste, "atari-ste"),
    ] {
        assert_eq!(chipset.id(), id);
        assert_eq!(Chipset::from_id(&id.to_uppercase()), Some(chipset));
    }
    assert_eq!(Chipset::from_id("atari-tt"), None);
}

/// The STE's registers are in its state, an ST's state has no chipset part, and a hostile chipset part is refused.
#[test]
fn an_ste_state_carries_the_ste_registers() {
    let mut image = ste_rom();
    let program: [u16; 7] = [0x11FC, 0x005A, 0x820D, 0x11FC, 0x0003, 0x820F, 0x60FE];
    for (i, w) in program.iter().enumerate() {
        image[0x10 + 2 * i..0x12 + 2 * i].copy_from_slice(&w.to_be_bytes());
    }
    let mut a = AtariSt::with_chipset(config(image.clone(), RamSize::M1), Chipset::Ste).unwrap();
    for _ in 0..50 {
        a.step();
    }
    assert_eq!(a.bus().shifter().ste_state().unwrap().base_low, 0x5A);
    let state = a.save_state_checked().unwrap();
    let mut b = AtariSt::with_chipset(config(image.clone(), RamSize::M1), Chipset::Ste).unwrap();
    b.load_state_checked(&state).unwrap();
    let ste = b.bus().shifter().ste_state().unwrap();
    assert_eq!((ste.base_low, ste.line_offset), (0x5A, 3));
    // Cutting the chipset part short, or adding bytes to an ST's state, is refused.
    let mut short = state.clone();
    short.truncate(short.len() - 40);
    assert!(b.load_state_checked(&short).is_err());
}
