//! Builds a small CHD file for tests (here and in the disc crate): a two-track disc (data, then audio) in `cdzl` hunks of two
//! frames. Not a CHD writer: just enough of one to exercise the reader.

use crate::cd::{FRAME_SIZE, MAIN_SIZE};
use crate::{crc16, ecc};
use flate2::Compression;
use flate2::write::DeflateEncoder;
use std::io::Write;

pub(crate) const HUNK_FRAMES: usize = 2;
pub(crate) const HUNK_BYTES: usize = HUNK_FRAMES * FRAME_SIZE;
const LENGTH_BITS: u32 = 20;

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::best());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

/// A bit writer, most significant bit first.
#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    n: u32,
}

impl Bits {
    fn put(&mut self, value: u32, width: u32) {
        for i in (0..width).rev() {
            if self.n.is_multiple_of(8) {
                self.out.push(0);
            }
            if value >> i & 1 != 0 {
                *self.out.last_mut().unwrap() |= 0x80 >> (self.n % 8);
            }
            self.n += 1;
        }
    }
}

/// The frames of the test disc: 5 data frames (with real sync and parity) then 3 audio frames, each track padded to a multiple of
/// four frames in the file (so 8 + 4 = 12 frames, six hunks).
pub fn disc_frames() -> Vec<[u8; FRAME_SIZE]> {
    let mut frames = vec![[0u8; FRAME_SIZE]; 12];
    for (n, frame) in frames.iter_mut().enumerate().take(5) {
        frame[..12].copy_from_slice(&ecc::SYNC);
        frame[12..16].copy_from_slice(&[0, 2, n as u8, 1]);
        for (i, b) in frame[16..2064].iter_mut().enumerate() {
            *b = (i * 7 + n * 13) as u8;
        }
        let sector: &mut [u8; MAIN_SIZE] = (&mut frame[..MAIN_SIZE]).try_into().unwrap();
        ecc::generate(sector);
        frame[MAIN_SIZE..].fill(n as u8);
    }
    for (n, frame) in frames.iter_mut().enumerate().skip(8).take(3) {
        for (i, b) in frame[..MAIN_SIZE].iter_mut().enumerate() {
            *b = (i * 3 + n) as u8;
        }
        frame[MAIN_SIZE..].fill(0x40 + n as u8);
    }
    frames
}

/// One `cdzl` hunk: a flag bit per frame for sectors whose sync and parity were taken out, the base length, the base data, the subcode.
pub(crate) fn cdzl_hunk(frames: &[[u8; FRAME_SIZE]]) -> Vec<u8> {
    let mut flags = 0u8;
    let (mut main, mut sub) = (Vec::new(), Vec::new());
    for (i, f) in frames.iter().enumerate() {
        let mut sector = [0u8; MAIN_SIZE];
        sector.copy_from_slice(&f[..MAIN_SIZE]);
        if ecc::is_regenerable(&sector) {
            flags |= 1 << i;
            sector[..12].fill(0);
            sector[0x81C..].fill(0);
        }
        main.extend_from_slice(&sector);
        sub.extend_from_slice(&f[MAIN_SIZE..]);
    }
    let base = deflate(&main);
    let mut out = vec![flags];
    out.extend_from_slice(&(base.len() as u16).to_be_bytes());
    out.extend_from_slice(&base);
    out.extend_from_slice(&deflate(&sub));
    out
}

fn entry(tag: &[u8; 4], text: &str, next: u64) -> Vec<u8> {
    let mut data = text.as_bytes().to_vec();
    data.push(0);
    let mut out = tag.to_vec();
    out.push(1);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
    out.extend_from_slice(&next.to_be_bytes());
    out.extend_from_slice(&data);
    out
}

/// A complete file. `codec` is the tag of the first codec slot.
pub fn build(codec: &[u8; 4]) -> Vec<u8> {
    let frames = disc_frames();
    let hunks: Vec<Vec<u8>> = frames.chunks(HUNK_FRAMES).map(cdzl_hunk).collect();
    let mut body = Vec::new();
    let mut entries = Vec::new(); // (offset, length, crc)
    for (i, h) in hunks.iter().enumerate() {
        let raw: Vec<u8> = frames[i * HUNK_FRAMES..(i + 1) * HUNK_FRAMES].concat();
        entries.push((124 + body.len() as u64, h.len() as u32, crc16(&raw)));
        body.extend_from_slice(h);
    }
    // the map: the code (one symbol of one bit), every hunk's type, then each hunk's length and check value
    let mut bits = Bits::default();
    for width_value in [1u32, 1].into_iter().chain([0; 15]) {
        bits.put(width_value, 4);
    }
    for _ in &entries {
        bits.put(0, 1);
    }
    let mut rebuilt = Vec::new();
    for &(offset, length, crc) in &entries {
        bits.put(length, LENGTH_BITS);
        bits.put(u32::from(crc), 16);
        rebuilt.push(0);
        rebuilt.extend_from_slice(&length.to_be_bytes()[1..]);
        rebuilt.extend_from_slice(&offset.to_be_bytes()[2..]);
        rebuilt.extend_from_slice(&crc.to_be_bytes());
    }
    let mut map = Vec::new();
    map.extend_from_slice(&(bits.out.len() as u32).to_be_bytes());
    map.extend_from_slice(&entries[0].0.to_be_bytes()[2..]);
    map.extend_from_slice(&crc16(&rebuilt).to_be_bytes());
    map.extend_from_slice(&[LENGTH_BITS as u8, 0, 0, 0]);
    map.extend_from_slice(&bits.out);

    let map_offset = 124 + body.len() as u64;
    let meta_offset = map_offset + map.len() as u64;
    let first = entry(
        b"CHT2",
        "TRACK:1 TYPE:MODE1_RAW SUBTYPE:NONE FRAMES:5 PREGAP:0 PGTYPE:MODE1 PGSUB:NONE POSTGAP:0",
        0,
    );
    let second_at = meta_offset + first.len() as u64;
    let first = entry(
        b"CHT2",
        "TRACK:1 TYPE:MODE1_RAW SUBTYPE:NONE FRAMES:5 PREGAP:0 PGTYPE:MODE1 PGSUB:NONE POSTGAP:0",
        second_at,
    );
    let second = entry(
        b"CHT2",
        "TRACK:2 TYPE:AUDIO SUBTYPE:NONE FRAMES:3 PREGAP:2 PGTYPE:MODE1 PGSUB:NONE POSTGAP:0",
        0,
    );

    let mut file = Vec::new();
    file.extend_from_slice(b"MComprHD");
    file.extend_from_slice(&124u32.to_be_bytes());
    file.extend_from_slice(&5u32.to_be_bytes());
    file.extend_from_slice(codec);
    file.extend_from_slice(&[0; 12]);
    file.extend_from_slice(&((frames.len() * FRAME_SIZE) as u64).to_be_bytes());
    file.extend_from_slice(&map_offset.to_be_bytes());
    file.extend_from_slice(&meta_offset.to_be_bytes());
    file.extend_from_slice(&(HUNK_BYTES as u32).to_be_bytes());
    file.extend_from_slice(&(FRAME_SIZE as u32).to_be_bytes());
    file.extend_from_slice(&[0; 60]);
    assert_eq!(file.len(), 124);
    file.extend_from_slice(&body);
    file.extend_from_slice(&map);
    file.extend_from_slice(&first);
    file.extend_from_slice(&second);
    file
}
