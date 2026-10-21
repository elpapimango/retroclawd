use atari_8bit::{Atari800, Roms, Timing};
use machine::Machine;

/// Explicit private ROM inputs; never falls back to guessed or stock firmware.
#[test]
#[ignore = "requires ATARI800_OS, ATARI800_BASIC and ATARI800_OUTPUT"]
fn replacement_rom_reaches_basic_and_types_through_pokey() {
    let os = std::fs::read(std::env::var("ATARI800_OS").expect("ATARI800_OS")).unwrap();
    let basic = std::fs::read(std::env::var("ATARI800_BASIC").expect("ATARI800_BASIC")).unwrap();
    let output =
        std::path::PathBuf::from(std::env::var("ATARI800_OUTPUT").expect("ATARI800_OUTPUT"));
    for pal in [true, false] {
        for timing in [Timing::Fast, Timing::Accurate] {
            let mut m = Atari800::new(
                Roms {
                    os: os.clone(),
                    basic: Some(basic.clone()),
                },
                pal,
                timing,
            )
            .unwrap();
            let frame = if pal { 35568 } else { 29868 };
            let advance = |m: &mut Atari800, frames: u64| {
                let end = m.cycles() + frames * frame;
                while m.cycles() < end {
                    m.step();
                    assert!(!m.halted(), "CPU jam at {:04x}", m.pc());
                }
            };
            advance(&mut m, 300);
            let stem = format!("{}-{timing:?}", if pal { "pal" } else { "ntsc" });
            let screen_memory = |m: &Atari800| {
                let start = u16::from_le_bytes([m.read_memory(0x58), m.read_memory(0x59)]);
                (0..960)
                    .map(|i| m.read_memory(u64::from(start) + i))
                    .collect::<Vec<_>>()
            };
            let boot = screen_memory(&m);
            std::fs::write(output.join(format!("{stem}-boot.bin")), &boot).unwrap();
            std::fs::write(
                output.join(format!("{stem}-boot.rgb")),
                m.render_frame().unwrap().2,
            )
            .unwrap();
            println!(
                "{stem}: PC={:04x}, RAMTOP={:02x}, screen={:02x?}",
                m.pc(),
                m.read_memory(0x6a),
                &boot[..80]
            );
            assert!(
                boot.windows(5).any(|w| w == [0x32, 0x65, 0x61, 0x64, 0x79]),
                "No replacement BASIC Ready screen"
            );
            for scan in [
                0x0a, 0x28, 0x0d, 0x23, 0x2d, 0x21, 0x1f, 0x1e, 0x07, 0x1a, 0x18, 0x0c,
            ] {
                m.set_keyboard(1u64 << scan, false, false, false);
                advance(&mut m, 4);
                m.set_keyboard(0, false, false, false);
                advance(&mut m, 4);
            }
            advance(&mut m, 10);
            let typed = screen_memory(&m);
            std::fs::write(output.join(format!("{stem}-typed.bin")), &typed).unwrap();
            std::fs::write(
                output.join(format!("{stem}-typed.rgb")),
                m.render_frame().unwrap().2,
            )
            .unwrap();
            assert!(
                typed.windows(3).any(|w| w == [0x14, 0x10, 0x18]),
                "PRINT 12*34 did not produce 408"
            );
            let saved = m.save_state();
            advance(&mut m, 5);
            let future = m.save_state();
            m.load_state(&saved).unwrap();
            advance(&mut m, 5);
            assert_eq!(m.save_state(), future);
            m.reset();
            advance(&mut m, 100);
            assert!(
                screen_memory(&m)
                    .windows(5)
                    .any(|w| w == [0x32, 0x65, 0x61, 0x64, 0x79])
            );
        }
    }
}
