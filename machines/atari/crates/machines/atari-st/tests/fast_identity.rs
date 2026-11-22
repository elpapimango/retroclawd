//! `fast` mode must not change while `accurate` mode is built (Phase 3 plan, Global Constraints): the frame, RAM, I/O
//! log, saved state, emulated time and PC of a few runs are hashed and compared with `fast_identity.table`, which was
//! recorded with the code before the time base moved into the bus (`RC_RECORD_IDENTITY=1` rewrites it; only do that
//! on purpose and say why in the commit). Needs EmuTOS (`RC_EMUTOS` or the `--firmware auto` cache) and, for the
//! probe run, `timing/build.sh`; a missing input is a note and a pass.

mod common;
use common::{boot_floppy, config, emutos, io_lines};

use atari_st::{AtariSt, RamSize};
use machine::Machine;
use std::fmt::Write as _;
use std::path::PathBuf;

fn table_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fast_identity.table")
}

fn sha(bytes: &[u8]) -> String {
    atari_st::state::hex(&atari_st::state::sha1_of(bytes))
}

/// One line per run: name, frame, RAM, I/O log, state, time and PC hashes.
fn summary(name: &str, m: &AtariSt) -> String {
    let (_, _, frame) = m.render_frame().unwrap();
    let ram: Vec<u8> = (0..0x10_0000u32).map(|a| m.bus().peek(a)).collect();
    let io = io_lines(m).join("\n");
    let state = m.save_state_checked().unwrap_or_default();
    let mut out = String::new();
    write!(
        out,
        "{name} frame={} ram={} io={} state={} now={} pc={:06X}",
        sha(&frame),
        sha(&ram),
        sha(io.as_bytes()),
        sha(&state),
        m.now(),
        m.cpu().pc
    )
    .unwrap();
    out
}

fn run_frames(m: &mut AtariSt, vbls: u64) {
    while m.io_stop().is_none() && m.vbl_count() < vbls {
        m.step();
    }
}

fn runs() -> Option<Vec<String>> {
    let rom = emutos()?;
    let mut out = Vec::new();

    let mut desktop = AtariSt::new(config(rom.clone(), RamSize::M1)).unwrap();
    run_frames(&mut desktop, 200);
    out.push(summary("desktop-200", &desktop));

    let mut floppy = AtariSt::new(config(rom.clone(), RamSize::M1)).unwrap();
    floppy.insert_disk(0, boot_floppy()).unwrap();
    run_frames(&mut floppy, 400);
    out.push(summary("boot-floppy-400", &floppy));

    let mut half_meg = AtariSt::new(config(rom, RamSize::K512)).unwrap();
    run_frames(&mut half_meg, 120);
    out.push(summary("desktop-512k-120", &half_meg));
    Some(out)
}

#[test]
fn fast_mode_is_byte_identical_to_the_recorded_table() {
    let Some(lines) = runs() else {
        return;
    };
    let text = lines.join("\n") + "\n";
    if std::env::var_os("RC_RECORD_IDENTITY").is_some() {
        std::fs::write(table_path(), &text).unwrap();
        eprintln!("recorded {}", table_path().display());
        return;
    }
    let want = std::fs::read_to_string(table_path()).unwrap_or_default();
    assert_eq!(
        text.replace("\r\n", "\n"),
        want.replace("\r\n", "\n"),
        "fast mode changed: a hash, the time or the PC differs from tests/fast_identity.table"
    );
}
