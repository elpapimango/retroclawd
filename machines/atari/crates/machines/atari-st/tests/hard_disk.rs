//! Hard disk images behind ACSI: what an image may be, and that two targets are two disks.

use atari_st::acsi::{BLOCK_BYTES, BlockDevice, MAX_DISK_BYTES, MemoryDisk};

#[test]
fn an_image_larger_than_the_cap_is_refused() {
    // Checked on the length, so no 512 MiB buffer is needed to see it.
    let big = vec![0u8; (MAX_DISK_BYTES as usize) + BLOCK_BYTES];
    let e = MemoryDisk::from_image(big).unwrap_err();
    assert!(e.contains("512 MiB"), "{e}");
    assert!(MemoryDisk::from_image(vec![0; 2 * BLOCK_BYTES]).is_ok());
}

#[test]
fn an_image_not_a_multiple_of_512_is_an_error() {
    let e = MemoryDisk::from_image(vec![0; 1000]).unwrap_err();
    assert!(e.contains("512"), "{e}");
    assert!(
        MemoryDisk::from_image(Vec::new())
            .unwrap_err()
            .contains("empty")
    );
}

#[test]
fn a_guest_write_is_exported_dirty_and_saving_clears_it() {
    let mut d = MemoryDisk::from_image(vec![0; 4 * BLOCK_BYTES]).unwrap();
    assert!(!d.is_modified());
    d.write(1, &[7u8; 512]).unwrap();
    assert!(d.is_modified());
    assert_eq!(&d.image()[512..1024], &[7u8; 512]);
    d.mark_saved();
    assert!(!d.is_modified());
    assert!(d.write(4, &[0u8; 512]).is_err(), "past the end");
    assert!(
        d.write(u64::MAX, &[0u8; 512]).is_err(),
        "an address that overflows"
    );
    let mut out = [0u8; 512];
    assert!(d.read(u64::MAX / 512, &mut out).is_err());
}
