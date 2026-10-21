use atari_media::{Atr, Segment, Xex};

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
