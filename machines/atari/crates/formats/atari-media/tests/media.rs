use atari_media::{Atr, Cartridge, Cas, CasPlayer, CasPosition, Mapper, Segment, Xex};

#[test]
fn xex_keeps_order_overlaps_and_vector_segments() {
    let bytes = [
        255, 255, 255, 255, 0, 32, 1, 32, 0xaa, 0xbb, 255, 255, 255, 255, 1, 32, 1, 32, 0xcc, 0xe2,
        2, 0xe3, 2, 0, 32, 0xe0, 2, 0xe1, 2, 1, 32,
    ];
    let xex = Xex::parse(&bytes).unwrap();
    assert_eq!(
        xex.segments.iter().map(|s| s.start).collect::<Vec<_>>(),
        [0x2000, 0x2001, 0x02e2, 0x02e0]
    );
    assert_eq!(xex.segments[1].bytes, [0xcc]);
    assert_eq!(Xex::parse(&xex.to_bytes().unwrap()).unwrap(), xex);
}

#[test]
fn xex_rejects_each_truncation_and_wrapped_range() {
    let bytes = [255, 255, 0, 32, 1, 32, 0xaa, 0xbb];
    for end in 0..bytes.len() {
        assert!(Xex::parse(&bytes[..end]).is_err(), "prefix {end}");
    }
    assert!(Xex::parse(&[255, 255, 1, 32, 0, 32]).is_err());
    for suffix in [&[0][..], &[255, 255][..], &[0, 32, 0][..]] {
        let mut bad = bytes.to_vec();
        bad.extend_from_slice(suffix);
        assert!(Xex::parse(&bad).is_err());
    }
}

#[test]
fn xex_checks_public_segments_before_serializing() {
    for segment in [
        Segment {
            start: 0x2000,
            bytes: vec![],
        },
        Segment {
            start: 0xfffe,
            bytes: vec![1, 2, 3],
        },
        Segment {
            start: 0xffff,
            bytes: vec![1],
        },
    ] {
        assert!(
            Xex {
                segments: vec![segment]
            }
            .to_bytes()
            .is_err()
        );
    }
    let xex = Xex {
        segments: vec![Segment {
            start: 0xfffe,
            bytes: vec![1, 2],
        }],
    };
    assert_eq!(Xex::parse(&xex.to_bytes().unwrap()).unwrap(), xex);
    let full = Xex {
        segments: vec![Segment {
            start: 0,
            bytes: vec![7; 65536],
        }],
    };
    assert_eq!(Xex::parse(&full.to_bytes().unwrap()).unwrap(), full);
    let huge = Xex {
        segments: vec![full.segments[0].clone(); 256],
    };
    assert!(huge.to_bytes().is_err());
    assert!(Xex::parse(&vec![0xff; 16 * 1024 * 1024 + 1]).is_err());
}

#[test]
fn atr_logical_boot_sectors_have_exact_offsets() {
    for size in [128, 256] {
        let mut atr = Atr::blank(720, size).unwrap();
        for number in [1, 3, 4, 720] {
            let sector = atr.sector_mut(number).unwrap();
            assert_eq!(sector.len(), if number <= 3 { 128 } else { size as usize });
            sector.fill(number as u8);
        }
        assert!(atr.sector(0).is_err());
        assert!(atr.sector(721).is_err());
        assert!(atr.sector_mut(721).is_err());
        let bytes = atr.to_bytes();
        assert_eq!(
            &bytes[..8],
            if size == 128 {
                &[0x96, 2, 0x80, 0x16, 0x80, 0, 0, 0]
            } else {
                &[0x96, 2, 0xe8, 0x2c, 0, 1, 0, 0]
            }
        );
        assert_eq!(bytes[16 + 384], 4);
        assert_eq!(Atr::parse(&bytes).unwrap(), atr);
    }
}

#[test]
fn atr_preserves_header_and_honors_any_nonzero_protection_byte() {
    for protection in [1, 0xff] {
        let mut bytes = Atr::blank(4, 256).unwrap().to_bytes();
        bytes[15] = protection;
        bytes[16] = 42;
        let mut atr = Atr::parse(&bytes).unwrap();
        assert!(atr.write_protected());
        assert_eq!(atr.sector(1).unwrap()[0], 42);
        assert!(atr.sector_mut(1).unwrap_err().contains("write protected"));
        assert_eq!(atr.to_bytes(), bytes);
    }
}

#[test]
fn atr_rejects_truncation_size_mismatch_extensions_and_noncanonical_geometry() {
    let bytes = Atr::blank(4, 256).unwrap().to_bytes();
    for end in 0..bytes.len() {
        assert!(Atr::parse(&bytes[..end]).is_err(), "prefix {end}");
    }
    for offset in [0, 1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14] {
        let mut bad = bytes.clone();
        bad[offset] ^= 0x40;
        assert!(Atr::parse(&bad).is_err(), "header offset {offset}");
    }
    let mut bad = bytes.clone();
    bad.push(0);
    assert!(Atr::parse(&bad).is_err());
    // A physical/SIO2PC layout stores 4 * 256 bytes instead of 3 * 128 + 256.
    let mut physical = vec![0; 16 + 1024];
    physical[..16].copy_from_slice(&bytes[..16]);
    physical[2..4].copy_from_slice(&64u16.to_le_bytes());
    assert!(Atr::parse(&physical).unwrap_err().contains("geometry"));
}

#[test]
fn atr_limits_and_small_images_are_checked_before_allocation() {
    assert!(Atr::blank(0, 128).is_err());
    assert!(Atr::blank(720, 512).is_err());
    for count in 1..=4 {
        let atr = Atr::blank(count, 256).unwrap();
        assert_eq!(Atr::parse(&atr.to_bytes()).unwrap(), atr);
    }
    for size in [128, 256] {
        let atr = Atr::blank(u16::MAX, size).unwrap();
        assert_eq!(
            Atr::parse(&atr.to_bytes()).unwrap().sector_count(),
            u16::MAX
        );
    }
    let mut too_many = Atr::blank(u16::MAX, 128).unwrap().to_bytes();
    too_many.extend_from_slice(&[0; 128]);
    let paragraphs = ((too_many.len() - 16) as u32 / 16).to_le_bytes();
    too_many[2..4].copy_from_slice(&paragraphs[..2]);
    too_many[6..8].copy_from_slice(&paragraphs[2..]);
    assert!(Atr::parse(&too_many).unwrap_err().contains("sector count"));
    assert!(Atr::parse(&vec![0; 16 * 1024 * 1024 + 1]).is_err());
}

#[test]
fn xfd_normalizes_real_sd_ed_dd_layouts_without_losing_sectors() {
    for (count, size) in [(720, 128), (1040, 128), (720, 256), (1440, 256)] {
        let mut atr = Atr::blank(count, size).unwrap();
        for number in [1, 2, 3, 4, count] {
            atr.sector_mut(number).unwrap().fill(number as u8);
        }
        for physical in [false, true] {
            let raw = atr.to_xfd(physical);
            assert_eq!(Atr::parse_xfd(&raw).unwrap(), atr);
            if physical && size == 256 {
                assert_eq!(raw[256], 2);
                assert_eq!(raw[768], 4);
                let mut bad = raw.clone();
                bad[128] = 1;
                assert!(Atr::parse_xfd(&bad).is_err());
            } else {
                assert_eq!(raw[128], 2);
            }
            assert!(Atr::parse_xfd(&raw[..raw.len() - 1]).is_err());
        }
    }
    for size in [0, 128, 1024, 92_161, 184_319, 16 * 1024 * 1024 + 1] {
        assert!(Atr::parse_xfd(&vec![0; size]).is_err());
    }
}

fn car(mapper: u32, rom: &[u8]) -> Vec<u8> {
    let mut bytes = b"CART".to_vec();
    bytes.extend_from_slice(&mapper.to_be_bytes());
    bytes.extend_from_slice(&rom.iter().map(|&n| u32::from(n)).sum::<u32>().to_be_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(rom);
    bytes
}

#[test]
fn cartridge_headers_checksums_and_raw_ambiguity_are_checked() {
    for (size, mapper, low) in [(8192, 1, 0xa000), (16384, 2, 0x8000)] {
        let rom = vec![0xa5; size];
        let bytes = car(mapper, &rom);
        let cart = Cartridge::parse_car(&bytes).unwrap();
        assert_eq!(cart, Cartridge::parse_raw(&rom, None).unwrap());
        assert_eq!(cart.to_car(), bytes);
        assert_eq!(cart.read(low), Some(0xa5));
        assert_eq!(cart.read(0xbfff), Some(0xa5));
        assert_eq!(cart.read(low - 1), None);
        assert_eq!(cart.read(0xc000), None);
        for offset in [0, 4, 7, 8, 11, 12, 15, 16] {
            let mut bad = bytes.clone();
            bad[offset] ^= 0x80;
            assert!(Cartridge::parse_car(&bad).is_err(), "offset {offset}");
        }
        for length in [0, 3, 15, 16, bytes.len() - 1] {
            assert!(Cartridge::parse_car(&bytes[..length]).is_err());
        }
    }
    assert!(Cartridge::parse_raw(&vec![0; 32768], None).is_err());
    assert!(Cartridge::parse_car(&car(4, &vec![0; 32768])).is_err());
    assert!(Cartridge::parse_raw(&vec![0; 8192], Some(Mapper::Standard16K)).is_err());
}

#[test]
fn xegs_has_fixed_last_bank_and_data_selected_lower_bank_not_disable() {
    for (mapper, banks) in [(12, 4), (13, 8), (14, 16)] {
        let rom: Vec<u8> = (0..banks).flat_map(|bank| vec![bank; 8192]).collect();
        let mut cart = Cartridge::parse_car(&car(mapper, &rom)).unwrap();
        assert_eq!(cart.to_car(), car(mapper, &rom));
        assert_eq!(cart.read(0x8000), Some(0));
        for value in 0..=255 {
            cart.write_control(0xd5a7, value);
            assert_eq!(cart.read(0x8000), Some(value & (banks - 1)));
            assert_eq!(cart.read(0x9fff), Some(value & (banks - 1)));
            assert_eq!(cart.read(0xa000), Some(banks - 1));
            assert_eq!(cart.read(0xbfff), Some(banks - 1));
        }
        let bank = cart.bank();
        assert!(cart.restore_bank(banks).is_err());
        assert_eq!(cart.bank(), bank);
        cart.write_control(0xd600, 0);
        assert_eq!(cart.bank(), bank);
        cart.reset();
        assert_eq!(cart.read(0x8000), Some(0));
    }
}

fn chunk(bytes: &mut Vec<u8>, tag: &[u8; 4], aux: u16, data: &[u8]) {
    bytes.extend_from_slice(tag);
    bytes.extend_from_slice(&(data.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&aux.to_le_bytes());
    bytes.extend_from_slice(data);
}

#[test]
fn cas_plays_exact_8n1_data_gaps_baud_changes_and_motor_pause() {
    let mut bytes = Vec::new();
    chunk(&mut bytes, b"FUJI", 0, b"test");
    chunk(&mut bytes, b"data", 2, &[0x55]);
    chunk(&mut bytes, b"baud", 1200, &[]);
    chunk(&mut bytes, b"FUJI", 0, b"part two");
    chunk(&mut bytes, b"data", 1, &[0xa5]);
    let cas = Cas::parse(&bytes).unwrap();
    assert_eq!(cas.to_bytes(), bytes);
    let mut player = CasPlayer::new(cas, 600_000).unwrap();
    assert!(player.tick(1199, true));
    assert!(!player.tick(1, true));
    for expected in [true, false, true, false, true, false, true, false, true] {
        assert_eq!(player.tick(1000, true), expected);
    }
    assert!(player.tick(1000, true)); // next record's 1ms gap
    let position = player.position();
    assert!(player.tick(u64::MAX, false));
    assert_eq!(player.position(), position);
    assert!(!player.tick(600, true));
    for expected in [true, false, true, false, false, true, false, true, true] {
        assert_eq!(player.tick(500, true), expected);
    }
    assert!(player.tick(500, true));
    assert!(player.finished());
}

#[test]
fn cas_rejects_bad_chunks_and_continues_from_fractional_bit_phase() {
    let mut bytes = Vec::new();
    chunk(&mut bytes, b"FUJI", 0, &[]);
    chunk(&mut bytes, b"data", 1, &[0x55, 0xaa]);
    let cas = Cas::parse(&bytes).unwrap();
    for length in [0, 1, 7, 9, bytes.len() - 1] {
        assert!(Cas::parse(&bytes[..length]).is_err());
    }
    for (tag, aux, data) in [
        (b"baud", 0, &[][..]),
        (b"baud", 600, &[0][..]),
        (b"fsk ", 0, &[][..]),
        (b"FUJI", 0, &[0xff][..]),
    ] {
        let mut bad = bytes.clone();
        chunk(&mut bad, tag, aux, data);
        assert!(Cas::parse(&bad).is_err());
    }
    let mut a = CasPlayer::new(cas.clone(), 1_773_447).unwrap();
    let mut b = CasPlayer::new(cas, 1_773_447).unwrap();
    a.tick(12_345, true);
    b.restore_position(a.position()).unwrap();
    let position = b.position();
    for bad in [
        CasPosition {
            record: 2,
            cycle: 0,
        },
        CasPosition {
            record: 1,
            cycle: 1,
        },
        CasPosition {
            record: 0,
            cycle: u64::MAX,
        },
    ] {
        assert!(b.restore_position(bad).is_err());
        assert_eq!(b.position(), position);
    }
    for _ in 0..50_000 {
        assert_eq!(a.tick(1, true), b.tick(1, true));
        assert_eq!(a.position(), b.position());
    }
    let mut empty = Vec::new();
    chunk(&mut empty, b"FUJI", 0, &[]);
    chunk(&mut empty, b"data", 0, &[]);
    assert!(
        CasPlayer::new(Cas::parse(&empty).unwrap(), 1_789_772)
            .unwrap()
            .finished()
    );
}
