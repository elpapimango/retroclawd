//! A HuCard image as the machine sees it: 8 KB pages mapped into banks `$00-$7F`.
//!
//! Dumps come as `.pce` (or `.tg16`) files, sometimes with a 512-byte copier header, and some US dumps have the
//! bits of every byte reversed. The card's mapping depends on its size: sizes that are a power of two repeat
//! through the 1 MB space, a 384 KB card is a 256 KB and a 128 KB chip (bank bit 6 picks the chip), and the 2.5 MB Street Fighter II card has a
//! switchable 512 KB window. Facts: `machines/nec/docs/hardware-facts.md`, rows M1-M8.
#![forbid(unsafe_code)]

const PAGE: usize = 0x2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    rom: Vec<u8>,
    /// Which 512 KB block the Street Fighter II window shows (0-4).
    block: u8,
    /// The header was present and removed.
    pub had_header: bool,
    /// The bytes were bit-reversed back.
    pub was_reversed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Empty,
    TooLarge(usize),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Empty => write!(f, "the HuCard image is empty"),
            Error::TooLarge(n) => write!(
                f,
                "a HuCard image of {n} bytes is larger than the 2.5 MB the bank mapper reaches"
            ),
        }
    }
}

impl std::error::Error for Error {}

impl Card {
    /// Builds a card from a file's bytes.
    pub fn from_bytes(mut bytes: Vec<u8>) -> Result<Self, Error> {
        let had_header = bytes.len() % PAGE == 512;
        if had_header {
            bytes.drain(..512);
        }
        if bytes.is_empty() {
            return Err(Error::Empty);
        }
        if bytes.len() > 0x28_0000 {
            return Err(Error::TooLarge(bytes.len()));
        }
        // The reset vector lies in bank 0's last page, so its high byte is $E0 or more on a good dump.
        let mut was_reversed = false;
        let vector_hi = |b: &[u8]| b.get(0x1FFF).copied().unwrap_or(0xFF);
        if vector_hi(&bytes) < 0xE0 {
            let reversed: Vec<u8> = bytes.iter().map(|b| b.reverse_bits()).collect();
            if vector_hi(&reversed) >= 0xE0 {
                bytes = reversed;
                was_reversed = true;
            }
        }
        Ok(Self {
            rom: bytes,
            block: 0,
            had_header,
            was_reversed,
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rom.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rom.is_empty()
    }

    fn pages(&self) -> usize {
        self.rom.len().div_ceil(PAGE)
    }

    /// The ROM page a bank shows.
    fn page_of(&self, bank: usize) -> usize {
        let pages = self.pages();
        if self.rom.len() == 0x28_0000 {
            return if bank < 0x40 {
                bank
            } else {
                0x40 * usize::from(self.block) + (bank - 0x40)
            };
        }
        if self.rom.len() == 0x6_0000 {
            // two chips: bank bit 6 picks the 128 KB one (repeated through its half), else the 256 KB one
            return if bank & 0x40 != 0 {
                0x20 + (bank & 0x0F)
            } else {
                bank & 0x1F
            };
        }
        if pages.is_power_of_two() {
            bank & (pages - 1)
        } else {
            bank % pages
        }
    }

    /// A read of bank `bank` (`$00-$7F`) at `offset` within the 8 KB page.
    #[must_use]
    pub fn read(&self, bank: u8, offset: u16) -> u8 {
        let page = self.page_of(usize::from(bank & 0x7F));
        self.rom
            .get(page * PAGE + usize::from(offset & 0x1FFF))
            .copied()
            .unwrap_or(0xFF)
    }

    /// A write into the ROM space: only the Street Fighter II card's mapper at `$1FF0-$1FF3` answers.
    pub fn write(&mut self, bank: u8, offset: u16, _value: u8) {
        if self.rom.len() == 0x28_0000
            && bank & 0x7F == 0
            && (0x1FF0..=0x1FF3).contains(&(offset & 0x1FFF))
        {
            self.block = ((offset & 3) as u8) + 1;
        }
    }

    /// The raw ROM bytes (header removed, bit order corrected).
    #[must_use]
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    /// The mapper state for save states.
    #[must_use]
    pub fn block(&self) -> u8 {
        self.block
    }

    pub fn set_block(&mut self, block: u8) {
        self.block = block.min(4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(len: usize) -> Vec<u8> {
        let mut v: Vec<u8> = (0..len).map(|i| (i / PAGE) as u8).collect();
        v[0x1FFF] = 0xE1;
        v
    }

    #[test]
    fn a_header_is_removed() {
        let mut v = vec![0u8; 512];
        v.extend(card(0x4000));
        let c = Card::from_bytes(v).unwrap();
        assert!(c.had_header);
        assert_eq!(c.len(), 0x4000);
    }

    #[test]
    fn bit_reversed_dumps_are_put_right() {
        let good = card(0x4000);
        let swapped: Vec<u8> = good.iter().map(|b| b.reverse_bits()).collect();
        let c = Card::from_bytes(swapped).unwrap();
        assert!(c.was_reversed);
        assert_eq!(c.rom(), &good[..]);
    }

    #[test]
    fn small_cards_repeat() {
        let c = Card::from_bytes(card(0x4_0000)).unwrap();
        assert_eq!(c.read(0x01, 0), 1);
        assert_eq!(c.read(0x21, 0), 1);
        assert_eq!(c.read(0x7F, 0), 31);
    }

    #[test]
    fn a_384k_card_is_two_chips() {
        let c = Card::from_bytes(card(0x6_0000)).unwrap();
        assert_eq!(c.read(0x1F, 0), 31);
        assert_eq!(c.read(0x20, 0), 0);
        assert_eq!(c.read(0x40, 0), 32);
        assert_eq!(c.read(0x4F, 0), 47);
        assert_eq!(c.read(0x5F, 0), 47);
        assert_eq!(c.read(0x6F, 0), 47);
    }

    #[test]
    fn street_fighter_ii_switches_its_window() {
        let mut c = Card::from_bytes(card(0x28_0000)).unwrap();
        assert_eq!(c.read(0x40, 0), 0);
        c.write(0, 0x1FF2, 0);
        assert_eq!(c.read(0x40, 0), (0x40 * 3) as u8);
        assert_eq!(c.read(0x3F, 0), 0x3F);
    }
}
