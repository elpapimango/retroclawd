//! The Huffman code the compressed map uses for hunk types: 16 symbols, codes of at most 8 bits, the code lengths sent as a short
//! run-length coded list and the codes assigned canonically (the longest codes get the lowest values).

use crate::bits::BitReader;

const SYMBOLS: usize = 16;
const MAX_BITS: u32 = 8;
/// Width of one code length in the transmitted list (a maximum code length of 8 or more).
const LENGTH_FIELD_BITS: u32 = 4;

pub(crate) struct Decoder {
    /// Indexed by the next 8 bits: the symbol and the number of bits its code takes (0: no code starts like this).
    lookup: [(u8, u8); 1 << MAX_BITS],
}

impl Decoder {
    /// Reads the code lengths and builds the decoder. `None`: the list is malformed or the lengths cannot form a prefix code.
    pub(crate) fn from_stream(bits: &mut BitReader) -> Option<Decoder> {
        let mut lengths = [0u8; SYMBOLS];
        let mut at = 0usize;
        while at < SYMBOLS {
            let value = bits.read(LENGTH_FIELD_BITS);
            if value != 1 {
                lengths[at] = value as u8;
                at += 1;
                continue;
            }
            // a one is an escape: two ones are a single one, otherwise a length and a repeat count follow
            let value = bits.read(LENGTH_FIELD_BITS);
            if value == 1 {
                lengths[at] = 1;
                at += 1;
            } else {
                let repeat = bits.read(LENGTH_FIELD_BITS) as usize + 3;
                if at + repeat > SYMBOLS {
                    return None;
                }
                lengths[at..at + repeat].fill(value as u8);
                at += repeat;
            }
        }
        if bits.overflowed() {
            return None;
        }
        Self::from_lengths(&lengths)
    }

    fn from_lengths(lengths: &[u8; SYMBOLS]) -> Option<Decoder> {
        let mut count = [0u32; 33];
        for &len in lengths {
            if u32::from(len) > MAX_BITS {
                return None;
            }
            count[usize::from(len)] += 1;
        }
        // the first code of each length, from the longest down: each length starts where the previous one's codes end, halved
        let mut start = [0u32; 33];
        let mut next = 0u32;
        for len in (1..=32usize).rev() {
            let end = next + count[len];
            let half = end >> 1;
            if len != 1 && half * 2 != end {
                return None;
            }
            start[len] = next;
            next = half;
        }
        let mut lookup = [(0u8, 0u8); 1 << MAX_BITS];
        for (symbol, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let len = u32::from(len);
            let code = start[len as usize];
            start[len as usize] += 1;
            let shift = MAX_BITS - len;
            let first = (code << shift) as usize;
            let last = ((code + 1) << shift) as usize;
            if last > lookup.len() {
                return None;
            }
            lookup[first..last].fill((symbol as u8, len as u8));
        }
        Some(Decoder { lookup })
    }

    /// The next symbol; `None` when the bits start no code.
    pub(crate) fn decode(&self, bits: &mut BitReader) -> Option<u8> {
        let (symbol, len) = self.lookup[bits.peek(MAX_BITS) as usize];
        if len == 0 {
            return None;
        }
        bits.remove(u32::from(len));
        Some(symbol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Packs `(value, width)` fields most significant bit first.
    fn pack(fields: &[(u32, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        let (mut acc, mut n) = (0u64, 0u32);
        for &(v, w) in fields {
            acc = (acc << w) | u64::from(v);
            n += w;
            while n >= 8 {
                out.push((acc >> (n - 8)) as u8);
                n -= 8;
            }
        }
        if n > 0 {
            out.push((acc << (8 - n)) as u8);
        }
        out
    }

    #[test]
    fn a_single_one_bit_code_decodes_as_symbol_zero() {
        // symbol 0 has a 1-bit code (sent as the doubled escape 1,1), the other fifteen have none
        let mut fields = vec![(1, 4), (1, 4)];
        fields.extend(std::iter::repeat_n((0, 4), 15));
        fields.extend([(0, 1), (0, 1), (0, 1)]); // three symbols, each the one-bit code 0
        let data = pack(&fields);
        let mut bits = BitReader::new(&data);
        let d = Decoder::from_stream(&mut bits).unwrap();
        assert_eq!(d.decode(&mut bits), Some(0));
        assert_eq!(d.decode(&mut bits), Some(0));
        assert_eq!(d.decode(&mut bits), Some(0));
    }

    #[test]
    fn canonical_codes_give_the_longest_codes_the_lowest_values() {
        // symbols 0 and 1 have 1-bit codes... no: lengths 2,2,2,3,3 for symbols 0-4, run-length coded raw
        let lengths = [2u32, 2, 2, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        // 3-bit codes come first (000, 001), then the three 2-bit codes (01, 10, 11)
        let fields: Vec<(u32, u32)> = lengths.iter().map(|&l| (l, 4)).collect();
        let data = pack(&fields);
        let mut bits = BitReader::new(&data);
        let d = Decoder::from_stream(&mut bits).unwrap();
        let stream = pack(&[(0b000, 3), (0b001, 3), (0b01, 2), (0b10, 2), (0b11, 2)]);
        let mut s = BitReader::new(&stream);
        let got: Vec<u8> = (0..5).map(|_| d.decode(&mut s).unwrap()).collect();
        assert_eq!(got, [3, 4, 0, 1, 2]);
    }

    #[test]
    fn lengths_that_cannot_form_a_prefix_code_are_refused() {
        // three codes of length 1 (over-subscribed) and a code longer than 8 bits
        let fields: Vec<(u32, u32)> = [1u32, 1, 1, 1].iter().map(|&l| (l, 4)).collect();
        let mut two_ones = vec![(1, 4), (1, 4), (1, 4), (1, 4), (1, 4), (1, 4)];
        two_ones.extend(std::iter::repeat_n((0, 4), 13));
        let data = pack(&two_ones);
        assert!(Decoder::from_stream(&mut BitReader::new(&data)).is_none());
        let _ = fields;
        let long: Vec<(u32, u32)> = std::iter::repeat_n((9, 4), 16).collect();
        assert!(Decoder::from_stream(&mut BitReader::new(&pack(&long))).is_none());
        // a list cut short
        assert!(Decoder::from_stream(&mut BitReader::new(&[0x22])).is_none());
    }
}
