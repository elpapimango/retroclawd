//! A FLAC encoder for 16-bit stereo audio at 44.1 kHz, writing bare frames (no stream header, as CHD stores them).
//!
//! Each frame picks the cheaper of the four stereo layouts (left/right, left/side, side/right, mid/side), and each channel the cheapest of
//! a constant, a verbatim and the five fixed predictors (orders 0 to 4) with a Rice-coded residual whose partitions are searched. Every
//! frame is a valid FLAC frame, so any decoder reads it; the ratio is that of a simple encoder, not of the reference one.

use crate::bits::BitWriter;

const MAX_RICE: u32 = 14;
const MAX_PARTITION_ORDER: u32 = 8;
/// Fixed predictors of orders 0 to 4 (FLAC format: the coefficients of the polynomial predictors).
const FIXED: [&[i64]; 5] = [&[], &[1], &[2, -1], &[3, -3, 1], &[4, -6, 4, -1]];

/// A subframe decided but not yet written.
struct Plan {
    kind: Kind,
    bits: u64,
}

enum Kind {
    Constant,
    Verbatim,
    /// Fixed predictor order, partition order and the Rice parameter of each partition.
    Fixed(usize, u32, Vec<u32>),
}

fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn residual(x: &[i32], order: usize) -> Vec<i64> {
    (order..x.len())
        .map(|i| {
            let predicted: i64 = FIXED[order]
                .iter()
                .enumerate()
                .map(|(j, &c)| c * i64::from(x[i - 1 - j]))
                .sum();
            i64::from(x[i]) - predicted
        })
        .collect()
}

fn zigzag(r: i64) -> u64 {
    ((r << 1) ^ (r >> 63)) as u64
}

/// The bits a partition costs with Rice parameter `k`, and the best `k`.
fn best_rice(folded: &[u64]) -> (u32, u64) {
    let sum: u64 = folded.iter().sum();
    let n = folded.len().max(1) as u64;
    let guess = (64 - (sum / n).leading_zeros()).min(MAX_RICE);
    let cost = |k: u32| -> u64 { folded.iter().map(|&u| (u >> k) + 1 + u64::from(k)).sum() };
    let lo = guess.saturating_sub(1);
    let hi = (guess + 1).min(MAX_RICE);
    (lo..=hi)
        .map(|k| (k, cost(k)))
        .min_by_key(|&(_, c)| c)
        .expect("at least one")
}

fn plan_fixed(x: &[i32], bps: u32, order: usize) -> Option<Plan> {
    if x.len() <= order {
        return None;
    }
    let res = residual(x, order);
    let folded: Vec<u64> = res.iter().map(|&r| zigzag(r)).collect();
    let mut best: Option<(u64, u32, Vec<u32>)> = None;
    for part_order in 0..=MAX_PARTITION_ORDER {
        let parts = 1usize << part_order;
        if !x.len().is_multiple_of(parts) || x.len() / parts <= order {
            break;
        }
        let size = x.len() / parts;
        let (mut total, mut params) = (0u64, Vec::new());
        for p in 0..parts {
            let from = if p == 0 { 0 } else { p * size - order };
            let to = (p + 1) * size - order;
            let (k, bits) = best_rice(&folded[from..to]);
            total += bits + 4;
            params.push(k);
        }
        if best.as_ref().is_none_or(|b| total < b.0) {
            best = Some((total, part_order, params));
        }
    }
    let (rice_bits, part_order, params) = best?;
    // subframe header 8, warm-up samples, residual coding method 2, partition order 4, the partitions
    let bits = 8 + order as u64 * u64::from(bps) + 2 + 4 + rice_bits;
    Some(Plan {
        kind: Kind::Fixed(order, part_order, params),
        bits,
    })
}

fn plan(x: &[i32], bps: u32) -> Plan {
    let mut best = Plan {
        kind: Kind::Verbatim,
        bits: 8 + x.len() as u64 * u64::from(bps),
    };
    if x.iter().all(|&v| v == x[0]) {
        return Plan {
            kind: Kind::Constant,
            bits: 8 + u64::from(bps),
        };
    }
    for order in 0..=4 {
        if let Some(p) = plan_fixed(x, bps, order)
            && p.bits < best.bits
        {
            best = p;
        }
    }
    best
}

fn put_signed(w: &mut BitWriter, v: i32, bits: u32) {
    w.put((v as u32) & (u32::MAX >> (32 - bits)), bits);
}

fn write_subframe(w: &mut BitWriter, x: &[i32], bps: u32, plan: &Plan) {
    match &plan.kind {
        Kind::Constant => {
            w.put(0, 8);
            put_signed(w, x[0], bps);
        }
        Kind::Verbatim => {
            w.put(0b0000_0010, 8);
            for &v in x {
                put_signed(w, v, bps);
            }
        }
        Kind::Fixed(order, part_order, params) => {
            w.put((8 + *order as u32) << 1, 8);
            for &v in &x[..*order] {
                put_signed(w, v, bps);
            }
            w.put(0, 2);
            w.put(*part_order, 4);
            let res = residual(x, *order);
            let size = x.len() >> part_order;
            for (p, &k) in params.iter().enumerate() {
                w.put(k, 4);
                let from = if p == 0 { 0 } else { p * size - order };
                let to = (p + 1) * size - order;
                for &r in &res[from..to] {
                    let u = zigzag(r);
                    let q = u >> k;
                    for _ in 0..q {
                        w.put(0, 1);
                    }
                    w.put(1, 1);
                    if k > 0 {
                        w.put((u & ((1 << k) - 1)) as u32, k);
                    }
                }
            }
        }
    }
}

fn block_size_code(n: usize) -> (u32, Option<(u32, u32)>) {
    match n {
        192 => (1, None),
        576 => (2, None),
        1152 => (3, None),
        2304 => (4, None),
        4608 => (5, None),
        256 => (8, None),
        512 => (9, None),
        1024 => (10, None),
        2048 => (11, None),
        4096 => (12, None),
        8192 => (13, None),
        16384 => (14, None),
        32768 => (15, None),
        n if n <= 256 => (6, Some((n as u32 - 1, 8))),
        n => (7, Some((n as u32 - 1, 16))),
    }
}

/// The frame number as FLAC's UTF-8-like variable length code (up to 36 bits; frame numbers here are far smaller).
fn utf8_number(n: u32, out: &mut Vec<u8>) {
    if n < 0x80 {
        out.push(n as u8);
        return;
    }
    let len = match n {
        0..0x800 => 2,
        0x800..0x1_0000 => 3,
        0x1_0000..0x20_0000 => 4,
        0x20_0000..0x400_0000 => 5,
        _ => 6,
    };
    out.push(((0xFF00u32 >> len) as u8) | (n >> (6 * (len - 1))) as u8);
    for i in (0..len - 1).rev() {
        out.push(0x80 | ((n >> (6 * i)) & 0x3F) as u8);
    }
}

/// One frame of `left.len()` samples: the header, the cheapest stereo layout, the subframes, the CRC.
fn frame(number: u32, left: &[i32], right: &[i32]) -> Vec<u8> {
    let n = left.len();
    let mid: Vec<i32> = left
        .iter()
        .zip(right)
        .map(|(&l, &r)| (l + r) >> 1)
        .collect();
    let side: Vec<i32> = left.iter().zip(right).map(|(&l, &r)| l - r).collect();
    let (pl, pr) = (plan(left, 16), plan(right, 16));
    let (pm, ps) = (plan(&mid, 16), plan(&side, 17));
    // assignment code, the two channels, their widths and plans
    let layouts = [
        (1u32, pl.bits + pr.bits),
        (8, pl.bits + ps.bits),
        (9, ps.bits + pr.bits),
        (10, pm.bits + ps.bits),
    ];
    let (assignment, _) = *layouts.iter().min_by_key(|l| l.1).expect("four layouts");

    let mut header = vec![0xFF, 0xF8];
    let (code, extra) = block_size_code(n);
    header.push(((code << 4) | 9) as u8);
    header.push(((assignment << 4) | (0b100 << 1)) as u8);
    utf8_number(number, &mut header);
    if let Some((value, width)) = extra {
        if width == 8 {
            header.push(value as u8);
        } else {
            header.extend_from_slice(&(value as u16).to_be_bytes());
        }
    }
    header.push(crc8(&header));

    let mut w = BitWriter::new();
    let (first, second, bps_first, bps_second, plan_first, plan_second): (
        &[i32],
        &[i32],
        u32,
        u32,
        &Plan,
        &Plan,
    ) = match assignment {
        1 => (left, right, 16, 16, &pl, &pr),
        8 => (left, &side, 16, 17, &pl, &ps),
        9 => (&side, right, 17, 16, &ps, &pr),
        _ => (&mid, &side, 16, 17, &pm, &ps),
    };
    write_subframe(&mut w, first, bps_first, plan_first);
    write_subframe(&mut w, second, bps_second, plan_second);
    let mut out = header;
    out.extend(w.finish());
    out.extend_from_slice(&crc16(&out).to_be_bytes());
    out
}

/// Encodes interleaved 16-bit stereo samples (`left, right` pairs) in blocks of `block` samples (the last may be shorter).
pub(crate) fn encode(samples: &[(i16, i16)], block: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for (number, chunk) in samples.chunks(block).enumerate() {
        let left: Vec<i32> = chunk.iter().map(|s| i32::from(s.0)).collect();
        let right: Vec<i32> = chunk.iter().map(|s| i32::from(s.1)).collect();
        out.extend(frame(number as u32, &left, &right));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn decode(bytes: &[u8], count: usize) -> Vec<(i16, i16)> {
        let mut cursor = Cursor::new(bytes);
        let mut frames = claxon::frame::FrameReader::new(&mut cursor);
        let (mut out, mut buffer) = (Vec::new(), Vec::new());
        while out.len() < count {
            let block = frames
                .read_next_or_eof(std::mem::take(&mut buffer))
                .unwrap()
                .expect("frames");
            for i in 0..block.duration() {
                out.push((block.sample(0, i) as i16, block.sample(1, i) as i16));
            }
            buffer = block.into_buffer();
        }
        assert_eq!(
            cursor.position() as usize,
            bytes.len(),
            "the frames take exactly the stream"
        );
        out
    }

    fn tone(n: usize, amp: f64) -> Vec<(i16, i16)> {
        (0..n)
            .map(|i| {
                let t = i as f64 / 44100.0;
                (
                    (amp * (t * 440.0 * std::f64::consts::TAU).sin()) as i16,
                    (amp * (t * 660.0 * std::f64::consts::TAU).sin() * 0.5) as i16,
                )
            })
            .collect()
    }

    #[test]
    fn every_kind_of_signal_decodes_to_what_went_in() {
        let mut noise = 12345u32;
        let hash: Vec<(i16, i16)> = (0..4704)
            .map(|_| {
                noise = noise.wrapping_mul(1664525).wrapping_add(1013904223);
                ((noise >> 16) as i16, (noise >> 3) as i16)
            })
            .collect();
        let extremes: Vec<(i16, i16)> = (0..600)
            .map(|i| {
                if i % 2 == 0 {
                    (i16::MIN, i16::MAX)
                } else {
                    (i16::MAX, i16::MIN)
                }
            })
            .collect();
        type Case = (&'static str, Vec<(i16, i16)>, usize);
        let cases: Vec<Case> = vec![
            ("tone", tone(4704, 12000.0), 2352),
            ("silence", vec![(0, 0); 1000], 1000),
            ("noise", hash, 2352),
            ("extremes", extremes, 600),
            ("short last block", tone(1000, 8000.0), 384),
            ("tiny", tone(5, 100.0), 5),
            ("block over 256", tone(700, 3000.0), 700),
        ];
        for (name, samples, block) in cases {
            let bytes = encode(&samples, block);
            assert_eq!(decode(&bytes, samples.len()), samples, "{name}");
        }
    }

    #[test]
    fn a_tone_takes_a_fraction_of_its_size() {
        let samples = tone(4704, 12000.0);
        let bytes = encode(&samples, 2352);
        assert!(bytes.len() < samples.len() * 4 / 2, "{} bytes", bytes.len());
    }

    #[test]
    fn frame_numbers_beyond_one_byte_are_coded_in_utf8_style() {
        let mut v = Vec::new();
        utf8_number(0x7F, &mut v);
        utf8_number(0x80, &mut v);
        utf8_number(0x7FF, &mut v);
        utf8_number(0x800, &mut v);
        assert_eq!(v, [0x7F, 0xC2, 0x80, 0xDF, 0xBF, 0xE0, 0xA0, 0x80]);
    }
}
