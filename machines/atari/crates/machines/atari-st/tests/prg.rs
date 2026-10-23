//! A GEMDOS program run from a floppy built in memory (`AUTO\PROG.PRG`): EmuTOS runs the AUTO folder at boot.
//! Needs EmuTOS (`RC_EMUTOS` or the `--firmware auto` cache); without it the test says so and passes.

mod common;
use common::{config, emutos};

use atari_st::{AtariSt, RamSize};
use machine::Machine;
use st_floppy::boot::with_auto_program;

/// A position independent GEMDOS program: Cconws of `text`, a marker byte at `$A0000`, then a loop. The header is the
/// 28 bytes of the format (Atari Compendium, chapter 2) and a zero long for the fixups.
fn program(text: &str) -> Vec<u8> {
    let mut code: Vec<u8> = Vec::new();
    let w = |code: &mut Vec<u8>, ws: &[u16]| {
        for x in ws {
            code.extend(x.to_be_bytes());
        }
    };
    w(&mut code, &[0x487A, 0x0014]); // PEA msg(PC): the message follows the 22 bytes of code
    w(&mut code, &[0x3F3C, 0x0009]); // MOVE.W #9,-(SP)   Cconws
    w(&mut code, &[0x4E41]); // TRAP #1
    w(&mut code, &[0x5C8F]); // ADDQ.L #6,SP
    w(&mut code, &[0x13FC, 0x005A, 0x000A, 0x0000]); // MOVE.B #$5A,$A0000.L (user mode may not write below $800)
    w(&mut code, &[0x60FE]); // BRA.S *
    code.extend(text.as_bytes());
    code.push(0);
    if code.len() % 2 == 1 {
        code.push(0);
    }
    let mut prg = vec![0u8; 28];
    prg[0] = 0x60;
    prg[1] = 0x1A;
    prg[2..6].copy_from_slice(&(code.len() as u32).to_be_bytes());
    prg.extend(code);
    prg.extend([0, 0, 0, 0]);
    prg
}

/// Boots EmuTOS with `prg` as the AUTO program of drive A until its marker appears (or the frames run out).
fn run(rom: &[u8], prg: &[u8]) -> Option<(AtariSt, bool)> {
    let mut m = AtariSt::new(config(rom.to_vec(), RamSize::M1)).unwrap();
    m.insert_disk(0, with_auto_program("prog", prg).unwrap())
        .unwrap();
    while m.io_stop().is_none() && m.vbl_count() < 1500 {
        m.step();
        if m.bus().peek(0xA0000) == 0x5A {
            // A few frames more so the text is drawn.
            let until = m.vbl_count() + 3;
            while m.vbl_count() < until {
                m.step();
            }
            return Some((m, true));
        }
    }
    Some((m, false))
}

#[test]
fn a_prg_prints_its_string_on_the_desktop() {
    let Some(rom) = emutos() else {
        return;
    };
    if let Some(path) = std::env::var_os("RC_DUMP_AUTO_FLOPPY") {
        let image = with_auto_program("prog", &program("HELLO FROM A PRG\r\n")).unwrap();
        std::fs::write(path, image.to_bytes()).unwrap();
    }
    let (shown, ran) = run(&rom, &program("HELLO FROM A PRG\r\n")).unwrap();
    assert!(ran, "the program never ran: {:?}", shown.io_stop());
    let (blank, ran) = run(&rom, &program("")).unwrap();
    assert!(ran);
    let (_, _, a) = shown.render_frame().unwrap();
    let (_, _, b) = blank.render_frame().unwrap();
    assert_ne!(a, b, "the picture shows what the program printed");
}
