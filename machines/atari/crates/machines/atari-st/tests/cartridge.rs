//! The cartridge slot at `$FA0000-$FBFFFF` (*Internals* p.96: up to 128K, read only: "a write access to these
//! addresses creates a bus error. The GLUE takes care of this.").

mod common;
use common::{BRA_SELF, config, move_b, rom};

use atari_st::{AtariSt, CARTRIDGE_BYTES, RamSize, StBus};
use machine::Machine;
use mc68000::{BusFault, FunctionCode, M68kBus, Size};

const SUPER: FunctionCode = FunctionCode::SUPERVISOR_DATA;
const USER: FunctionCode = FunctionCode::USER_DATA;

fn bus() -> StBus {
    StBus::new(&config(rom(&[BRA_SELF], &[]), RamSize::M1)).unwrap()
}

/// An image starting with the application header TOS scans for (`$ABCDEF42`, *Internals* ROM listing p.273).
fn image(len: usize) -> Vec<u8> {
    let mut v: Vec<u8> = (0..len).map(|i| i as u8).collect();
    v[..4].copy_from_slice(&0xABCD_EF42u32.to_be_bytes());
    v
}

#[test]
fn cartridge_is_visible_at_fa0000() {
    let mut b = bus();
    b.set_cartridge(Some(image(CARTRIDGE_BYTES))).unwrap();
    assert_eq!(b.read(0xFA0000, Size::Word, SUPER), Ok(0xABCD));
    assert_eq!(b.read(0xFA0002, Size::Word, SUPER), Ok(0xEF42));
    assert_eq!(b.read(0xFA0001, Size::Byte, SUPER), Ok(0xCD));
    // The last byte of the window is the image's last byte.
    assert_eq!(
        b.read(0xFBFFFF, Size::Byte, SUPER),
        Ok(((CARTRIDGE_BYTES - 1) & 0xFF) as u32)
    );
    assert_eq!(b.read(0xFBFFFE, Size::Word, SUPER), Ok(0xFEFF));
    // A user program may read it too (only the I/O area and the vectors are supervisor only).
    assert_eq!(b.read(0xFA0000, Size::Word, USER), Ok(0xABCD));
    // The debugger sees it without a side effect.
    assert_eq!(b.peek(0xFA0001), 0xCD);
    assert!(b.stop().is_none());
}

/// A shorter image fills the start of the window; [H?] the rest reads `$FF`, as an empty slot does.
#[test]
fn a_short_cartridge_reads_ff_past_its_end() {
    let mut b = bus();
    b.set_cartridge(Some(image(16))).unwrap();
    assert_eq!(b.read(0xFA000E, Size::Word, SUPER), Ok(0x0E0F));
    assert_eq!(b.read(0xFA0010, Size::Word, SUPER), Ok(0xFFFF));
    assert_eq!(b.read(0xFBFFFF, Size::Byte, SUPER), Ok(0xFF));
}

#[test]
fn a_write_to_the_cartridge_window_is_refused_as_documented() {
    for cartridge in [Some(image(256)), None] {
        let mut b = bus();
        b.set_cartridge(cartridge.clone()).unwrap();
        for addr in [0xFA0000, 0xFA0100, 0xFBFFFE] {
            assert_eq!(
                b.write(addr, Size::Word, 0x1234, SUPER),
                Err(BusFault),
                "{addr:#x}"
            );
        }
        assert_eq!(b.write(0xFA0001, Size::Byte, 0x12, SUPER), Err(BusFault));
        assert_eq!(b.write(0xFBFFFF, Size::Byte, 0x12, SUPER), Err(BusFault));
        assert_eq!(
            b.cartridge(),
            cartridge.as_deref(),
            "the image is untouched"
        );
        assert!(b.stop().is_none(), "a bus error, not a stop");
    }
}

#[test]
fn a_cartridge_over_128k_is_an_error() {
    let mut b = bus();
    b.set_cartridge(Some(image(1024))).unwrap();
    let e = b
        .set_cartridge(Some(vec![0; CARTRIDGE_BYTES + 1]))
        .unwrap_err();
    assert!(e.contains("131072") && e.contains("128 KB"), "{e}");
    assert_eq!(
        b.cartridge().map(<[u8]>::len),
        Some(1024),
        "the old cartridge stays"
    );
    assert!(
        b.set_cartridge(Some(Vec::new())).is_err(),
        "an empty image is no cartridge"
    );
    assert!(
        b.set_cartridge(Some(vec![0; CARTRIDGE_BYTES])).is_ok(),
        "exactly 128 KB fits"
    );
}

/// [H?] an empty slot is an open bus: bytes read `$FF`, with no bus error (slice 1's choice; the document gives no
/// level).
#[test]
fn an_empty_slot_reads_as_sourced() {
    let mut b = bus();
    assert_eq!(b.read(0xFA0000, Size::Byte, SUPER), Ok(0xFF));
    assert_eq!(b.read(0xFA0000, Size::Word, SUPER), Ok(0xFFFF));
    assert_eq!(b.read(0xFBFFFE, Size::Word, SUPER), Ok(0xFFFF));
    b.set_cartridge(Some(image(64))).unwrap();
    b.set_cartridge(None).unwrap();
    assert_eq!(
        b.read(0xFA0000, Size::Word, SUPER),
        Ok(0xFFFF),
        "after it is taken out"
    );
}

/// Code in the cartridge runs: TOS's own scan jumps to the application's init routine there.
#[test]
fn the_cpu_runs_code_from_the_cartridge() {
    // JMP $FA0004 in ROM; the cartridge holds MOVE.B #$5A,$0600.W then BRA.S *.
    let program = [0x4EF9, 0x00FA, 0x0004];
    let mut m = AtariSt::new(config(rom(&program, &[]), RamSize::M1)).unwrap();
    let mut cart = vec![0xAB, 0xCD, 0xEF, 0x42];
    for w in move_b(0x5A, 0x0600).into_iter().chain([BRA_SELF]) {
        cart.extend(w.to_be_bytes());
    }
    m.set_cartridge(Some(cart)).unwrap();
    for _ in 0..50 {
        m.step();
    }
    assert_eq!(m.bus().peek(0x600), 0x5A);
    assert!(m.io_stop().is_none());
}
