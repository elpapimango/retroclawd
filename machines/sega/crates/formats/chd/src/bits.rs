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

/// The writing side of `BitReader`: fields go in most significant bit first and the last byte is padded with zeros.
#[derive(Default)]
pub(crate) struct BitWriter {
    out: Vec<u8>,
    /// Bits used in the last byte (0 means the last byte is full or there is none).
    used: u32,
}

impl BitWriter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Writes the low `n` (at most 32) bits of `value`.
    pub(crate) fn put(&mut self, value: u32, n: u32) {
        for i in (0..n).rev() {
            if self.used == 0 {
                self.out.push(0);
            }
            if (value >> i) & 1 != 0 {
                *self.out.last_mut().expect("pushed above") |= 0x80 >> self.used;
            }
            self.used = (self.used + 1) % 8;
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_writer_puts_the_reader_gets() {
        let mut w = BitWriter::new();
        w.put(0b101, 3);
        w.put(0, 5);
        w.put(0xBEEF, 16);
        w.put(1, 1);
        let bytes = w.finish();
        assert_eq!(bytes, [0b1010_0000, 0xBE, 0xEF, 0x80]);
        let mut r = BitReader::new(&bytes);
        assert_eq!(
            (r.read(3), r.read(5), r.read(16), r.read(1)),
            (5, 0, 0xBEEF, 1)
        );
    }

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
