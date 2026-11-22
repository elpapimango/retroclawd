//! Writing the compressed map: the inverse of `map::compressed_map`. Each hunk's type is chosen (a reference that follows the previous
//! one costs no bits of its own), runs of one type become run codes, the 16 type and run symbols get a Huffman code, and the lengths,
//! check values and references follow in hunk order.

use crate::bits::BitWriter;
use crate::crc16;
use crate::huff::{codes_from_lengths, lengths_from_histogram};
use crate::map::{Entry, Kind};

const SYMBOLS: usize = 16;
const MAX_BITS: u32 = 8;
const TYPE_PLAIN: u8 = 4;
const TYPE_SELF: u8 = 5;
const TYPE_PARENT: u8 = 6;
const TYPE_RLE_SMALL: u8 = 7;
const TYPE_RLE_LARGE: u8 = 8;
const TYPE_SELF_0: u8 = 9;
const TYPE_SELF_1: u8 = 10;
const TYPE_PARENT_SELF: u8 = 11;
const TYPE_PARENT_0: u8 = 12;
const TYPE_PARENT_1: u8 = 13;

fn bit_width(v: u64) -> u32 {
    64 - v.leading_zeros()
}

/// One hunk as the map's type stream names it, with the fields that follow in the second part of the stream.
struct Coded {
    symbol: u8,
    /// A codec hunk's compressed length.
    length: Option<u32>,
    crc: Option<u16>,
    /// An explicit self or parent reference.
    reference: Option<u64>,
}

/// The tokens of the type stream: a type literal or a run code with the run's value symbols.
enum Token {
    Literal(u8),
    Small(u8),
    Large(u8, u8),
}

/// Encodes the map of `entries` (hunks in order, codec and plain data laid end to end from the first one's offset).
pub(crate) fn encode(entries: &[Entry], hunk_bytes: u32, unit_bytes: u32) -> Vec<u8> {
    let per_hunk = u64::from(hunk_bytes / unit_bytes.max(1));
    let (mut last_self, mut last_parent) = (0u64, 0u64);
    let mut coded = Vec::with_capacity(entries.len());
    let mut rebuilt = Vec::with_capacity(entries.len() * 12);
    let mut first_offset = None;
    for (n, e) in entries.iter().enumerate() {
        let c = match e.kind {
            Kind::Codec(slot) => {
                first_offset.get_or_insert(e.offset);
                Coded {
                    symbol: slot,
                    length: Some(e.length),
                    crc: Some(e.crc),
                    reference: None,
                }
            }
            Kind::Plain => {
                first_offset.get_or_insert(e.offset);
                Coded {
                    symbol: TYPE_PLAIN,
                    length: None,
                    crc: Some(e.crc),
                    reference: None,
                }
            }
            Kind::SelfRef => {
                let c = if e.offset == last_self {
                    (TYPE_SELF_0, None)
                } else if e.offset == last_self + 1 {
                    (TYPE_SELF_1, None)
                } else {
                    (TYPE_SELF, Some(e.offset))
                };
                last_self = e.offset;
                Coded {
                    symbol: c.0,
                    length: None,
                    crc: None,
                    reference: c.1,
                }
            }
            Kind::Parent => {
                let c = if e.offset == n as u64 * per_hunk {
                    (TYPE_PARENT_SELF, None)
                } else if e.offset == last_parent {
                    (TYPE_PARENT_0, None)
                } else if e.offset == last_parent + per_hunk {
                    (TYPE_PARENT_1, None)
                } else {
                    (TYPE_PARENT, Some(e.offset))
                };
                last_parent = e.offset;
                Coded {
                    symbol: c.0,
                    length: None,
                    crc: None,
                    reference: c.1,
                }
            }
        };
        rebuilt.push(match e.kind {
            Kind::Codec(slot) => slot,
            Kind::Plain => TYPE_PLAIN,
            Kind::SelfRef => TYPE_SELF,
            Kind::Parent => TYPE_PARENT,
        });
        rebuilt.extend_from_slice(&e.length.to_be_bytes()[1..]);
        rebuilt.extend_from_slice(&e.offset.to_be_bytes()[2..]);
        rebuilt.extend_from_slice(&e.crc.to_be_bytes());
        coded.push(c);
    }

    let length_bits = bit_width(
        coded
            .iter()
            .filter_map(|c| c.length)
            .map(u64::from)
            .max()
            .unwrap_or(0),
    );
    let self_bits = bit_width(
        coded
            .iter()
            .filter(|c| c.symbol == TYPE_SELF)
            .filter_map(|c| c.reference)
            .max()
            .unwrap_or(0),
    );
    let parent_bits = bit_width(
        coded
            .iter()
            .filter(|c| c.symbol == TYPE_PARENT)
            .filter_map(|c| c.reference)
            .max()
            .unwrap_or(0),
    );

    // runs of the type last written as a literal become run codes (the type before the first hunk counts as codec 0)
    let mut tokens = Vec::new();
    let (mut last, mut i) = (0u8, 0usize);
    while i < coded.len() {
        let t = coded[i].symbol;
        if t != last {
            tokens.push(Token::Literal(t));
            last = t;
            i += 1;
            continue;
        }
        let mut run = 1;
        while i + run < coded.len() && coded[i + run].symbol == last && run < 274 {
            run += 1;
        }
        match run {
            1 | 2 => {
                for _ in 0..run {
                    tokens.push(Token::Literal(last));
                }
            }
            3..=18 => tokens.push(Token::Small((run - 3) as u8)),
            _ => {
                let v = run - 19;
                tokens.push(Token::Large((v >> 4) as u8, (v & 15) as u8));
            }
        }
        i += run;
    }

    let mut hist = [0u32; SYMBOLS];
    for t in &tokens {
        match *t {
            Token::Literal(s) => hist[usize::from(s)] += 1,
            Token::Small(v) => {
                hist[usize::from(TYPE_RLE_SMALL)] += 1;
                hist[usize::from(v)] += 1;
            }
            Token::Large(h, l) => {
                hist[usize::from(TYPE_RLE_LARGE)] += 1;
                hist[usize::from(h)] += 1;
                hist[usize::from(l)] += 1;
            }
        }
    }
    let lengths = lengths_from_histogram(&hist, MAX_BITS);
    let codes = codes_from_lengths(&lengths, MAX_BITS).expect("a Huffman code is complete");

    let mut out = BitWriter::new();
    write_lengths(&lengths, &mut out);
    let put = |out: &mut BitWriter, s: u8| {
        let (code, len) = codes[usize::from(s)];
        out.put(code, u32::from(len));
    };
    for t in &tokens {
        match *t {
            Token::Literal(s) => put(&mut out, s),
            Token::Small(v) => {
                put(&mut out, TYPE_RLE_SMALL);
                put(&mut out, v);
            }
            Token::Large(h, l) => {
                put(&mut out, TYPE_RLE_LARGE);
                put(&mut out, h);
                put(&mut out, l);
            }
        }
    }
    for c in &coded {
        if let Some(l) = c.length {
            out.put(l, length_bits);
        }
        if let Some(crc) = c.crc {
            out.put(u32::from(crc), 16);
        }
        if let Some(r) = c.reference {
            out.put(
                r as u32,
                if c.symbol == TYPE_SELF {
                    self_bits
                } else {
                    parent_bits
                },
            );
        }
    }
    let body = out.finish();

    let mut map = Vec::with_capacity(16 + body.len());
    map.extend_from_slice(&(body.len() as u32).to_be_bytes());
    map.extend_from_slice(&first_offset.unwrap_or(0).to_be_bytes()[2..]);
    map.extend_from_slice(&crc16(&rebuilt).to_be_bytes());
    map.extend_from_slice(&[length_bits as u8, self_bits as u8, parent_bits as u8, 0]);
    map.extend(body);
    map
}

/// The 16 code lengths as 4-bit fields: a one is sent doubled, a run of three or more equal lengths (other than one) as the escape,
/// the length and the count less three.
fn write_lengths(lengths: &[u8], out: &mut BitWriter) {
    let mut i = 0;
    while i < lengths.len() {
        let l = u32::from(lengths[i]);
        let mut run = 1;
        while i + run < lengths.len() && lengths[i + run] == lengths[i] {
            run += 1;
        }
        if l == 1 {
            out.put(1, 4);
            out.put(1, 4);
            i += 1;
        } else if run < 3 {
            out.put(l, 4);
            i += 1;
        } else {
            let take = run.min(3 + 15);
            out.put(1, 4);
            out.put(l, 4);
            out.put((take - 3) as u32, 4);
            i += take;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::compressed_map;

    fn decode(map: &[u8], hunks: u32, hunk_bytes: u32, unit: u32) -> Vec<Entry> {
        let header: [u8; 16] = map[..16].try_into().unwrap();
        compressed_map(&header, &map[16..], hunks, hunk_bytes, unit).expect("decodes")
    }

    fn same(a: &[Entry], b: &[Entry]) {
        assert_eq!(a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            assert_eq!(
                (x.kind, x.length, x.offset, x.crc),
                (y.kind, y.length, y.offset, y.crc),
                "hunk {i}"
            );
        }
    }

    #[test]
    fn a_map_of_every_kind_round_trips() {
        let hb = 4096u32;
        let mut entries = Vec::new();
        let mut at = 124u64;
        // 40 codec hunks of slot 0 (a long run), some of slot 1, plain ones, then references of every shape
        for i in 0..40u32 {
            entries.push(Entry {
                kind: Kind::Codec(0),
                length: 900 + i,
                offset: at,
                crc: i as u16 * 7,
            });
            at += u64::from(900 + i);
        }
        for i in 0..5u32 {
            entries.push(Entry {
                kind: Kind::Codec(1),
                length: 1200 + i,
                offset: at,
                crc: 99,
            });
            at += u64::from(1200 + i);
        }
        entries.push(Entry {
            kind: Kind::Plain,
            length: hb,
            offset: at,
            crc: 5,
        });
        at += u64::from(hb);
        for r in [3u64, 3, 4, 5, 20, 21, 2] {
            entries.push(Entry {
                kind: Kind::SelfRef,
                length: 0,
                offset: r,
                crc: 0,
            });
        }
        let base = entries.len() as u64;
        for off in [base, base + 1, 7, 7, 8, 12345, 100] {
            // unit size 1: a hunk is 4096 units
            entries.push(Entry {
                kind: Kind::Parent,
                length: 0,
                offset: off * 4096 * u64::from(off != base),
                crc: 0,
            });
        }
        let _ = at;
        let map = encode(&entries, hb, 1);
        let back = decode(&map, entries.len() as u32, hb, 1);
        same(&entries, &back);
    }

    #[test]
    fn long_runs_and_a_single_hunk_round_trip() {
        let hb = 2448 * 8;
        for count in [1usize, 2, 3, 18, 19, 20, 274, 275, 600] {
            let entries: Vec<Entry> = (0..count)
                .map(|i| Entry {
                    kind: Kind::Codec(0),
                    length: 777,
                    offset: 124 + 777 * i as u64,
                    crc: i as u16,
                })
                .collect();
            let back = decode(&encode(&entries, hb, 2448), count as u32, hb, 2448);
            same(&entries, &back);
        }
    }

    #[test]
    fn a_file_of_nothing_but_references_round_trips() {
        let hb = 512u32;
        let entries: Vec<Entry> = (0..30u64)
            .map(|i| Entry {
                kind: Kind::SelfRef,
                length: 0,
                offset: i / 3,
                crc: 0,
            })
            .collect();
        same(&entries, &decode(&encode(&entries, hb, 512), 30, hb, 512));
    }
}
