//! ACSI hard disks behind the DMA chip: bare-metal programs issue the command sequence of Atari's ACSI application notes
//! (27 September 1985, section 3): DMA address, sector count, the controller's command register, the six descriptor
//! bytes (A0 = 0 for the first), then the completion status.

mod common;
use common::{BRA_SELF, config, move_b, rom};

use atari_st::{AtariSt, RamSize};
use machine::Machine;

const MODE: u16 = 0x8606;
const DATA: u16 = 0x8604;

/// The mode register values: the HDC selected, with the sector count register (bit 4) or the command register,
/// A0 = 0 for the first command byte and 1 for the rest; bit 8 is the write direction.
const COUNT_REG: u16 = 0x0098;
const CMD_FIRST: u16 = 0x0088;
const CMD_REST: u16 = 0x008A;
const WRITE_DIR: u16 = 0x0100;

/// `MOVE.W #imm,$FFxxxx.W`
fn move_w(value: u16, addr: u16) -> [u16; 3] {
    [0x31FC, value, addr]
}

/// A program that runs one ACSI command: the DMA address, the sector count, the command bytes, then reads the status
/// into `$0600` (a word) and the DMA status register into `$0602`.
fn command(addr: u32, sectors: u8, cdb: &[u8], write: bool) -> Vec<u16> {
    let dir = if write { WRITE_DIR } else { 0 };
    let mut p = Vec::new();
    p.extend(move_b((addr >> 16) as u8, 0x8609));
    p.extend(move_b((addr >> 8) as u8, 0x860B));
    p.extend(move_b(addr as u8, 0x860D));
    // Toggle the write bit to clear the status, then select the direction and the count register.
    p.extend(move_w(
        CMD_FIRST ^ WRITE_DIR | dir ^ WRITE_DIR | (dir ^ WRITE_DIR),
        MODE,
    ));
    p.extend(move_w(COUNT_REG | dir, MODE));
    p.extend(move_w(sectors as u16, DATA));
    p.extend(move_w(CMD_FIRST | dir, MODE));
    p.extend(move_w(cdb[0] as u16, DATA));
    p.extend(move_w(CMD_REST | dir, MODE));
    for &b in &cdb[1..] {
        p.extend(move_w(b as u16, DATA));
    }
    p.extend([0x31F8, DATA, 0x0600]); // MOVE.W $FF8604.W,$0600.W: the completion status
    p.extend([0x31F8, MODE, 0x0602]); // MOVE.W $FF8606.W,$0602.W: the DMA status
    p.push(BRA_SELF);
    p
}

/// A disk of `blocks` blocks, block `i` filled with `0x10 + i`.
fn disk(blocks: usize) -> Vec<u8> {
    (0..blocks)
        .flat_map(|i| std::iter::repeat_n(0x10 + i as u8, 512))
        .collect()
}

fn machine(program: &[u16], blocks: usize) -> AtariSt {
    let mut m = AtariSt::new(config(rom(program, &[]), RamSize::M1)).unwrap();
    m.attach_hard_disk(0, disk(blocks)).unwrap();
    m
}

fn run(m: &mut AtariSt) {
    for _ in 0..2000 {
        m.step();
    }
    assert!(m.io_stop().is_none(), "{:?}", m.io_stop());
}

fn word(m: &AtariSt, at: u32) -> u16 {
    u16::from_be_bytes([m.bus().peek(at), m.bus().peek(at + 1)])
}

/// A 6-byte descriptor: controller 0, `op`, block address, count.
fn cdb6(op: u8, lba: u32, count: u8) -> [u8; 6] {
    [
        op,
        (lba >> 16) as u8 & 0x1F,
        (lba >> 8) as u8,
        lba as u8,
        count,
        0,
    ]
}

#[test]
fn test_unit_ready_good_status() {
    let mut m = machine(&command(0x20000, 0, &cdb6(0x00, 0, 0), false), 16);
    run(&mut m);
    assert_eq!(
        word(&m, 0x600),
        0xFF00,
        "no error: status $00 under the DMA's ones"
    );
}

#[test]
fn read6_transfers_the_blocks_to_ram() {
    let mut m = machine(&command(0x20000, 2, &cdb6(0x08, 1, 2), false), 16);
    run(&mut m);
    assert_eq!(word(&m, 0x600) & 0xFF, 0x00);
    assert!(
        m.bus().peek(0x20000) == 0x11 && m.bus().peek(0x201FF) == 0x11,
        "block 1"
    );
    assert!(
        m.bus().peek(0x20200) == 0x12 && m.bus().peek(0x203FF) == 0x12,
        "block 2"
    );
    assert_eq!(m.bus().peek(0x20400), 0, "and no more");
    // The sector count is down to 0 (status bit 1 clear) and no DMA error (bit 0 set, [O] polarity).
    assert_eq!(word(&m, 0x602) & 0x3, 0x1, "{:#x}", word(&m, 0x602));
}

#[test]
fn write6_changes_the_backing_image_only_in_memory() {
    let program = command(0x30000, 1, &cdb6(0x0A, 3, 1), true);
    let mut m = machine(&program, 16);
    m.load(0x30000, &[0xA5; 512]);
    let before = m.hard_disk_image(0).unwrap().to_vec();
    assert!(!m.hard_disk_modified(0));
    run(&mut m);
    assert_eq!(word(&m, 0x600) & 0xFF, 0x00);
    let after = m.hard_disk_image(0).unwrap();
    assert!(m.hard_disk_modified(0));
    assert_eq!(&after[3 * 512..4 * 512], &[0xA5; 512]);
    assert_eq!(&after[..3 * 512], &before[..3 * 512]);
    assert_eq!(&after[4 * 512..], &before[4 * 512..]);
    m.mark_hard_disk_saved(0);
    assert!(!m.hard_disk_modified(0));
}

/// The notes' "Command Errors": `$21` is an invalid address; REQUEST SENSE then says why.
#[test]
fn read_beyond_the_end_is_check_condition_with_sense() {
    let mut m = machine(&command(0x20000, 1, &cdb6(0x08, 16, 1), false), 16);
    run(&mut m);
    assert_eq!(word(&m, 0x600) & 0xFF, 0x21, "invalid address");
    assert_eq!(m.bus().peek(0x20000), 0, "nothing was transferred");
    // A count that runs past the end is the same.
    let mut m = machine(&command(0x20000, 4, &cdb6(0x08, 14, 4), false), 16);
    run(&mut m);
    assert_eq!(word(&m, 0x600) & 0xFF, 0x21);
    // REQUEST SENSE after a failure: the key of the failure in byte 2.
    let mut p = command(0x20000, 1, &cdb6(0x08, 99, 1), false);
    p.pop();
    p.extend(command(0x21000, 1, &cdb6(0x03, 0, 18), false));
    let mut m = machine(&p, 16);
    run(&mut m);
    assert_eq!(word(&m, 0x600) & 0xFF, 0x00);
    assert_eq!(m.bus().peek(0x21000), 0x70, "fixed format sense");
    assert_eq!(m.bus().peek(0x21002), 5, "the key of the invalid address");
}

#[test]
fn an_invalid_command_and_an_invalid_device_are_refused_with_their_codes() {
    let mut m = machine(&command(0x20000, 0, &cdb6(0x13, 0, 0), false), 16);
    run(&mut m);
    assert_eq!(word(&m, 0x600) & 0xFF, 0x20, "invalid command");
    let mut cdb = cdb6(0x00, 0, 0);
    cdb[1] |= 0x20; // device number 1 in byte 1
    let mut m = machine(&command(0x20000, 0, &cdb, false), 16);
    run(&mut m);
    assert_eq!(word(&m, 0x600) & 0xFF, 0x25, "invalid device number");
}

#[test]
fn inquiry_mode_sense_and_read_capacity_answer() {
    // INQUIRY: a direct-access device.
    let mut m = machine(&command(0x20000, 1, &cdb6(0x12, 0, 36), false), 16);
    run(&mut m);
    assert_eq!(m.bus().peek(0x20000), 0, "device type 0: direct access");
    assert_eq!(m.bus().peek(0x20004), 31, "additional length");
    // READ CAPACITY(10) through the extended command: the last block (15) and the block size (512).
    let mut ext = vec![0x1F, 0x25];
    ext.extend([0u8; 9]);
    let mut m = machine(&command(0x20000, 1, &ext, false), 16);
    run(&mut m);
    let long =
        |m: &AtariSt, at: u32| u32::from_be_bytes([0, 1, 2, 3].map(|i| m.bus().peek(at + i)));
    assert_eq!((long(&m, 0x20000), long(&m, 0x20004)), (15, 512));
    // MODE SENSE: the block descriptor carries the block count.
    let mut m = machine(&command(0x20000, 1, &cdb6(0x1A, 0, 12), false), 16);
    run(&mut m);
    assert_eq!(m.bus().peek(0x20007), 16, "the block count's low byte");
}

#[test]
fn a_finished_command_pulls_gpip_i5_low_until_the_status_is_read() {
    // The program stops before it reads the status: I5 is low then.
    let mut p = command(0x20000, 0, &cdb6(0x00, 0, 0), false);
    let at = p.len() - 7; // before the two status reads and the loop
    p.truncate(at);
    p.push(BRA_SELF);
    let mut m = machine(&p, 16);
    run(&mut m);
    assert_eq!(
        m.bus().mfp().peek(0) & 0x20,
        0,
        "I5 low: the controller is done"
    );
    // With the status read it is high again.
    let mut m = machine(&command(0x20000, 0, &cdb6(0x00, 0, 0), false), 16);
    run(&mut m);
    assert_eq!(m.bus().mfp().peek(0) & 0x20, 0x20);
}

/// A target that is not there behaves as it did without ACSI: writes vanish, I5 stays high, and a read of the HDC data
/// register is the known stop.
#[test]
fn an_absent_target_id_behaves_as_before() {
    let mut m = AtariSt::new(config(
        rom(&command(0x20000, 0, &cdb6(0x00, 0, 0), false), &[]),
        RamSize::M1,
    ))
    .unwrap();
    m.attach_hard_disk(1, disk(4)).unwrap(); // target 1, not 0: the command goes to 0
    for _ in 0..2000 {
        m.step();
    }
    assert!(
        m.io_stop().is_some(),
        "the status read of nothing is a stop"
    );
    assert_eq!(m.bus().mfp().peek(0) & 0x20, 0x20, "I5 stayed high");
}

/// The deferred item of the earlier slices: a transfer that would run past the top of RAM moves nothing and sets the DMA
/// error bit. (The machine starts with the memory controller at `$00`: 128K + 128K, so RAM ends at `$40000`.)
#[test]
fn a_transfer_across_the_top_of_ram_is_refused() {
    let mut m = machine(&command(0x3FE00, 2, &cdb6(0x08, 1, 2), false), 16);
    run(&mut m);
    assert_eq!(m.bus().peek(0x3FE00), 0, "nothing was written");
    assert_eq!(
        word(&m, 0x602) & 1,
        0,
        "the DMA error bit: status bit 0 clear"
    );
    // Fitting inside RAM it works.
    let mut m = machine(&command(0x3FC00, 2, &cdb6(0x08, 1, 2), false), 16);
    run(&mut m);
    assert_eq!(m.bus().peek(0x3FC00), 0x11);
    assert_eq!(m.bus().peek(0x3FFFF), 0x12);
}

#[test]
fn a_short_sector_count_limits_the_transfer() {
    let mut m = machine(&command(0x20000, 1, &cdb6(0x08, 1, 2), false), 16);
    run(&mut m);
    assert_eq!(m.bus().peek(0x20000), 0x11);
    assert_eq!(
        m.bus().peek(0x20200),
        0,
        "the DMA stopped when its count reached 0"
    );
}

/// The 1986 table's other four commands ([D] ST spec p.20): accepted when the range exists, nothing changes.
#[test]
fn verify_format_correction_and_mode_select_are_accepted_and_change_nothing() {
    for (op, count) in [(0x05u8, 2u8), (0x06, 1), (0x0D, 0)] {
        let mut m = machine(&command(0x20000, 0, &cdb6(op, 4, count), false), 16);
        let before = m.hard_disk_image(0).unwrap().to_vec();
        run(&mut m);
        assert_eq!(word(&m, 0x600) & 0xFF, 0x00, "opcode {op:#04x}");
        assert!(!m.hard_disk_modified(0), "opcode {op:#04x}");
        assert_eq!(
            m.hard_disk_image(0).unwrap(),
            &before[..],
            "opcode {op:#04x}"
        );
    }
    // a range past the end is an invalid address for the two track commands
    for op in [0x05u8, 0x06] {
        let mut m = machine(&command(0x20000, 0, &cdb6(op, 15, 2), false), 16);
        run(&mut m);
        assert_eq!(word(&m, 0x600) & 0xFF, 0x21, "opcode {op:#04x}");
    }
    // MODE SELECT takes its parameter list from memory (twelve bytes of the one sector the DMA allows) and keeps nothing
    let mut m = machine(&command(0x30000, 1, &cdb6(0x15, 0, 12), true), 16);
    m.load(0x30000, &[0xEE; 12]);
    run(&mut m);
    assert_eq!(word(&m, 0x600) & 0xFF, 0x00);
    assert!(!m.hard_disk_modified(0));
    // a parameter list longer than the sector count allows is refused with the DMA error bit, as a short write is
    let mut m = machine(&command(0x30000, 0, &cdb6(0x15, 0, 12), true), 16);
    run(&mut m);
    assert_eq!(word(&m, 0x602) & 1, 0, "DMA error: {:#x}", word(&m, 0x602));
}
