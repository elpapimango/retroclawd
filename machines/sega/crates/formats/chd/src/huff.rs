//! Canonical Huffman codes as CHD uses them, and the `huff` hunk codec (one 8-bit Huffman code per hunk).
//!
//! The code lengths are what the file carries; the codes follow from them: lengths are taken from the longest down, each length starts
//! where the previous one's codes end (halved), so longer codes get lower values, and within one length the symbols take consecutive
//! values in order. A code must be complete (no unused code space) except that a lone one-bit code is allowed.
//!
//! A `huff` hunk is the tree followed by one code per byte, most significant bit first. The tree is sent in two stages: a small
//! Huffman code of 24 symbols (at most 6 bits per code; its lengths are plain 3-bit fields) that describes the 256 code lengths of
//! the byte code as runs. Small-code symbol 0 means "repeat the last length"; symbol n >= 1 means "the length n - 1".

use crate::bits::{BitReader, BitWriter};
use crate::{ChdError, format_err};

const BYTE_SYMBOLS: usize = 256;
/// The longest code of the byte code.
pub(crate) const BYTE_MAX_BITS: u32 = 16;
const SMALL_SYMBOLS: usize = 24;
const SMALL_MAX_BITS: u32 = 6;
/// Width of the field that extends a long repeat count: the bit length of `symbols - 9`.
const EXTRA_BITS: u32 = 8;

/// Code lengths for the histogram `hist` of at most `max_bits` bits (0 for a symbol that does not occur). The weights are scaled and the
/// scale raised for as long as the deepest code stays within the limit, so a skewed histogram is flattened just enough.
pub(crate) fn lengths_from_histogram(hist: &[u32], max_bits: u32) -> Vec<u8> {
    let total: u64 = hist.iter().map(|&h| u64::from(h)).sum();
    let used = hist.iter().filter(|&&h| h != 0).count();
    let mut lengths = vec![0u8; hist.len()];
    if used == 0 {
        return lengths;
    }
    if used == 1 {
        let at = hist.iter().position(|&h| h != 0).expect("one used");
        lengths[at] = 1;
        return lengths;
    }
    let flattest = |w: u64| -> Vec<u8> {
        let weights: Vec<u64> = hist
            .iter()
            .map(|&h| {
                if h == 0 {
                    0
                } else {
                    (u64::from(h) * w / total).max(1)
                }
            })
            .collect();
        depths(&weights)
    };
    // the largest scale whose deepest code still fits (a scale of 0 makes every weight 1: a balanced tree, which always fits)
    let (mut lo, mut hi) = (0u64, total * 2);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if flattest(mid).iter().all(|&l| u32::from(l) <= max_bits) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    flattest(lo)
}

/// The depth of every leaf of the Huffman tree of `weights` (0 weight: no leaf, depth 0).
fn depths(weights: &[u64]) -> Vec<u8> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    // (weight, order, node); `order` makes ties deterministic
    let mut heap = BinaryHeap::new();
    let mut parent: Vec<usize> = Vec::new();
    for (i, &w) in weights.iter().enumerate() {
        parent.push(usize::MAX);
        if w != 0 {
            heap.push(Reverse((w, i, i)));
        }
    }
    let mut order = weights.len();
    while heap.len() > 1 {
        let Reverse((wa, _, a)) = heap.pop().expect("two items");
        let Reverse((wb, _, b)) = heap.pop().expect("two items");
        let node = parent.len();
        parent.push(usize::MAX);
        parent[a] = node;
        parent[b] = node;
        heap.push(Reverse((wa + wb, order, node)));
        order += 1;
    }
    weights
        .iter()
        .enumerate()
        .map(|(i, &w)| {
            if w == 0 {
                return 0;
            }
            let (mut d, mut at) = (0u32, i);
            while parent[at] != usize::MAX {
                at = parent[at];
                d += 1;
            }
            d.max(1) as u8
        })
        .collect()
}

/// The code of every symbol (`None` for a symbol with length 0), or `None` when the lengths cannot form a code.
pub(crate) fn codes_from_lengths(lengths: &[u8], max_bits: u32) -> Option<Vec<(u32, u8)>> {
    let mut count = [0u32; 33];
    for &l in lengths {
        if u32::from(l) > max_bits {
            return None;
        }
        count[usize::from(l)] += 1;
    }
    count[0] = 0;
    let mut start = [0u32; 33];
    let mut next = 0u32;
    for len in (1..=32usize).rev() {
        let end = next.checked_add(count[len])?;
        if len != 1 && end % 2 != 0 {
            return None;
        }
        start[len] = next;
        next = end >> 1;
    }
    let mut out = Vec::with_capacity(lengths.len());
    for &l in lengths {
        if l == 0 {
            out.push((0, 0));
        } else {
            out.push((start[usize::from(l)], l));
            start[usize::from(l)] += 1;
        }
    }
    Some(out)
}

/// Decodes symbols from a `BitReader` for a set of code lengths.
pub(crate) struct Decoder {
    /// For each length: the first code, how many codes, and where this length's symbols start in `symbols`.
    start: [u32; 33],
    count: [u32; 33],
    first_symbol: [usize; 33],
    symbols: Vec<u16>,
    max: u32,
}

impl Decoder {
    pub(crate) fn new(lengths: &[u8], max_bits: u32) -> Option<Decoder> {
        let codes = codes_from_lengths(lengths, max_bits)?;
        let mut count = [0u32; 33];
        let mut start = [u32::MAX; 33];
        for (&(code, len), _) in codes.iter().zip(lengths) {
            if len != 0 {
                count[usize::from(len)] += 1;
                start[usize::from(len)] = start[usize::from(len)].min(code);
            }
        }
        let mut first_symbol = [0usize; 33];
        let mut at = 0usize;
        for len in 1..=32usize {
            first_symbol[len] = at;
            at += count[len] as usize;
        }
        // symbols grouped by length, in ascending symbol order inside a length (which is ascending code order)
        let mut symbols = vec![0u16; at];
        let mut fill = first_symbol;
        for (symbol, &len) in lengths.iter().enumerate() {
            if len != 0 {
                symbols[fill[usize::from(len)]] = symbol as u16;
                fill[usize::from(len)] += 1;
            }
        }
        Some(Decoder {
            start,
            count,
            first_symbol,
            symbols,
            max: lengths.iter().copied().max().map_or(0, u32::from),
        })
    }

    pub(crate) fn decode(&self, bits: &mut BitReader) -> Option<u16> {
        let mut value = 0u32;
        for len in 1..=self.max {
            value = (value << 1) | bits.read(1);
            let n = self.count[len as usize];
            if n != 0 && value >= self.start[len as usize] && value - self.start[len as usize] < n {
                let index =
                    self.first_symbol[len as usize] + (value - self.start[len as usize]) as usize;
                return self.symbols.get(index).copied();
            }
        }
        None
    }
}

/// Writes the 256 code lengths in the two-stage form.
fn write_tree(lengths: &[u8], out: &mut BitWriter) {
    // tokens: (small symbol, extra field value, extra field width)
    let mut tokens: Vec<(usize, u32, u32)> = Vec::new();
    let mut i = 0;
    while i < lengths.len() {
        let l = lengths[i];
        let mut run = 1;
        while i + run < lengths.len() && lengths[i + run] == l {
            run += 1;
        }
        tokens.push((usize::from(l) + 1, 0, 0));
        let mut repeats = run - 1;
        if repeats == 1 {
            tokens.push((usize::from(l) + 1, 0, 0));
        } else {
            while repeats >= 2 {
                let take = repeats.min(9 + (1 << EXTRA_BITS) - 1);
                let c = (take - 2).min(7) as u32;
                if c == 7 {
                    tokens.push((0, 7, 3));
                    tokens.push((usize::MAX, (take - 9) as u32, EXTRA_BITS));
                } else {
                    tokens.push((0, c, 3));
                }
                repeats -= take;
            }
            if repeats == 1 {
                tokens.push((usize::from(l) + 1, 0, 0));
            }
        }
        i += run;
    }
    let mut hist = [0u32; SMALL_SYMBOLS];
    for &(s, _, _) in &tokens {
        if s != usize::MAX {
            hist[s] += 1;
        }
    }
    let small = lengths_from_histogram(&hist, SMALL_MAX_BITS);
    let codes = codes_from_lengths(&small, SMALL_MAX_BITS).expect("a Huffman code is complete");
    out.put(u32::from(small[0]), 3);
    let first = small[1..].iter().position(|&l| l != 0).map_or(1, |p| p + 1);
    let last = small.iter().rposition(|&l| l != 0).unwrap_or(0).max(first);
    let first = first.min(8);
    out.put((first - 1) as u32, 3);
    for &l in &small[first..=last.min(SMALL_SYMBOLS - 1)] {
        out.put(u32::from(l), 3);
    }
    if last < SMALL_SYMBOLS - 1 {
        out.put(7, 3);
    }
    for (s, extra, width) in tokens {
        if s == usize::MAX {
            out.put(extra, width);
        } else {
            let (code, len) = codes[s];
            out.put(code, u32::from(len));
            if width != 0 {
                out.put(extra, width);
            }
        }
    }
}

fn read_tree(bits: &mut BitReader) -> Option<Vec<u8>> {
    let mut small = [0u8; SMALL_SYMBOLS];
    small[0] = bits.read(3) as u8;
    let start = bits.read(3) as usize + 1;
    for slot in small.iter_mut().skip(start) {
        let c = bits.read(3) as u8;
        if c == 7 {
            break;
        }
        *slot = c;
    }
    let decoder = Decoder::new(&small, SMALL_MAX_BITS)?;
    let mut lengths = vec![0u8; BYTE_SYMBOLS];
    let (mut at, mut last) = (0usize, 0u8);
    while at < BYTE_SYMBOLS {
        let symbol = decoder.decode(bits)?;
        if symbol != 0 {
            last = (symbol - 1) as u8;
            lengths[at] = last;
            at += 1;
        } else {
            let mut count = bits.read(3) as usize + 2;
            if count == 9 {
                count += bits.read(EXTRA_BITS) as usize;
            }
            let end = (at + count).min(BYTE_SYMBOLS);
            lengths[at..end].fill(last);
            at = end;
        }
        if bits.overflowed() {
            return None;
        }
    }
    Some(lengths)
}

/// Compresses `data` with one Huffman code; `None` when the result is not smaller than the input.
pub(crate) fn compress(data: &[u8]) -> Option<Vec<u8>> {
    let mut hist = vec![0u32; BYTE_SYMBOLS];
    for &b in data {
        hist[usize::from(b)] += 1;
    }
    let lengths = lengths_from_histogram(&hist, BYTE_MAX_BITS);
    let codes = codes_from_lengths(&lengths, BYTE_MAX_BITS)?;
    let mut out = BitWriter::new();
    write_tree(&lengths, &mut out);
    for &b in data {
        let (code, len) = codes[usize::from(b)];
        out.put(code, u32::from(len));
    }
    let bytes = out.finish();
    (bytes.len() < data.len()).then_some(bytes)
}

/// Decompresses to exactly `size` bytes.
pub(crate) fn decompress(src: &[u8], size: usize) -> Result<Vec<u8>, ChdError> {
    let bad =
        || ChdError::Format("a hunk fails to decode: a Huffman stream is damaged".to_string());
    let mut bits = BitReader::new(src);
    let lengths = read_tree(&mut bits).ok_or_else(bad)?;
    let decoder = Decoder::new(&lengths, BYTE_MAX_BITS).ok_or_else(bad)?;
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        out.push(decoder.decode(&mut bits).ok_or_else(bad)? as u8);
    }
    if bits.overflowed() {
        return format_err("a hunk fails to decode: a Huffman stream ends early");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kraft_complete(lengths: &[u8]) -> bool {
        let used: Vec<u8> = lengths.iter().copied().filter(|&l| l != 0).collect();
        used.len() < 2
            || used
                .iter()
                .map(|&l| 1u64 << (32 - u32::from(l)))
                .sum::<u64>()
                == 1 << 32
    }

    #[test]
    fn lengths_are_complete_and_limited() {
        // a very skewed histogram that would need codes longer than the limit if built plainly
        let mut hist = vec![0u32; 256];
        for (i, h) in hist.iter_mut().enumerate().take(40) {
            *h = 1 << (i / 2).min(30);
        }
        let l = lengths_from_histogram(&hist, 16);
        assert!(l.iter().all(|&x| x <= 16));
        assert!(kraft_complete(&l));
        assert!(l[..40].iter().all(|&x| x > 0) && l[40..].iter().all(|&x| x == 0));
        // one symbol gets a one-bit code
        let mut one = vec![0u32; 256];
        one[7] = 100;
        assert_eq!(lengths_from_histogram(&one, 16)[7], 1);
    }

    #[test]
    fn canonical_codes_put_longer_codes_lower() {
        // lengths 1, 2, 2 give the second symbol 00, the third 01 and the first 1
        let codes = codes_from_lengths(&[1, 2, 2], 8).unwrap();
        assert_eq!(codes, [(1, 1), (0, 2), (1, 2)]);
        // an incomplete code is refused
        assert!(codes_from_lengths(&[2, 2, 2], 8).is_none());
    }

    #[test]
    fn data_round_trips_through_the_codec() {
        let skewed: Vec<u8> = (0..4096u32)
            .map(|i| match i % 11 {
                0..=6 => 0,
                7 | 8 => 0x41,
                9 => (i / 11) as u8,
                _ => 0xFF,
            })
            .collect();
        let packed = compress(&skewed).expect("compressible");
        assert!(packed.len() < skewed.len() / 2);
        assert_eq!(decompress(&packed, skewed.len()).unwrap(), skewed);
        // all 256 byte values, each once: no gain, so no output
        let flat: Vec<u8> = (0..=255u8).collect();
        assert!(compress(&flat).is_none());
        // one value over and over
        let same = vec![9u8; 2048];
        let packed = compress(&same).unwrap();
        assert_eq!(decompress(&packed, 2048).unwrap(), same);
        // a damaged stream is an error, not a panic
        assert!(decompress(&packed[..packed.len() / 2], 2048).is_err());
        assert!(decompress(&[0xFF; 40], 2048).is_err());
    }

    #[test]
    fn long_runs_of_one_length_use_the_extended_count() {
        // 300 identical code lengths do not exist (256 symbols), but 256 equal lengths of 8 bits do: one literal and one long repeat
        let data: Vec<u8> = (0..=255u8).cycle().take(2048 * 3).collect();
        // flat histogram: every symbol 8 bits; no gain, so go through the tree writer directly
        let lengths = vec![8u8; 256];
        let mut w = BitWriter::new();
        write_tree(&lengths, &mut w);
        let bytes = w.finish();
        assert!(bytes.len() < 12);
        assert_eq!(read_tree(&mut BitReader::new(&bytes)).unwrap(), lengths);
        let _ = data;
    }
}
