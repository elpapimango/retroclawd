//! Shifter registers and the frame render (facts: `docs/hardware-facts.md`, "Shifter").

use st_shifter::{HEIGHT, Monitor, Resolution, Shifter, WIDTH};

fn word(s: &mut Shifter, addr: u32, value: u16) {
    s.write(addr, true, value);
}

fn byte(s: &mut Shifter, addr: u32, value: u8) {
    s.write(addr, false, value as u16);
}

/// A 32K video RAM at `$F8000` built from `(offset, word)` pairs.
fn ram(words: &[(u32, u16)]) -> impl Fn(u32) -> u8 {
    let mut mem = vec![0u8; 32_000];
    for &(off, w) in words {
        mem[off as usize..off as usize + 2].copy_from_slice(&w.to_be_bytes());
    }
    move |addr| {
        mem.get((addr.wrapping_sub(0xF8000)) as usize)
            .copied()
            .unwrap_or(0)
    }
}

fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * WIDTH + x) * 3) as usize;
    [frame[i], frame[i + 1], frame[i + 2]]
}

fn shifter_at_f8000() -> Shifter {
    let mut s = Shifter::new(true);
    byte(&mut s, 0xFF8201, 0x0F);
    byte(&mut s, 0xFF8203, 0x80);
    s
}

#[test]
fn video_base_from_high_and_mid_bytes() {
    let s = shifter_at_f8000();
    assert_eq!(s.video_base(), 0x0F8000);
    assert_eq!(s.read(0xFF8201, false), Some(0x0F));
    assert_eq!(s.read(0xFF8203, false), Some(0x80));
}

#[test]
fn sync_and_resolution_registers() {
    let mut s = Shifter::new(true);
    assert!(s.sync_50hz(), "PAL power-on [H?]");
    byte(&mut s, 0xFF820A, 0x00);
    assert!(!s.sync_50hz());
    byte(&mut s, 0xFF820A, 0xFE);
    assert!(s.sync_50hz());
    assert_eq!(s.read(0xFF820A, false), Some(0x02), "bits 7-2 read 0 [H?]");
    byte(&mut s, 0xFF8260, 0x01);
    assert_eq!(s.resolution(), Resolution::Medium);
    assert!(!Shifter::new(false).sync_50hz(), "NTSC power-on");
}

#[test]
fn palette_keeps_nine_bits() {
    let mut s = Shifter::new(true);
    word(&mut s, 0xFF8240, 0xFFFF);
    assert_eq!(s.palette()[0], 0x0777);
    assert_eq!(s.read(0xFF8240, true), Some(0x0777));
    byte(&mut s, 0xFF8243, 0x25);
    assert_eq!(s.palette()[1], 0x0025, "a byte write to the odd half");
    byte(&mut s, 0xFF8242, 0x07);
    assert_eq!(s.palette()[1], 0x0725);
    assert_eq!(s.read(0xFF8200, false), None, "not a Shifter register");
}

/// Low resolution: four words per 16 pixels, plane 0 = bit 0 of the colour index, the most significant bit the
/// leftmost pixel; each pixel 2 × 2 on the canvas.
#[test]
fn low_res_plane_order_and_pixel_doubling() {
    let mut s = shifter_at_f8000();
    for (i, rgb) in [
        (0u32, 0x000),
        (1, 0x700),
        (2, 0x070),
        (4, 0x007),
        (15, 0x777),
    ] {
        word(&mut s, 0xFF8240 + 2 * i, rgb);
    }
    // Pixel 0 colour 1 (plane 0), pixel 1 colour 2 (plane 1), pixel 2 colour 4 (plane 2), pixel 15 colour 15.
    let mem = ram(&[(0, 0x8001), (2, 0x4001), (4, 0x2001), (6, 0x0001)]);
    let mut frame = Vec::new();
    s.render(Monitor::Colour, &mem, &mut frame);
    assert_eq!(frame.len(), (WIDTH * HEIGHT * 3) as usize);
    assert_eq!(pixel(&frame, 0, 0), [255, 0, 0]);
    assert_eq!(pixel(&frame, 1, 1), [255, 0, 0], "doubled");
    assert_eq!(pixel(&frame, 2, 0), [0, 255, 0]);
    assert_eq!(pixel(&frame, 4, 1), [0, 0, 255]);
    assert_eq!(pixel(&frame, 30, 0), [255, 255, 255]);
    assert_eq!(pixel(&frame, 6, 0), [0, 0, 0]);
}

#[test]
fn medium_res_two_planes() {
    let mut s = shifter_at_f8000();
    byte(&mut s, 0xFF8260, 1);
    word(&mut s, 0xFF8240, 0x000);
    word(&mut s, 0xFF8246, 0x777);
    // Second line of the screen (160 bytes per line): pixel 0 colour 3.
    let mem = ram(&[(160, 0x8000), (162, 0x8000)]);
    let mut frame = Vec::new();
    s.render(Monitor::Colour, &mem, &mut frame);
    assert_eq!(pixel(&frame, 0, 2), [255, 255, 255]);
    assert_eq!(pixel(&frame, 0, 3), [255, 255, 255], "line doubled");
    assert_eq!(pixel(&frame, 1, 2), [0, 0, 0]);
    assert_eq!(pixel(&frame, 0, 0), [0, 0, 0]);
}

/// High resolution: one plane, 80 bytes per line; a set bit is dark unless palette 0 bit 0 inverts.
#[test]
fn high_res_mono_and_inversion() {
    let mut s = shifter_at_f8000();
    byte(&mut s, 0xFF8260, 2);
    let mem = ram(&[(80, 0x8000)]);
    let mut frame = Vec::new();
    s.render(Monitor::Mono, &mem, &mut frame);
    assert_eq!(pixel(&frame, 0, 1), [0, 0, 0], "set bit: dark");
    assert_eq!(pixel(&frame, 1, 1), [255, 255, 255]);
    word(&mut s, 0xFF8240, 0x001);
    s.render(Monitor::Mono, &mem, &mut frame);
    assert_eq!(pixel(&frame, 0, 1), [255, 255, 255], "inverted");
}

#[test]
fn colour_monitor_high_res_is_black() {
    let mut s = shifter_at_f8000();
    byte(&mut s, 0xFF8260, 2);
    word(&mut s, 0xFF8240, 0x777);
    let mut frame = Vec::new();
    s.render(Monitor::Colour, &ram(&[]), &mut frame);
    assert!(frame.iter().all(|&b| b == 0));
    byte(&mut s, 0xFF8260, 0);
    s.render(Monitor::Mono, &ram(&[]), &mut frame);
    assert!(
        frame.iter().all(|&b| b == 0),
        "mono monitor in low resolution"
    );
}

#[test]
fn render_is_deterministic() {
    let mut s = shifter_at_f8000();
    word(&mut s, 0xFF8242, 0x123);
    let mem = ram(&[(0, 0xAAAA), (100, 0x5555)]);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    s.render(Monitor::Colour, &mem, &mut a);
    s.render(Monitor::Colour, &mem, &mut b);
    assert_eq!(a, b);
}
