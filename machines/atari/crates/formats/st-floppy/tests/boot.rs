//! The floppy built in memory to run one program: `AUTO\NAME.PRG` on a FAT12 disk.

use st_floppy::Image;
use st_floppy::boot::{BootError, MAX_PROGRAM_BYTES, with_auto_program};

/// A GEMDOS program: the header, `size` bytes of text, a zero fixup long.
fn prg(size: usize) -> Vec<u8> {
    let mut p = vec![0u8; 28];
    p[0] = 0x60;
    p[1] = 0x1A;
    p[2..6].copy_from_slice(&(size as u32).to_be_bytes());
    p.extend((0..size).map(|i| (i * 7 + 3) as u8));
    p.extend([0, 0, 0, 0]);
    p
}

fn bytes(image: &Image) -> Vec<u8> {
    image.to_bytes()
}

fn le16(b: &[u8], at: usize) -> usize {
    u16::from_le_bytes([b[at], b[at + 1]]) as usize
}

/// The FAT12 entry of `cluster`.
fn fat(disk: &[u8], cluster: usize) -> usize {
    let at = 512 + cluster * 3 / 2;
    let pair = le16(disk, at);
    if cluster.is_multiple_of(2) {
        pair & 0xFFF
    } else {
        pair >> 4
    }
}

/// The 32-byte entries of a directory held in `data` that are in use: (name, attr, first cluster, size).
fn entries(data: &[u8]) -> Vec<(String, u8, usize, usize)> {
    data.chunks(32)
        .take_while(|e| e[0] != 0)
        .filter(|e| e[0] != 0xE5)
        .map(|e| {
            (
                String::from_utf8_lossy(&e[..11]).to_string(),
                e[11],
                le16(e, 26),
                u32::from_le_bytes(e[28..32].try_into().unwrap()) as usize,
            )
        })
        .collect()
}

/// The bytes of the chain starting at `first`, cut to `size` (a directory reads whole clusters).
fn chain(disk: &[u8], first: usize, size: Option<usize>) -> Vec<u8> {
    let (data_start, mut out, mut c, mut seen) = (14usize, Vec::new(), first, 0);
    loop {
        assert!(seen < 800, "a cyclic chain");
        seen += 1;
        let at = (data_start + (c - 2) * 2) * 512;
        out.extend(&disk[at..at + 1024]);
        let next = fat(disk, c);
        if next >= 0xFF8 {
            break;
        }
        c = next;
    }
    if let Some(n) = size {
        out.truncate(n);
    }
    out
}

#[test]
fn with_auto_program_lists_the_file_under_auto() {
    let program = prg(3000);
    let image = with_auto_program("hello", &program).unwrap();
    let disk = bytes(&image);
    assert_eq!(disk.len(), 737_280);
    assert_eq!(
        (
            image.geometry.cylinders,
            image.geometry.sides,
            image.geometry.sectors
        ),
        (80, 2, 9)
    );
    let root = &disk[(1 + 6) * 512..(1 + 6 + 7) * 512];
    let root_entries = entries(root);
    assert_eq!(root_entries.len(), 1);
    let (name, attr, cluster, _) = &root_entries[0];
    assert_eq!(name, "AUTO       ");
    assert_eq!(*attr, 0x10, "a directory");
    let auto = chain(&disk, *cluster, None);
    let listed: Vec<_> = entries(&auto).into_iter().map(|e| e.0).collect();
    assert_eq!(listed, [".          ", "..         ", "HELLO   PRG"]);
    let file = &entries(&auto)[2];
    assert_eq!(file.3, program.len());
    assert_eq!(
        chain(&disk, file.2, Some(file.3)),
        program,
        "the file's bytes come back through the FAT"
    );
}

#[test]
fn the_boot_sector_describes_the_disk_and_is_not_executable() {
    let disk = bytes(&with_auto_program("p.prg", &prg(10)).unwrap());
    assert_eq!(le16(&disk, 11), 512, "bytes per sector");
    assert_eq!(disk[13], 2, "sectors per cluster");
    assert_eq!(le16(&disk, 19), 1440, "sectors");
    assert_eq!(disk[21], 0xF9, "media");
    assert_eq!(le16(&disk, 24), 9);
    assert_eq!(le16(&disk, 26), 2);
    let sum = disk[..512].chunks(2).fold(0u16, |s, w| {
        s.wrapping_add(u16::from_be_bytes([w[0], w[1]]))
    });
    assert_ne!(sum, 0x1234, "TOS must not run the boot sector as code");
}

#[test]
fn both_fats_agree_and_unused_clusters_are_free() {
    let disk = bytes(&with_auto_program("p.prg", &prg(2500)).unwrap());
    assert_eq!(&disk[512..512 + 1536], &disk[512 + 1536..512 + 3072]);
    assert_eq!(fat(&disk, 0), 0xFF9);
    assert_eq!(fat(&disk, 2), 0xFFF, "the folder is one cluster");
    // 2500 + 28 + 4 bytes is 3 clusters: 3 -> 4 -> 5 -> end.
    assert_eq!((fat(&disk, 3), fat(&disk, 4), fat(&disk, 5)), (4, 5, 0xFFF));
    assert_eq!(fat(&disk, 6), 0, "free");
}

#[test]
fn the_same_program_gives_the_same_disk() {
    let p = prg(999);
    assert_eq!(
        bytes(&with_auto_program("a.prg", &p).unwrap()),
        bytes(&with_auto_program("a.prg", &p).unwrap())
    );
}

#[test]
fn a_prg_larger_than_the_disk_is_an_error() {
    let big = prg(MAX_PROGRAM_BYTES);
    assert!(matches!(
        with_auto_program("big.prg", &big),
        Err(BootError::TooLarge { .. })
    ));
    // The largest that fits does.
    let mut fits = prg(0);
    fits.resize(MAX_PROGRAM_BYTES, 7);
    let image = with_auto_program("fits.prg", &fits).unwrap();
    let disk = bytes(&image);
    let auto = chain(&disk, 2, None);
    let file = &entries(&auto)[2];
    assert_eq!(chain(&disk, file.2, Some(file.3)), fits);
}

#[test]
fn names_and_non_programs_are_refused() {
    let p = prg(10);
    for bad in [
        "",
        ".prg",
        "toolongname.prg",
        "a.prgx",
        "sp ace.prg",
        "a/b.prg",
        "ünï.prg",
    ] {
        assert!(
            matches!(with_auto_program(bad, &p), Err(BootError::BadName(_))),
            "{bad:?}"
        );
    }
    assert!(
        with_auto_program("NoExt", &p).is_ok(),
        "the extension is added"
    );
    let disk = bytes(&with_auto_program("lower.tos", &p).unwrap());
    let auto = chain(&disk, 2, None);
    assert_eq!(entries(&auto)[2].0, "LOWER   TOS", "upper-cased");
    assert_eq!(with_auto_program("p.prg", &[]), Err(BootError::NotAProgram));
    assert_eq!(
        with_auto_program("p.prg", &[0x4E; 40]),
        Err(BootError::NotAProgram)
    );
    assert_eq!(
        with_auto_program("p.prg", &[0x60, 0x1A, 0, 0]),
        Err(BootError::NotAProgram)
    );
}
