use atari_8bit::{Atari800, Profile, Roms, Timing};
use machine::Machine;

fn roms() -> Vec<(Profile, Vec<u8>)> {
    let os800 = std::fs::read(std::env::var("ATARI800_OS").expect("ATARI800_OS")).unwrap();
    let osxl = std::fs::read(std::env::var("ATARIXL_OS").expect("ATARIXL_OS")).unwrap();
    vec![
        (Profile::Atari800, os800),
        (Profile::Atari800Xl, osxl.clone()),
        (Profile::Atari65Xe, osxl.clone()),
        (Profile::Atari130Xe, osxl),
    ]
}

fn cassette_record(data: &[u8]) -> Vec<u8> {
    assert_eq!(data.len(), 128);
    let mut record = vec![0x55, 0x55, 0xfc];
    record.extend_from_slice(data);
    let sum = record.iter().fold(0u16, |sum, &byte| {
        let next = sum + u16::from(byte);
        (next & 255) + (next >> 8)
    }) as u8;
    record.push(sum);
    let mut cas = b"FUJI\0\0\0\0data".to_vec();
    cas.extend_from_slice(&(record.len() as u16).to_le_bytes());
    cas.extend_from_slice(&20000u16.to_le_bytes());
    cas.extend_from_slice(&record);
    cas
}

#[test]
#[ignore = "requires ATARI800_OS, ATARIXL_OS and ATARI800_BASIC"]
fn replacement_rom_loads_and_runs_cassette_through_cio() {
    let basic = std::fs::read(std::env::var("ATARI800_BASIC").expect("ATARI800_BASIC")).unwrap();
    for (profile, os) in roms() {
        let mut m = Atari800::with_profile(
            profile,
            Roms {
                os,
                basic: Some(basic.clone()),
            },
            true,
            Timing::Accurate,
        )
        .unwrap();
        while m.cycles() < 35568 * 300 {
            m.step();
        }
        let mut payload = [0; 128];
        let program = [
            0xa2, 0, 0x8a, 0x49, 0xa5, 0x9d, 0, 0x40, 0xe8, 0xd0, 0xf7, 0x60,
        ];
        payload[..program.len()].copy_from_slice(&program);
        let cas = cassette_record(&payload);
        m.attach_tape(&cas).unwrap();
        assert_eq!(m.load(0x700, b"C:\x9b"), 3);
        let iocb = [0xff, 0, 3, 0, 0, 7, 0, 0, 128, 0, 4, 0];
        assert_eq!(m.load(0x350, &iocb), iocb.len());
        let code = [
            0xa2, 0x10, 0x20, 0x56, 0xe4, 0x8c, 0, 6, 0xa9, 7, 0x8d, 0x52, 3, 0xa9, 8, 0x8d, 0x55,
            3, 0x20, 0x56, 0xe4, 0x8c, 1, 6, 0x20, 0, 8, 0xa9, 0x7f, 0x8d, 2, 6, 0x4c, 0x20, 5,
        ];
        assert_eq!(m.load(0x500, &code), code.len());
        m.set_pc(0x500);
        let end = m.cycles() + 35568 * 1500;
        while m.cycles() < end && m.read_memory(0x602) != 0x7f {
            let frame = m.cycles() / 35568;
            m.set_keyboard(
                if frame % 16 < 4 { 1 << 0x0c } else { 0 },
                false,
                false,
                false,
            );
            m.step();
            assert!(!m.halted(), "{} CAS jam at {:04x}", profile.id(), m.pc());
        }
        assert_eq!(
            m.read_memory(0x602),
            0x7f,
            "{} cassette did not return at {:04x}, motor {}, position {:?}",
            profile.id(),
            m.pc(),
            m.tape_motor(),
            m.tape().map(|tape| tape.position())
        );
        assert_eq!(m.read_memory(0x600), 1, "{} cassette OPEN", profile.id());
        assert_eq!(m.read_memory(0x601), 1, "{} cassette READ", profile.id());
        for index in 0..128 {
            assert_eq!(
                m.read_memory(0x800 + index),
                payload[index as usize],
                "{} CAS",
                profile.id()
            );
        }
        for index in 0..256 {
            assert_eq!(
                m.read_memory(0x4000 + index),
                index as u8 ^ 0xa5,
                "{} CAS Program",
                profile.id()
            );
        }
        if let Ok(output) = std::env::var("ATARI800_OUTPUT") {
            std::fs::write(std::path::Path::new(&output).join("scenario-read.cas"), cas).unwrap();
        }
    }
}

#[test]
#[ignore = "requires ATARI800_OS, ATARIXL_OS and ATARI800_BASIC"]
fn replacement_rom_boots_standard_and_xegs_cartridges() {
    let basic = std::fs::read(std::env::var("ATARI800_BASIC").expect("ATARI800_BASIC")).unwrap();
    for (profile, os) in roms() {
        for mapper in [1, 2, 12, 13, 14] {
            let size = match mapper {
                1 => 8192,
                2 => 16384,
                12 => 32768,
                13 => 65536,
                _ => 131072,
            };
            let mut rom = vec![0xff; size];
            let start = size - 8192;
            // OS cartridge entry at A000 writes the same complete S3 memory pattern.
            let code = [
                0x78, 0xa9, 0, 0x8d, 0x0e, 0xd4, 0xa2, 0, 0x8a, 0x49, 0xa5, 0x9d, 0, 0x40, 0xe8,
                0xd0, 0xf7, 0x4c, 0x11, 0xa0, 0x60,
            ];
            rom[start..start + code.len()].copy_from_slice(&code);
            rom[size - 6..].copy_from_slice(&[0, 0xa0, 0, 4, 0x14, 0xa0]);
            let mut car = b"CART".to_vec();
            car.extend_from_slice(&u32::to_be_bytes(mapper));
            car.extend_from_slice(
                &rom.iter()
                    .map(|&byte| u32::from(byte))
                    .sum::<u32>()
                    .to_be_bytes(),
            );
            car.extend_from_slice(&[0; 4]);
            car.extend_from_slice(&rom);
            let mut m = Atari800::with_profile(
                profile,
                Roms {
                    os: os.clone(),
                    basic: Some(basic.clone()),
                },
                true,
                Timing::Accurate,
            )
            .unwrap();
            m.attach_cartridge("car", &car).unwrap();
            while m.cycles() < 35568 * 300 {
                m.step();
                assert!(
                    !m.halted(),
                    "{} CAR {mapper} jam at {:04x}",
                    profile.id(),
                    m.pc()
                );
            }
            for index in 0..256 {
                assert_eq!(
                    m.read_memory(0x4000 + index),
                    index as u8 ^ 0xa5,
                    "{} CAR {mapper}",
                    profile.id()
                );
            }
            if let Ok(output) = std::env::var("ATARI800_OUTPUT") {
                std::fs::write(
                    std::path::Path::new(&output).join(format!("scenario-type{mapper}.car")),
                    &car,
                )
                .unwrap();
            }
        }
    }
}

#[test]
#[ignore = "requires ATARI800_OS, ATARIXL_OS and ATARI800_BASIC"]
fn replacement_rom_writes_and_reads_second_drive_through_siov() {
    let basic = std::fs::read(std::env::var("ATARI800_BASIC").expect("ATARI800_BASIC")).unwrap();
    for (profile, os) in roms() {
        let mut m = Atari800::with_profile(
            profile,
            Roms {
                os,
                basic: Some(basic.clone()),
            },
            true,
            Timing::Accurate,
        )
        .unwrap();
        while m.cycles() < 35568 * 300 {
            m.step();
        }
        let disk = atari_media::Atr::blank(720, 128).unwrap().to_bytes();
        m.attach_disk_in(0, "atr", &disk).unwrap();
        m.attach_disk_in(1, "atr", &disk).unwrap();
        assert_eq!(m.load(0x400, &[0xa5; 128]), 128);
        let dcb = [0x31, 2, 0x50, 0x80, 0, 4, 15, 0, 128, 0, 1, 0];
        assert_eq!(m.load(0x300, &dcb), dcb.len());
        // Call the OS SIO vector, then read the same D2 sector into a new buffer.
        let code = [
            0x20, 0x59, 0xe4, 0x8c, 0x00, 0x06, 0xa9, 0x52, 0x8d, 0x02, 0x03, 0xa9, 0x40, 0x8d,
            0x03, 0x03, 0xa9, 0x80, 0x8d, 0x04, 0x03, 0x20, 0x59, 0xe4, 0x8c, 0x01, 0x06, 0xa9,
            0x7f, 0x8d, 0x02, 0x06, 0x4c, 0x20, 0x05,
        ];
        assert_eq!(m.load(0x500, &code), code.len());
        m.set_pc(0x500);
        let end = m.cycles() + 35568 * 500;
        while m.cycles() < end && m.read_memory(0x602) != 0x7f {
            m.step();
            assert!(!m.halted(), "{} jam at {:04x}", profile.id(), m.pc());
        }
        assert_eq!(
            m.read_memory(0x602),
            0x7f,
            "{} SIO did not return",
            profile.id()
        );
        assert_eq!(m.read_memory(0x600), 1, "{} PUT status", profile.id());
        assert_eq!(m.read_memory(0x601), 1, "{} READ status", profile.id());
        assert!((0x480..0x500).all(|address| m.read_memory(address) == 0xa5));
        assert!(m.disk_dirty_in(1));
        assert!(!m.disk_dirty_in(0));
        let saved = m.export_disk_in(1).unwrap();
        let atr = atari_media::Atr::parse(&saved).unwrap();
        assert_eq!(atr.sector(1).unwrap(), &[0xa5; 128]);
        m.mark_disk_saved_in(1).unwrap();
        assert!(!m.disk_dirty_in(1));
    }
}

#[test]
#[ignore = "requires ATARI800_OS, ATARIXL_OS, ATARI800_BASIC, ATARI800_DISK and ATARI800_OUTPUT"]
fn replacement_rom_boots_atr_through_serial_pins() {
    let basic = std::fs::read(std::env::var("ATARI800_BASIC").expect("ATARI800_BASIC")).unwrap();
    let disk = std::fs::read(std::env::var("ATARI800_DISK").expect("ATARI800_DISK")).unwrap();
    let output =
        std::path::PathBuf::from(std::env::var("ATARI800_OUTPUT").expect("ATARI800_OUTPUT"));
    for (profile, os) in roms() {
        for pal in [true, false] {
            for timing in [Timing::Fast, Timing::Accurate] {
                let mut m = Atari800::with_profile(
                    profile,
                    Roms {
                        os: os.clone(),
                        basic: Some(basic.clone()),
                    },
                    pal,
                    timing,
                )
                .unwrap();
                m.attach_disk(&disk).unwrap();
                let frame = if pal { 35568 } else { 29868 };
                while m.disk_reads() < 2 && m.cycles() < frame * 300 {
                    m.step();
                }
                assert!(m.disk_busy(), "second sector never began");
                let saved = m.save_state();
                let end = m.cycles() + frame * 10;
                while m.cycles() < end {
                    m.step();
                }
                let future = m.save_state();
                m.load_state(&saved).unwrap();
                while m.cycles() < end {
                    m.step();
                }
                assert_eq!(m.save_state(), future, "in-flight SIO continuation differs");
                while m.cycles() < frame * 600 {
                    m.step();
                    assert!(!m.halted(), "jam at {:04x}", m.pc());
                }
                let name = format!(
                    "{}-disk-{}-{timing:?}",
                    profile.id(),
                    if pal { "pal" } else { "ntsc" }
                );
                std::fs::write(
                    output.join(format!("{name}.rgb")),
                    m.render_frame().unwrap().2,
                )
                .unwrap();
                let ram: Vec<_> = (0..49152).map(|address| m.read_memory(address)).collect();
                std::fs::write(output.join(format!("{name}.ram")), ram).unwrap();
                println!(
                    "{name}: PC={:04x}, reads={}, sector={}, POKEY={:02x?}",
                    m.pc(),
                    m.disk_reads(),
                    m.disk_sector(),
                    m.chip_registers("pokey")
                );
                assert!(
                    m.disk_reads() > 5,
                    "OS never read the diagnostic through SIO"
                );
                let saved = m.save_state();
                let end = m.cycles() + frame * 3;
                while m.cycles() < end {
                    m.step();
                }
                let future = m.save_state();
                m.load_state(&saved).unwrap();
                while m.cycles() < end {
                    m.step();
                }
                assert_eq!(m.save_state(), future);
            }
        }
    }
}

/// Explicit private ROM inputs; never falls back to guessed or stock firmware.
#[test]
#[ignore = "requires ATARI800_OS, ATARIXL_OS, ATARI800_BASIC and ATARI800_OUTPUT"]
fn replacement_rom_reaches_basic_and_types_through_pokey() {
    let basic = std::fs::read(std::env::var("ATARI800_BASIC").expect("ATARI800_BASIC")).unwrap();
    let output =
        std::path::PathBuf::from(std::env::var("ATARI800_OUTPUT").expect("ATARI800_OUTPUT"));
    for (profile, os) in roms() {
        for pal in [true, false] {
            for timing in [Timing::Fast, Timing::Accurate] {
                let mut m = Atari800::with_profile(
                    profile,
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
                let stem = format!(
                    "{}-{}-{timing:?}",
                    profile.id(),
                    if pal { "pal" } else { "ntsc" }
                );
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
}
