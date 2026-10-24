//! Mega Drive cartridge images: the ROM header at `$100` (layout: Plutiedev "ROM header reference", fact rows H1 in
//! `docs/hardware-facts.md`), the region field in both its styles, and the `.smd` interleaved dump format.
//!
//! The input is untrusted: every function returns an error for a short, empty or oversized file instead of panicking, and a
//! file without a valid header still loads (homebrew and test ROMs) with `header_valid = false`.
#![forbid(unsafe_code)]

use std::fmt;

/// The largest image accepted (the Mega Drive's own cartridge space is 4 MB; mapper games go beyond it).
pub const MAX_ROM_BYTES: usize = 16 * 1024 * 1024;

/// The size of the `.smd` header and of one interleaved block.
const SMD_HEADER: usize = 512;
const SMD_BLOCK: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CartError {
    Empty,
    TooLarge(usize),
}

impl fmt::Display for CartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CartError::Empty => write!(f, "the cartridge image is empty"),
            CartError::TooLarge(n) => write!(
                f,
                "the cartridge image is {n} bytes; at most {MAX_ROM_BYTES} are accepted"
            ),
        }
    }
}

impl std::error::Error for CartError {}

/// The regions the header lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Regions {
    pub japan: bool,
    pub usa: bool,
    pub europe: bool,
}

impl Regions {
    pub fn is_empty(self) -> bool {
        !(self.japan || self.usa || self.europe)
    }
}

/// The battery memory the header declares (`"RA"`, a type byte, `$20`, start, end).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SramInfo {
    pub kind: u8,
    pub start: u32,
    pub end: u32,
}

impl SramInfo {
    /// Whether the type byte says the memory keeps its contents (bit 6).
    pub fn saves(&self) -> bool {
        self.kind & 0x40 != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub console: String,
    pub copyright: String,
    pub domestic_name: String,
    pub overseas_name: String,
    pub serial: String,
    pub checksum: u16,
    pub devices: String,
    pub rom_start: u32,
    pub rom_end: u32,
    pub ram_start: u32,
    pub ram_end: u32,
    pub sram: Option<SramInfo>,
    pub regions: Regions,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cartridge {
    /// The ROM as the 68000 sees it from `$000000`: de-interleaved when the file was a `.smd` dump.
    pub rom: Vec<u8>,
    pub header: Header,
    /// Whether the system-type field at `$100` starts with `SEGA` (a console with TMSS refuses a ROM without it).
    pub header_valid: bool,
    /// The file was an interleaved `.smd` dump.
    pub was_smd: bool,
}

fn text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                b as char
            } else {
                ' '
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn be32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// The region field at `$1F0` (3 bytes): the old style lists letters `J`, `U`, `E` in any order; the new style is one
/// hexadecimal digit whose bits are domestic 60 Hz (1), domestic 50 Hz (2, no region), overseas 60 Hz (4), overseas 50 Hz (8).
/// `E` is ambiguous (Europe in the old style, everywhere but Japan in the new) and is read as Europe.
pub fn parse_regions(field: &[u8]) -> Regions {
    let old = Regions {
        japan: field.contains(&b'J'),
        usa: field.contains(&b'U'),
        europe: field.contains(&b'E'),
    };
    if !old.is_empty() {
        return old;
    }
    let digit = field
        .iter()
        .copied()
        .find(|b| *b != b' ' && *b != 0)
        .and_then(|b| (b as char).to_digit(16));
    match digit {
        Some(d) => Regions {
            japan: d & 1 != 0,
            usa: d & 4 != 0,
            europe: d & 8 != 0,
        },
        None => Regions::default(),
    }
}

/// The header of a ROM image and whether its system-type field starts with `SEGA`.
pub fn parse_header(rom: &[u8]) -> (Header, bool) {
    let mut padded = [b' '; 0x200];
    let n = rom.len().min(0x200);
    padded[..n].copy_from_slice(&rom[..n]);
    let h = &padded[0x100..];
    let valid = rom.len() >= 0x104 && h.starts_with(b"SEGA");
    let sram = (&h[0xB0..0xB2] == b"RA" && h[0xB3] == 0x20).then(|| SramInfo {
        kind: h[0xB2],
        start: be32(&h[0xB4..]),
        end: be32(&h[0xB8..]),
    });
    (
        Header {
            console: text(&h[0..16]),
            copyright: text(&h[0x10..0x20]),
            domestic_name: text(&h[0x20..0x50]),
            overseas_name: text(&h[0x50..0x80]),
            serial: text(&h[0x80..0x8E]),
            checksum: u16::from_be_bytes([h[0x8E], h[0x8F]]),
            devices: text(&h[0x90..0xA0]),
            rom_start: be32(&h[0xA0..]),
            rom_end: be32(&h[0xA4..]),
            ram_start: be32(&h[0xA8..]),
            ram_end: be32(&h[0xAC..]),
            sram,
            regions: parse_regions(&h[0xF0..0xF3]),
        },
        valid,
    )
}

/// De-interleaves a `.smd` body (the 512-byte header already removed): in each 16 KB block the first 8 KB hold the bytes at
/// odd ROM offsets and the second 8 KB those at even offsets. A final partial block is dropped.
pub fn deinterleave_smd(body: &[u8]) -> Vec<u8> {
    let blocks = body.len() / SMD_BLOCK;
    let mut out = vec![0u8; blocks * SMD_BLOCK];
    for b in 0..blocks {
        let block = &body[b * SMD_BLOCK..(b + 1) * SMD_BLOCK];
        let dest = &mut out[b * SMD_BLOCK..(b + 1) * SMD_BLOCK];
        for i in 0..SMD_BLOCK / 2 {
            dest[2 * i + 1] = block[i];
            dest[2 * i] = block[SMD_BLOCK / 2 + i];
        }
    }
    out
}

/// The checksum the header should hold: the sum of the words from `$200` to the end of the ROM, modulo 65 536 (an odd last
/// byte counts as the high byte of a word).
pub fn computed_checksum(rom: &[u8]) -> u16 {
    let mut sum = 0u16;
    let mut i = 0x200;
    while i < rom.len() {
        let hi = u16::from(rom[i]);
        let lo = rom.get(i + 1).copied().map_or(0, u16::from);
        sum = sum.wrapping_add(hi << 8 | lo);
        i += 2;
    }
    sum
}

impl Cartridge {
    /// Parses an image: a plain ROM, or a `.smd` dump (recognised by its size being 512 bytes more than whole 16 KB blocks and
    /// by the `SEGA` text appearing at `$100` only after de-interleaving).
    pub fn parse(bytes: &[u8]) -> Result<Cartridge, CartError> {
        if bytes.is_empty() {
            return Err(CartError::Empty);
        }
        if bytes.len() > MAX_ROM_BYTES + SMD_HEADER {
            return Err(CartError::TooLarge(bytes.len()));
        }
        let plain_has_header = bytes.len() >= 0x104 && &bytes[0x100..0x104] == b"SEGA";
        let (rom, was_smd) = if !plain_has_header
            && bytes.len() > SMD_HEADER
            && (bytes.len() - SMD_HEADER).is_multiple_of(SMD_BLOCK)
        {
            let candidate = deinterleave_smd(&bytes[SMD_HEADER..]);
            if candidate.len() >= 0x104 && &candidate[0x100..0x104] == b"SEGA" {
                (candidate, true)
            } else {
                (bytes.to_vec(), false)
            }
        } else {
            (bytes.to_vec(), false)
        };
        if rom.len() > MAX_ROM_BYTES {
            return Err(CartError::TooLarge(rom.len()));
        }
        let (header, header_valid) = parse_header(&rom);
        Ok(Cartridge {
            rom,
            header,
            header_valid,
            was_smd,
        })
    }

    /// Whether the header's checksum equals the computed one.
    pub fn checksum_ok(&self) -> bool {
        self.header.checksum == computed_checksum(&self.rom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4 KB ROM with a header: system type, names, serial, region field and the ROM range.
    fn rom_with_header(region: &[u8; 3]) -> Vec<u8> {
        let mut rom = vec![0u8; 0x1000];
        rom[0x100..0x110].copy_from_slice(b"SEGA MEGA DRIVE ");
        rom[0x110..0x120].copy_from_slice(b"(C)TEST 2026.OCT");
        rom[0x120..0x150].fill(b' ');
        rom[0x120..0x125].copy_from_slice(b"HELLO");
        rom[0x150..0x180].fill(b' ');
        rom[0x150..0x158].copy_from_slice(b"HELLO GB");
        rom[0x180..0x18E].copy_from_slice(b"GM 12345678-01");
        rom[0x190..0x1A0].fill(b' ');
        rom[0x190..0x192].copy_from_slice(b"J6");
        rom[0x1A0..0x1A4].copy_from_slice(&0u32.to_be_bytes());
        rom[0x1A4..0x1A8].copy_from_slice(&0xFFFu32.to_be_bytes());
        rom[0x1A8..0x1AC].copy_from_slice(&0xFF0000u32.to_be_bytes());
        rom[0x1AC..0x1B0].copy_from_slice(&0xFFFFFFu32.to_be_bytes());
        rom[0x1B0..0x1C8].fill(b' ');
        rom[0x1F0..0x1F3].copy_from_slice(region);
        rom
    }

    /// The inverse of `deinterleave_smd`, written independently for the round trip.
    fn interleave_smd(rom: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; SMD_HEADER];
        for block in rom.chunks(SMD_BLOCK) {
            let odd: Vec<u8> = block.iter().skip(1).step_by(2).copied().collect();
            let even: Vec<u8> = block.iter().step_by(2).copied().collect();
            out.extend(odd);
            out.extend(even);
        }
        out
    }

    #[test]
    fn parses_a_minimal_header() {
        let cart = Cartridge::parse(&rom_with_header(b"JUE")).unwrap();
        assert!(cart.header_valid);
        assert!(!cart.was_smd);
        let h = &cart.header;
        assert_eq!(h.console, "SEGA MEGA DRIVE");
        assert_eq!(h.copyright, "(C)TEST 2026.OCT");
        assert_eq!(h.domestic_name, "HELLO");
        assert_eq!(h.overseas_name, "HELLO GB");
        assert_eq!(h.serial, "GM 12345678-01");
        assert_eq!(h.devices, "J6");
        assert_eq!((h.rom_start, h.rom_end), (0, 0xFFF));
        assert_eq!((h.ram_start, h.ram_end), (0xFF0000, 0xFFFFFF));
        assert_eq!(h.sram, None);
    }

    #[test]
    fn region_letters_and_hex_form() {
        let r = |f: &[u8; 3]| parse_regions(f);
        assert_eq!(
            r(b"JUE"),
            Regions {
                japan: true,
                usa: true,
                europe: true
            }
        );
        assert_eq!(
            r(b"U  "),
            Regions {
                japan: false,
                usa: true,
                europe: false
            }
        );
        assert_eq!(
            r(b" E "),
            Regions {
                japan: false,
                usa: false,
                europe: true
            }
        );
        // new style: bit 0 domestic 60 Hz, bit 2 overseas 60 Hz, bit 3 overseas 50 Hz
        assert_eq!(
            r(b"5  "),
            Regions {
                japan: true,
                usa: true,
                europe: false
            }
        );
        assert_eq!(
            r(b"8  "),
            Regions {
                japan: false,
                usa: false,
                europe: true
            }
        );
        assert_eq!(
            r(b"F  "),
            Regions {
                japan: true,
                usa: true,
                europe: true
            }
        );
        assert!(r(b"   ").is_empty());
        assert!(r(b"???").is_empty());
    }

    #[test]
    fn smd_deinterleave_round_trip() {
        let mut rom = rom_with_header(b"JUE");
        rom.resize(SMD_BLOCK * 2, 0);
        for (i, b) in rom.iter_mut().enumerate().skip(0x200) {
            *b = (i * 7 + 3) as u8;
        }
        let smd = interleave_smd(&rom);
        assert_eq!(smd.len(), SMD_HEADER + rom.len());
        let cart = Cartridge::parse(&smd).unwrap();
        assert!(cart.was_smd);
        assert!(cart.header_valid);
        assert_eq!(cart.rom, rom);
    }

    #[test]
    fn a_plain_rom_is_not_taken_for_smd_even_at_a_matching_size() {
        let mut rom = rom_with_header(b"JUE");
        rom.resize(SMD_HEADER + SMD_BLOCK, 0xA5);
        let cart = Cartridge::parse(&rom).unwrap();
        assert!(!cart.was_smd);
        assert_eq!(cart.rom, rom);
    }

    #[test]
    fn odd_size_rom_is_kept_whole_not_rejected() {
        let mut rom = rom_with_header(b"JUE");
        rom.truncate(0x1001 - 1 + 1);
        rom.push(0x42);
        let cart = Cartridge::parse(&rom).unwrap();
        assert_eq!(cart.rom.len(), 0x1001);
    }

    #[test]
    fn a_file_without_a_header_still_loads() {
        let cart = Cartridge::parse(&[0x4E, 0x71, 0x60, 0xFE]).unwrap();
        assert!(!cart.header_valid);
        assert_eq!(cart.rom.len(), 4);
        assert!(cart.header.regions.is_empty());
        assert_eq!(cart.header.console, "");
    }

    #[test]
    fn empty_and_oversized_files_are_errors_not_panics() {
        assert_eq!(Cartridge::parse(&[]), Err(CartError::Empty));
        let big = vec![0u8; MAX_ROM_BYTES + SMD_HEADER + 1];
        assert!(matches!(
            Cartridge::parse(&big),
            Err(CartError::TooLarge(_))
        ));
    }

    #[test]
    fn sram_header_detected() {
        let mut rom = rom_with_header(b"JUE");
        rom[0x1B0..0x1B4].copy_from_slice(&[b'R', b'A', 0xF8, 0x20]);
        rom[0x1B4..0x1B8].copy_from_slice(&0x200001u32.to_be_bytes());
        rom[0x1B8..0x1BC].copy_from_slice(&0x20FFFFu32.to_be_bytes());
        let cart = Cartridge::parse(&rom).unwrap();
        let sram = cart.header.sram.unwrap();
        assert_eq!(sram.kind, 0xF8);
        assert!(sram.saves());
        assert_eq!((sram.start, sram.end), (0x200001, 0x20FFFF));
        rom[0x1B2] = 0xB0;
        assert!(!Cartridge::parse(&rom).unwrap().header.sram.unwrap().saves());
    }

    #[test]
    fn the_checksum_is_the_word_sum_from_0x200() {
        let mut rom = rom_with_header(b"JUE");
        rom[0x200..0x204].copy_from_slice(&[0x12, 0x34, 0x00, 0x01]);
        let want = computed_checksum(&rom);
        assert_eq!(want, 0x1235);
        rom[0x18E..0x190].copy_from_slice(&want.to_be_bytes());
        assert!(Cartridge::parse(&rom).unwrap().checksum_ok());
        rom[0x18E] ^= 1;
        assert!(!Cartridge::parse(&rom).unwrap().checksum_ok());
    }

    #[test]
    fn short_files_never_panic_at_any_length() {
        let rom = rom_with_header(b"JUE");
        for len in 1..0x220 {
            let _ = Cartridge::parse(&rom[..len]);
        }
    }
}
