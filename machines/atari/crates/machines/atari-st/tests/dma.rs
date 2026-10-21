//! The DMA chip and the WD1772 behind it (facts: `docs/hardware-facts.md`, "DMA and floppy controller").

mod common;
use common::{BRA_SELF, config, move_b, rom};

use atari_st::{AtariSt, RamSize, StBus, StopReason};
use machine::Machine;
use mc68000::{FunctionCode, M68kBus, Size};
use st_floppy::Image;

const SUPER: FunctionCode = FunctionCode::SUPERVISOR_DATA;

fn bus() -> StBus {
    StBus::new(&config(rom(&[BRA_SELF], &[]), RamSize::M1)).unwrap()
}

fn w(b: &mut StBus, addr: u32, v: u32) {
    b.write(addr, Size::Word, v, SUPER).unwrap();
}

fn r(b: &mut StBus, addr: u32) -> u32 {
    b.read(addr, Size::Word, SUPER).unwrap()
}

/// `MOVE.W #value,$addr.W`.
fn move_w(value: u16, addr: u16) -> [u16; 3] {
    [0x31FC, value, addr]
}

/// BTST #5,$FFFA01.W; BNE.S back: wait for the FDC's interrupt (GPIP I5 low).
const WAIT_FDC: [u16; 4] = [0x0838, 0x0005, 0xFA01, 0x66F8];

/// Selects drive A side 0 through the PSG's port A (R7 = `$C0` outputs, R14 = `$05`).
fn select_a() -> Vec<u16> {
    [
        move_b(7, 0x8800),
        move_b(0xC0, 0x8802),
        move_b(14, 0x8800),
        move_b(0x05, 0x8802),
    ]
    .concat()
}

/// A 720K image, every byte numbered by its position.
fn image() -> Image {
    let bytes: Vec<u8> = (0..737_280usize).map(|i| (i * 7 + i / 512) as u8).collect();
    Image::parse(&bytes).unwrap()
}

/// Reads sector 1 of cylinder 0 side 0 by DMA to `addr`, then stores the FDC status at `$1000`.
fn read_sector_program(addr: u32) -> Vec<u16> {
    let mut p = select_a();
    p.extend(move_b(addr as u8, 0x860D));
    p.extend(move_b((addr >> 8) as u8, 0x860B));
    p.extend(move_b((addr >> 16) as u8, 0x8609));
    p.extend(move_w(0x190, 0x8606));
    p.extend(move_w(0x090, 0x8606));
    p.extend(move_w(1, 0x8604)); // sector count 1
    p.extend(move_w(0x084, 0x8606));
    p.extend(move_w(1, 0x8604)); // sector register
    p.extend(move_w(0x080, 0x8606));
    p.extend(move_w(0x88, 0x8604)); // READ SECTOR, motor already on (h)
    p.extend(WAIT_FDC);
    p.extend(move_w(0x080, 0x8606));
    p.extend([0x31F8, 0x8604, 0x1000]); // MOVE.W $FF8604.W,$1000.W
    p.push(BRA_SELF);
    p
}

fn run(m: &mut AtariSt, cycles: u64) {
    while m.now() < cycles && m.io_stop().is_none() {
        m.step();
    }
}

#[test]
fn mode_register_selects_fdc_registers() {
    let mut b = bus();
    w(&mut b, 0xFF8606, 0x082);
    w(&mut b, 0xFF8604, 0x12);
    w(&mut b, 0xFF8606, 0x084);
    w(&mut b, 0xFF8604, 0x05);
    w(&mut b, 0xFF8606, 0x082);
    assert_eq!(r(&mut b, 0xFF8604) & 0xFF, 0x12, "track register");
    w(&mut b, 0xFF8606, 0x084);
    assert_eq!(r(&mut b, 0xFF8604) & 0xFF, 0x05, "sector register");
    assert!(b.stop().is_none());
}

/// Internals p.59: the upper 8 bits "consistently read 1".
#[test]
fn ff8604_upper_byte_reads_ones() {
    let mut b = bus();
    w(&mut b, 0xFF8606, 0x082);
    assert_eq!(r(&mut b, 0xFF8604) & 0xFF00, 0xFF00);
}

#[test]
fn sector_count_register() {
    let mut b = bus();
    w(&mut b, 0xFF8606, 0x090);
    w(&mut b, 0xFF8604, 3);
    assert_eq!(r(&mut b, 0xFF8604) & 0xFF, 3);
    assert_eq!(
        r(&mut b, 0xFF8606) & 0x02,
        0x02,
        "status: sector count not zero"
    );
}

#[test]
fn address_counter_bytes_read_back() {
    let mut b = bus();
    for (a, v) in [(0xFF860D, 0x34), (0xFF860B, 0x12), (0xFF8609, 0x05)] {
        b.write(a, Size::Byte, v, SUPER).unwrap();
    }
    assert_eq!(b.read(0xFF8609, Size::Byte, SUPER), Ok(0x05));
    assert_eq!(b.read(0xFF860B, Size::Byte, SUPER), Ok(0x12));
    assert_eq!(b.read(0xFF860D, Size::Byte, SUPER), Ok(0x34));
    assert_eq!(b.dma_address(), 0x05_1234);
}

#[test]
fn read_sector_1_into_ram() {
    let img = image();
    let mut m = AtariSt::new(config(
        rom(&read_sector_program(0x1_0000), &[]),
        RamSize::M1,
    ))
    .unwrap();
    m.insert_disk(0, img.clone()).unwrap();
    run(&mut m, 8_000_000);
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
    let got: Vec<u8> = (0..512).map(|i| m.read_memory(0x1_0000 + i)).collect();
    assert_eq!(got, img.sector(0, 0, 1).unwrap());
    assert_eq!(m.bus().dma_address(), 0x1_0200);
    let status = m.read_memory(0x1001);
    assert_eq!(status & 0x1C, 0, "no RNF, CRC or lost data: {status:02X}");
    assert_eq!(
        m.bus().mfp().read(0) & 0x20,
        0x20,
        "I5 high again after the status read"
    );
}

#[test]
fn dma_into_rom_is_ignored_and_into_io_stops() {
    let img = image();
    let image_rom = rom(&read_sector_program(0xFC_8000), &[]);
    let mut m = AtariSt::new(config(image_rom.clone(), RamSize::M1)).unwrap();
    m.insert_disk(0, img.clone()).unwrap();
    run(&mut m, 8_000_000);
    assert!(m.io_stop().is_none());
    let rom_after: Vec<u8> = (0..512).map(|i| m.read_memory(0xFC_8000 + i)).collect();
    assert_eq!(rom_after, image_rom[0x8000..0x8200]);

    let mut m = AtariSt::new(config(
        rom(&read_sector_program(0xFF_9000), &[]),
        RamSize::M1,
    ))
    .unwrap();
    m.insert_disk(0, img).unwrap();
    run(&mut m, 8_000_000);
    assert_eq!(m.io_stop().map(|s| s.reason), Some(StopReason::NoFact));
}

#[test]
fn acsi_write_dropped_and_i5_stays_high() {
    let mut b = bus();
    w(&mut b, 0xFF8606, 0x088);
    w(&mut b, 0xFF8604, 0x00);
    assert!(b.stop().is_none());
    assert_eq!(b.read(0xFFFA01, Size::Byte, SUPER).unwrap() & 0x20, 0x20);
}

#[test]
fn acsi_read_has_no_fact() {
    let mut b = bus();
    w(&mut b, 0xFF8606, 0x088);
    r(&mut b, 0xFF8604);
    assert_eq!(b.stop().map(|s| s.reason), Some(StopReason::NoFact));
}

/// EmuTOS's probe with no disk: force interrupt, RESTORE (`$0B`, motor on flag), wait for the interrupt; [O] the
/// status then reads `$E4` (motor on, write protect, spin-up, track 0).
#[test]
fn fdc_restore_with_no_disk_status_e4() {
    let mut p = select_a();
    p.extend(move_w(0x080, 0x8606));
    p.extend(move_w(0xD0, 0x8604));
    p.extend(move_w(0x0B, 0x8604));
    p.extend(WAIT_FDC);
    p.extend(move_w(0x080, 0x8606));
    p.extend([0x31F8, 0x8604, 0x1000]);
    p.push(BRA_SELF);
    let mut m = AtariSt::new(config(rom(&p, &[]), RamSize::M1)).unwrap();
    run(&mut m, 2_000_000);
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
    assert_eq!(m.read_memory(0x1001), 0xE4);
}

/// Ejecting the disk while a sector is being read: the controller never finds its sector, the DMA stops moving
/// bytes, nothing panics.
#[test]
fn eject_during_read_sector_never_panics() {
    let mut m = AtariSt::new(config(
        rom(&read_sector_program(0x1_0000), &[]),
        RamSize::M1,
    ))
    .unwrap();
    m.insert_disk(0, image()).unwrap();
    run(&mut m, 1_000);
    assert!(m.eject_disk(0).is_some());
    let at = m.bus().dma_address();
    run(&mut m, 8_000_000);
    assert!(m.io_stop().is_none());
    assert_eq!(m.bus().dma_address(), at);
}
