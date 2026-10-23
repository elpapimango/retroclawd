//! A floppy built in memory to run one GEMDOS program: a 720K disk with a FAT12 file system holding `AUTO\NAME.PRG`.
//!
//! TOS runs the programs in the `AUTO` folder of the boot drive before it starts the desktop, so a disk like this
//! boots straight into the program (spec section 15: "PRG on the ST without GEMDOS emulation"). The writer is the
//! boot-only subset of a FAT12 writer: one directory, one file, a deterministic layout and time stamp, nothing the
//! machine does not need. `rcdisk st` in `retroclawd-tools` makes real disks; this is a deliberate small duplicate
//! (the tools cannot be a dependency of this repository).
//!
//! Layout ([H] *Atari ST Internals* pp.178-179 for the BPB; the FAT12 layout is the standard DOS one): 512-byte
//! sectors, 2 sectors a cluster, 1 reserved sector, 2 FATs of 3 sectors, 112 root entries (7 sectors), 1440 sectors;
//! the data area starts at sector 14 and cluster 2 is its first cluster. The first cluster holds the `AUTO`
//! directory, the program follows from cluster 3.

use crate::Image;

/// A 720K disk.
pub const DISK_BYTES: usize = 737_280;
const SECTOR: usize = 512;
const SECTORS: usize = 1440;
const CLUSTER: usize = 2 * SECTOR;
const FAT_SECTORS: usize = 3;
const ROOT_ENTRIES: usize = 112;
const ROOT_SECTORS: usize = ROOT_ENTRIES * 32 / SECTOR;
const DATA_START: usize = 1 + 2 * FAT_SECTORS + ROOT_SECTORS;
const CLUSTERS: usize = (SECTORS - DATA_START) / 2;
/// The largest program: the data clusters less the one the `AUTO` folder takes.
pub const MAX_PROGRAM_BYTES: usize = (CLUSTERS - 1) * CLUSTER;
/// `2026-01-01 00:00:00` as a DOS date and time: the disk is the same every time it is made.
const DATE: u16 = (2026 - 1980) << 9 | 1 << 5 | 1;
const END_OF_CHAIN: u16 = 0xFFF;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootError {
    /// The name cannot be an 8.3 file name (after upper-casing).
    BadName(String),
    /// Not a GEMDOS executable: no `$601A` header.
    NotAProgram,
    TooLarge {
        bytes: usize,
        max: usize,
    },
}

impl std::fmt::Display for BootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootError::BadName(n) => write!(f, "\"{n}\" is not an 8.3 file name"),
            BootError::NotAProgram => write!(
                f,
                "not a GEMDOS program (it does not start with the $601A header)"
            ),
            BootError::TooLarge { bytes, max } => write!(
                f,
                "the program is {bytes} bytes; a floppy built for it holds {max}"
            ),
        }
    }
}

impl std::error::Error for BootError {}

/// `NAME.EXT` as the 11 bytes of a directory entry; the name is upper-cased.
fn short_name(name: &str) -> Result<[u8; 11], BootError> {
    let bad = || BootError::BadName(name.to_string());
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s, e),
        None => (name, ""),
    };
    let valid = |s: &str, max: usize| {
        s.len() <= max
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-$~!#%&@^".contains(&b))
    };
    if stem.is_empty() || !valid(stem, 8) || !valid(ext, 3) {
        return Err(bad());
    }
    let mut out = [b' '; 11];
    for (i, b) in stem.bytes().enumerate() {
        out[i] = b.to_ascii_uppercase();
    }
    for (i, b) in ext.bytes().enumerate() {
        out[8 + i] = b.to_ascii_uppercase();
    }
    Ok(out)
}

/// A 32-byte directory entry.
fn entry(name: [u8; 11], attr: u8, cluster: u16, size: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(&name);
    e[11] = attr;
    e[22..24].copy_from_slice(&0u16.to_le_bytes()); // time
    e[24..26].copy_from_slice(&DATE.to_le_bytes());
    e[26..28].copy_from_slice(&cluster.to_le_bytes());
    e[28..32].copy_from_slice(&size.to_le_bytes());
    e
}

fn set_fat(fat: &mut [u8], cluster: usize, value: u16) {
    let at = cluster * 3 / 2;
    if cluster.is_multiple_of(2) {
        fat[at] = value as u8;
        fat[at + 1] = (fat[at + 1] & 0xF0) | ((value >> 8) & 0x0F) as u8;
    } else {
        fat[at] = (fat[at] & 0x0F) | (((value & 0x0F) as u8) << 4);
        fat[at + 1] = (value >> 4) as u8;
    }
}

/// The 720K image holding `prg` as `AUTO\<name>`. A name may be given with or without its extension (`.PRG` is added
/// when there is none).
pub fn with_auto_program(name: &str, prg: &[u8]) -> Result<Image, BootError> {
    let name = if name.contains('.') {
        name.to_string()
    } else {
        format!("{name}.PRG")
    };
    let short = short_name(&name)?;
    if prg.len() < 28 || prg[..2] != [0x60, 0x1A] {
        return Err(BootError::NotAProgram);
    }
    if prg.len() > MAX_PROGRAM_BYTES {
        return Err(BootError::TooLarge {
            bytes: prg.len(),
            max: MAX_PROGRAM_BYTES,
        });
    }
    let mut disk = vec![0u8; DISK_BYTES];
    // The boot sector: a BPB of this disk, not executable (its checksum is not $1234). Serial and name are fixed.
    {
        let boot = &mut disk[..SECTOR];
        boot[0..2].copy_from_slice(&0x601Cu16.to_be_bytes());
        boot[2..8].copy_from_slice(b"RCLOAD");
        boot[8..11].copy_from_slice(&[0x52, 0x43, 0x4C]);
        let le = |b: &mut [u8], at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
        le(boot, 11, SECTOR as u16);
        boot[13] = 2;
        le(boot, 14, 1);
        boot[16] = 2;
        le(boot, 17, ROOT_ENTRIES as u16);
        le(boot, 19, SECTORS as u16);
        boot[21] = 0xF9;
        le(boot, 22, FAT_SECTORS as u16);
        le(boot, 24, 9);
        le(boot, 26, 2);
    }
    // The FAT: media descriptor, end of chain for the reserved entry, the AUTO folder (cluster 2) and the file chain.
    let file_clusters = prg.len().div_ceil(CLUSTER).max(1);
    let mut fat = vec![0u8; FAT_SECTORS * SECTOR];
    set_fat(&mut fat, 0, 0xFF9);
    set_fat(&mut fat, 1, END_OF_CHAIN);
    set_fat(&mut fat, 2, END_OF_CHAIN);
    for i in 0..file_clusters {
        let c = 3 + i;
        set_fat(
            &mut fat,
            c,
            if i + 1 == file_clusters {
                END_OF_CHAIN
            } else {
                (c + 1) as u16
            },
        );
    }
    for copy in 0..2 {
        let at = (1 + copy * FAT_SECTORS) * SECTOR;
        disk[at..at + fat.len()].copy_from_slice(&fat);
    }
    // The root directory: one entry, the AUTO folder.
    let root = (1 + 2 * FAT_SECTORS) * SECTOR;
    disk[root..root + 32].copy_from_slice(&entry(*b"AUTO       ", 0x10, 2, 0));
    // The AUTO folder (cluster 2): `.`, `..` and the program.
    let cluster_at = |c: usize| (DATA_START + (c - 2) * 2) * SECTOR;
    let auto = cluster_at(2);
    disk[auto..auto + 32].copy_from_slice(&entry(*b".          ", 0x10, 2, 0));
    disk[auto + 32..auto + 64].copy_from_slice(&entry(*b"..         ", 0x10, 0, 0));
    disk[auto + 64..auto + 96].copy_from_slice(&entry(short, 0x20, 3, prg.len() as u32));
    // The program.
    let at = cluster_at(3);
    disk[at..at + prg.len()].copy_from_slice(prg);
    Ok(Image::parse(&disk).expect("a 720K image with a valid BPB"))
}
