//! Neo Geo cartridge sets: one zip holds the cartridge's chips as files whose extension names the region (`.p1` the 68000
//! program, `.s1` the fix layer, `.c1`/`.c2` sprite data in byte-interleaved pairs, `.m1` the Z80 program, `.v11` and
//! `.v21` the two sample regions). The convention is the one the owner's sets show (fact N1 in `docs/hardware-facts.md`).
//!
//! The input is untrusted: a zip that is damaged, holds too much data or too many entries is an error, never a panic, and
//! the file names are only classified, never used as paths.
#![forbid(unsafe_code)]

use std::fmt;
use std::io::{Cursor, Read};

/// The most data one set may expand to (the largest cartridges are far below this).
pub const MAX_SET_BYTES: u64 = 256 * 1024 * 1024;
/// The most entries one zip may hold.
pub const MAX_ENTRIES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetError {
    /// Not a readable zip.
    Zip(String),
    TooManyEntries(usize),
    TooLarge,
    /// A set without a 68000 program is not a cartridge (the BIOS set has none either).
    NoProgram,
    /// `.c` files that do not pair up (an odd count, or two of a pair with different sizes).
    UnpairedSprites,
}

impl fmt::Display for SetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetError::Zip(e) => write!(f, "not a readable zip: {e}"),
            SetError::TooManyEntries(n) => write!(f, "{n} entries is more than a set holds"),
            SetError::TooLarge => write!(f, "the set expands to more than {MAX_SET_BYTES} bytes"),
            SetError::NoProgram => write!(f, "no .p file: not a cartridge set"),
            SetError::UnpairedSprites => write!(f, "the .c files do not form pairs of equal size"),
        }
    }
}

impl std::error::Error for SetError {}

/// A cartridge's regions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NeoSet {
    /// The 68000 program: `.p1` first, then `.p2`, and so on.
    pub p: Vec<u8>,
    /// The fix layer's tiles.
    pub s: Vec<u8>,
    /// The sprite data: each pair `.c1`/`.c2` byte-interleaved (`.c1` the even bytes), the pairs one after the other.
    pub c: Vec<u8>,
    /// The Z80 program.
    pub m: Vec<u8>,
    /// The sample region for ADPCM-A: every `.v1x` file in order, or every single-digit `.v` file when there is no `.v2x`.
    pub v_a: Vec<u8>,
    /// The sample region for ADPCM-B: every `.v2x` file in order; empty when the cartridge has none.
    pub v_b: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    P,
    S,
    C,
    M,
    V,
}

/// Which region a file name belongs to, and its number (`v11` is number 11).
fn classify(name: &str) -> Option<(Kind, u32)> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    // `.sp2` is the second program file (the banked part), named like a fix file (fact N47)
    let ext = match ext.strip_prefix("sp") {
        Some(rest) if !rest.is_empty() => format!("p{rest}"),
        _ => ext,
    };
    let kind = match ext.as_bytes().first()? {
        b'p' => Kind::P,
        b's' => Kind::S,
        b'c' => Kind::C,
        b'm' => Kind::M,
        b'v' => Kind::V,
        _ => return None,
    };
    let digits = &ext[1..];
    if digits.is_empty() || digits.len() > 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((kind, digits.parse().ok()?))
}

/// Reads a set from the bytes of its zip.
pub fn load(zip_bytes: &[u8]) -> Result<NeoSet, SetError> {
    load_with_limit(zip_bytes, MAX_SET_BYTES)
}

/// `load` with the cap on the expanded size as a parameter (a test lowers it).
fn load_with_limit(zip_bytes: &[u8], limit: u64) -> Result<NeoSet, SetError> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(zip_bytes)).map_err(|e| SetError::Zip(e.to_string()))?;
    if zip.len() > MAX_ENTRIES {
        return Err(SetError::TooManyEntries(zip.len()));
    }
    let mut files: Vec<(Kind, u32, Vec<u8>)> = Vec::new();
    // the bytes really read count against the cap: a header that declares less than the entry expands to is not believed
    let mut read_total = 0u64;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| SetError::Zip(e.to_string()))?;
        let Some((kind, number)) = classify(entry.name()) else {
            continue;
        };
        let declared = entry.size();
        if declared > limit - read_total {
            return Err(SetError::TooLarge);
        }
        let mut data = Vec::new();
        entry
            .take(limit - read_total + 1)
            .read_to_end(&mut data)
            .map_err(|e| SetError::Zip(e.to_string()))?;
        read_total += data.len() as u64;
        if read_total > limit || data.len() as u64 != declared {
            return Err(SetError::TooLarge);
        }
        files.push((kind, number, data));
    }
    files.sort_by_key(|(_, n, _)| *n);
    let of = |k: Kind| files.iter().filter(move |(kind, _, _)| *kind == k);
    let mut set = NeoSet::default();
    for (_, _, d) in of(Kind::P) {
        set.p.extend_from_slice(d);
    }
    if set.p.is_empty() {
        return Err(SetError::NoProgram);
    }
    for (_, _, d) in of(Kind::S) {
        set.s.extend_from_slice(d);
    }
    for (_, _, d) in of(Kind::M) {
        set.m.extend_from_slice(d);
    }
    let sprites: Vec<&Vec<u8>> = of(Kind::C).map(|(_, _, d)| d).collect();
    if !sprites.len().is_multiple_of(2) {
        return Err(SetError::UnpairedSprites);
    }
    // four 2 MiB chips: each pair holds two 2 MiB stretches of the sprite space, the first halves of both pairs below the second
    // halves (fact N46)
    let halves = sprites.len() == 4
        && sprites.iter().all(|d| d.len() == 2 * 1024 * 1024)
        && set.p.len() <= 1024 * 1024;
    let mut pairs: Vec<Vec<u8>> = Vec::new();
    for pair in sprites.chunks(2) {
        if pair[0].len() != pair[1].len() {
            return Err(SetError::UnpairedSprites);
        }
        let mut interleaved = Vec::with_capacity(pair[0].len() * 2);
        for (a, b) in pair[0].iter().zip(pair[1]) {
            interleaved.push(*a);
            interleaved.push(*b);
        }
        pairs.push(interleaved);
    }
    if halves {
        let h = pairs[0].len() / 2;
        for part in 0..2 {
            for pair in &pairs {
                set.c.extend_from_slice(&pair[part * h..(part + 1) * h]);
            }
        }
    } else {
        for pair in &pairs {
            set.c.extend_from_slice(pair);
        }
    }
    let has_b = of(Kind::V).any(|(_, n, _)| (20..30).contains(n) || (200..300).contains(n));
    for (_, n, d) in of(Kind::V) {
        if has_b && (20..30).contains(n) {
            set.v_b.extend_from_slice(d);
        } else {
            set.v_a.extend_from_slice(d);
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in files {
            w.start_file(*name, options).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn the_extension_names_the_region_and_the_number_the_order() {
        assert_eq!(classify("010-p1.p1"), Some((Kind::P, 1)));
        assert_eq!(classify("010-V12.V12"), Some((Kind::V, 12)));
        assert_eq!(classify("a.c10"), Some((Kind::C, 10)));
        assert_eq!(classify("readme.txt"), None);
        assert_eq!(classify("noext"), None);
        assert_eq!(classify("241-p2.sp2"), Some((Kind::P, 2)));
        assert_eq!(classify("x.s1"), Some((Kind::S, 1)));
        assert_eq!(classify("x.p"), None);
        assert_eq!(classify("x.p1x"), None);
    }

    #[test]
    fn a_set_is_split_into_regions_and_the_sprite_pairs_are_interleaved() {
        let bytes = zip_of(&[
            ("g-p2.p2", &[3, 4]),
            ("g-p1.p1", &[1, 2]),
            ("g-s1.s1", &[9]),
            ("g-m1.m1", &[7, 7]),
            ("g-c1.c1", &[0xA0, 0xA1]),
            ("g-c2.c2", &[0xB0, 0xB1]),
            ("g-c3.c3", &[0xC0]),
            ("g-c4.c4", &[0xD0]),
            ("g-v12.v12", &[2]),
            ("g-v11.v11", &[1]),
            ("g-v21.v21", &[5]),
            ("notes.txt", &[0]),
        ]);
        let set = load(&bytes).unwrap();
        assert_eq!(set.p, [1, 2, 3, 4]);
        assert_eq!(set.s, [9]);
        assert_eq!(set.m, [7, 7]);
        assert_eq!(set.c, [0xA0, 0xB0, 0xA1, 0xB1, 0xC0, 0xD0]);
        assert_eq!(set.v_a, [1, 2]);
        assert_eq!(set.v_b, [5]);
    }

    #[test]
    fn without_a_second_sample_region_every_v_file_is_one_region() {
        let set = load(&zip_of(&[
            ("a.p1", &[1]),
            ("a.v1", &[1, 2]),
            ("a.v2", &[3]),
        ]))
        .unwrap();
        assert_eq!(set.v_a, [1, 2, 3]);
        assert!(set.v_b.is_empty());
    }

    #[test]
    fn the_cap_counts_every_entry_together() {
        let bytes = zip_of(&[("a.p1", &[1; 600]), ("a.s1", &[2; 600])]);
        assert!(load_with_limit(&bytes, 1200).is_ok());
        assert_eq!(load_with_limit(&bytes, 1199), Err(SetError::TooLarge));
        assert_eq!(load_with_limit(&bytes, 599), Err(SetError::TooLarge));
    }

    #[test]
    fn four_chips_of_two_mib_put_the_first_halves_of_both_pairs_below_the_second_halves() {
        let mib = 1024 * 1024;
        let chip = |n: u8| -> Vec<u8> {
            // the first half of the chip is filled with n, the second with n + 10
            let mut v = vec![n; mib];
            v.extend(vec![n + 10; mib]);
            v
        };
        let (c1, c2, c3, c4) = (chip(1), chip(2), chip(3), chip(4));
        let bytes = zip_of(&[
            ("a.p1", &[1]),
            ("a.c1", &c1),
            ("a.c2", &c2),
            ("a.c3", &c3),
            ("a.c4", &c4),
        ]);
        let set = load(&bytes).unwrap();
        assert_eq!(set.c.len(), 8 * mib);
        // 2 MiB stretches of the sprite space: pair one first half, pair two first half, pair one second half, pair two second half
        let at = |k: usize| (set.c[k * 2 * mib], set.c[k * 2 * mib + 1]);
        assert_eq!(
            [at(0), at(1), at(2), at(3)],
            [(1, 2), (3, 4), (11, 12), (13, 14)]
        );
    }

    #[test]
    fn with_a_two_mib_program_four_two_mib_chips_stay_in_a_row() {
        let mib = 1024 * 1024;
        let chip = |n: u8| -> Vec<u8> {
            let mut v = vec![n; mib];
            v.extend(vec![n + 10; mib]);
            v
        };
        let (c1, c2, c3, c4) = (chip(1), chip(2), chip(3), chip(4));
        let p = vec![0u8; 2 * mib];
        let bytes = zip_of(&[
            ("a.p1", &p),
            ("a.c1", &c1),
            ("a.c2", &c2),
            ("a.c3", &c3),
            ("a.c4", &c4),
        ]);
        let set = load(&bytes).unwrap();
        let at = |k: usize| (set.c[k * mib], set.c[k * mib + 1]);
        assert_eq!(
            [at(0), at(2), at(4), at(6)],
            [(1, 2), (11, 12), (3, 4), (13, 14)]
        );
    }

    #[test]
    fn damaged_or_wrong_sets_are_errors() {
        assert!(matches!(load(b"not a zip"), Err(SetError::Zip(_))));
        assert_eq!(load(&zip_of(&[("a.s1", &[1])])), Err(SetError::NoProgram));
        assert_eq!(
            load(&zip_of(&[("a.p1", &[1]), ("a.c1", &[1])])),
            Err(SetError::UnpairedSprites)
        );
        assert_eq!(
            load(&zip_of(&[
                ("a.p1", &[1]),
                ("a.c1", &[1]),
                ("a.c2", &[1, 2])
            ])),
            Err(SetError::UnpairedSprites)
        );
    }
}

#[cfg(test)]
mod real_sets {
    /// Reads the sets named by `RC_NEOSETS` (paths separated by `;`) and prints their region sizes.
    #[test]
    #[ignore]
    fn real_sets_load() {
        let Ok(paths) = std::env::var("RC_NEOSETS") else {
            return;
        };
        for p in paths.split(';') {
            let set = super::load(&std::fs::read(p).unwrap()).unwrap();
            println!(
                "{p}: p {} s {} c {} m {} v_a {} v_b {}",
                set.p.len(),
                set.s.len(),
                set.c.len(),
                set.m.len(),
                set.v_a.len(),
                set.v_b.len()
            );
            assert!(!set.p.is_empty());
        }
    }
}
