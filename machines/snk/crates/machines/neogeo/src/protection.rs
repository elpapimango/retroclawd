//! The challenge and response of the protection chip of Fatal Fury 2 (and the cartridges with a PRO-CT0 or SNK-9201 like it): the
//! program writes a word to the chip, then reads bytes from fixed addresses and compares them with constants of its own code. What is
//! implemented is what the program was seen to ask (fact N45): the answers are the constants its comparisons subtract.

/// Where a challenge word is written, the word, and the address the answers are read from with the four bytes of the answer.
const CHALLENGES: [(u32, u16, u32, [u8; 4]); 4] = [
    (0x25_6782, 0x1234, 0x23_6001, [0xF0, 0x5A, 0x36, 0x01]),
    (0x25_6782, 0x1234, 0x23_6005, [0x0F, 0xA5, 0x63, 0x10]),
    (0x24_2812, 0x1824, 0x23_6009, [0x81, 0x42, 0x24, 0x18]),
    (0x24_2812, 0x1824, 0x23_600D, [0x18, 0x24, 0x42, 0x81]),
];

/// The first challenge: after a write of `$5555` to `$255552` these byte addresses read `$FF`, `$00`, `$FF`, `$00` (fact N45).
const FIRST_ANSWERS: [(u32, u8); 4] = [
    (0x25_5551, 0xFF),
    (0x2F_FFF1, 0x00),
    (0x20_0001, 0xFF),
    (0x2F_F001, 0x00),
];

#[derive(Default)]
pub struct Protection {
    /// The challenge words last written to `$256782` and `$242812`, and how far the answers have been read.
    written: [u16; 2],
    position: [u8; 4],
    first: bool,
}

impl Protection {
    /// A word written to a challenge address; `true` when it was one (fact N45).
    pub fn write_word(&mut self, addr: u32, value: u16) -> bool {
        match (addr, value) {
            (0x25_5552, 0x5555) => {
                self.first = true;
                true
            }
            (0x25_6782, v) => {
                self.written[0] = v;
                self.position[0] = 0;
                self.position[1] = 0;
                true
            }
            (0x24_2812, v) => {
                self.written[1] = v;
                self.position[2] = 0;
                self.position[3] = 0;
                true
            }
            _ => false,
        }
    }

    /// A byte written next to the chip (the program writes back where it read): accepted and ignored while a challenge is open.
    pub fn write_byte(&mut self, addr: u32) -> bool {
        (self.first && FIRST_ANSWERS.iter().any(|(a, _)| *a == addr))
            || CHALLENGES.iter().any(|(_, _, a, _)| *a == addr)
    }

    /// A byte read from an answer address; `None` when the chip does not answer there.
    pub fn read_byte(&mut self, addr: u32) -> Option<u8> {
        if self.first
            && let Some((_, v)) = FIRST_ANSWERS.iter().find(|(a, _)| *a == addr)
        {
            return Some(*v);
        }
        for (n, (_, word, a, bytes)) in CHALLENGES.iter().enumerate() {
            if *a == addr && self.written[n / 2] == *word {
                let at = &mut self.position[n];
                let v = bytes[usize::from(*at & 3)];
                *at = at.wrapping_add(1);
                return Some(v);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_challenge_answers_ff_00_ff_00() {
        let mut p = Protection::default();
        assert_eq!(p.read_byte(0x25_5551), None, "no challenge yet");
        assert!(p.write_word(0x25_5552, 0x5555));
        let answers: Vec<u8> = [0x25_5551, 0x2F_FFF1, 0x20_0001, 0x2F_F001]
            .iter()
            .map(|a| p.read_byte(*a).unwrap())
            .collect();
        assert_eq!(answers, [0xFF, 0x00, 0xFF, 0x00]);
    }

    #[test]
    fn the_second_and_third_challenges_answer_four_bytes_each() {
        let mut p = Protection::default();
        p.write_word(0x25_6782, 0x1234);
        let a: Vec<u8> = (0..4).map(|_| p.read_byte(0x23_6001).unwrap()).collect();
        assert_eq!(a, [0xF0, 0x5A, 0x36, 0x01]);
        let b: Vec<u8> = (0..4).map(|_| p.read_byte(0x23_6005).unwrap()).collect();
        assert_eq!(b, [0x0F, 0xA5, 0x63, 0x10]);
        p.write_word(0x24_2812, 0x1824);
        let c: Vec<u8> = (0..4).map(|_| p.read_byte(0x23_6009).unwrap()).collect();
        assert_eq!(c, [0x81, 0x42, 0x24, 0x18]);
        let d: Vec<u8> = (0..4).map(|_| p.read_byte(0x23_600D).unwrap()).collect();
        assert_eq!(d, [0x18, 0x24, 0x42, 0x81]);
        // a new challenge starts the answer again
        p.write_word(0x25_6782, 0x1234);
        assert_eq!(p.read_byte(0x23_6001), Some(0xF0));
    }
}
