//! A most-significant-bit-first reader over a byte slice, as the map and the Huffman trees are coded.

pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    /// The position in bits.
    pos: usize,
    overflow: bool,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            overflow: false,
        }
    }

    /// The next `n` (at most 32) bits without taking them; bits past the end read as zero.
    pub(crate) fn peek(&self, n: u32) -> u32 {
        let mut value = 0u32;
        for i in 0..n as usize {
            let at = self.pos + i;
            let bit = self.data.get(at / 8).map_or(0, |b| (b >> (7 - at % 8)) & 1);
            value = (value << 1) | u32::from(bit);
        }
        value
    }

    /// Drops `n` bits; running past the end is remembered, not a panic.
    pub(crate) fn remove(&mut self, n: u32) {
        self.pos += n as usize;
        if self.pos > self.data.len() * 8 {
            self.overflow = true;
        }
    }

    pub(crate) fn read(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.remove(n);
        v
    }

    /// Whether more bits were asked for than the data holds.
    pub(crate) fn overflowed(&self) -> bool {
        self.overflow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_come_out_most_significant_first_and_overflow_is_flagged() {
        let mut r = BitReader::new(&[0b1010_0000, 0xFF]);
        assert_eq!(r.read(3), 0b101);
        assert_eq!(r.read(5), 0);
        assert_eq!(r.read(8), 0xFF);
        assert!(!r.overflowed());
        assert_eq!(r.read(4), 0);
        assert!(r.overflowed());
    }
}
