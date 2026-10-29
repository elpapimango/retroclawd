//! The Ricoh RF5C164 PCM sound source as the Sega CD's sub-CPU sees it (facts P1-P4 in `docs/hardware-facts.md`): eight channels
//! with envelope, pan, address step, loop and start registers, a control register that selects a channel or a 4 KB bank of the
//! 64 KB wave RAM, and the on/off register.
//!
//! Slice 3 keeps the registers and the wave RAM only: nothing is played yet (slice 5), and the read-only address counters
//! are not answered.
#![forbid(unsafe_code)]

pub const WAVE_RAM_SIZE: usize = 0x1_0000;
/// The window of the wave RAM in the chip's address space is 4 KB.
pub const BANK_SIZE: usize = 0x1000;
pub const CHANNELS: usize = 8;

#[derive(Clone, Debug, Default)]
struct Channel {
    env: u8,
    pan: u8,
    fd: u16,
    loop_addr: u16,
    start: u8,
}

#[derive(Clone, Debug)]
pub struct Pcm {
    channels: [Channel; CHANNELS],
    wave: Box<[u8]>,
    /// Control register: bit 7 sounding, bit 6 MOD, bits 3-0 wave bank (MOD = 0) or bits 2-0 channel select (MOD = 1).
    on: bool,
    mod_channel: bool,
    bank: u8,
    selected: u8,
    /// The on/off register: a set bit suspends that channel.
    off_mask: u8,
}

impl Default for Pcm {
    fn default() -> Self {
        Self::new()
    }
}

impl Pcm {
    pub fn new() -> Self {
        Self {
            channels: Default::default(),
            wave: vec![0; WAVE_RAM_SIZE].into_boxed_slice(),
            on: false,
            mod_channel: false,
            bank: 0,
            selected: 0,
            off_mask: 0xFF,
        }
    }

    pub fn sounding(&self) -> bool {
        self.on
    }

    pub fn bank(&self) -> u8 {
        self.bank
    }

    pub fn selected_channel(&self) -> u8 {
        self.selected
    }

    pub fn suspended_mask(&self) -> u8 {
        self.off_mask
    }

    pub fn wave_ram(&self) -> &[u8] {
        &self.wave
    }

    /// A write to the chip's 13-bit address space (`$0000-$1FFF`). `false`: an address the manual calls not usable.
    pub fn write(&mut self, chip_addr: u16, v: u8) -> bool {
        match chip_addr {
            0x0..=0x6 => {
                let ch = &mut self.channels[usize::from(self.selected)];
                match chip_addr {
                    0 => ch.env = v,
                    1 => ch.pan = v,
                    2 => ch.fd = (ch.fd & 0xFF00) | u16::from(v),
                    3 => ch.fd = (ch.fd & 0x00FF) | (u16::from(v) << 8),
                    4 => ch.loop_addr = (ch.loop_addr & 0xFF00) | u16::from(v),
                    5 => ch.loop_addr = (ch.loop_addr & 0x00FF) | (u16::from(v) << 8),
                    _ => ch.start = v,
                }
                true
            }
            0x7 => {
                self.on = v & 0x80 != 0;
                self.mod_channel = v & 0x40 != 0;
                if self.mod_channel {
                    self.selected = v & 7;
                } else {
                    self.bank = v & 0xF;
                }
                true
            }
            0x8 => {
                self.off_mask = v;
                true
            }
            0x1000..=0x1FFF => {
                // wave RAM, in the bank the control register chose; while the chip is sounding the manual restricts writes
                // (facts P3); the bytes are stored either way
                let at = usize::from(self.bank) * BANK_SIZE + usize::from(chip_addr - 0x1000);
                self.wave[at] = v;
                true
            }
            _ => false,
        }
    }

    /// A read of the wave RAM window. The address counters (`$10-$1F`) are not answered yet (`None`).
    pub fn read(&self, chip_addr: u16) -> Option<u8> {
        match chip_addr {
            0x1000..=0x1FFF => {
                let at = usize::from(self.bank) * BANK_SIZE + usize::from(chip_addr - 0x1000);
                Some(self.wave[at])
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_control_register_selects_a_channel_or_a_bank_by_its_mod_bit() {
        let mut p = Pcm::new();
        assert!(p.write(7, 0x80 | 0x05)); // MOD = 0: bank 5, sounding
        assert_eq!((p.sounding(), p.bank(), p.selected_channel()), (true, 5, 0));
        assert!(p.write(7, 0x40 | 0x03)); // MOD = 1: channel 4, sound off
        assert_eq!(
            (p.sounding(), p.bank(), p.selected_channel()),
            (false, 5, 3)
        );
    }

    #[test]
    fn channel_registers_go_to_the_selected_channel_and_the_on_off_register_to_all() {
        let mut p = Pcm::new();
        p.write(7, 0x40 | 2);
        for (a, v) in [(0, 0xFF), (1, 0x77), (2, 0x00), (3, 0x04), (6, 0x10)] {
            assert!(p.write(a, v));
        }
        p.write(7, 0x40);
        assert!(p.write(8, 0xFE));
        assert_eq!(p.suspended_mask(), 0xFE);
        assert!(!p.write(0x0F, 0), "not usable addresses are refused");
    }

    #[test]
    fn wave_ram_is_a_4k_window_onto_64k_by_bank() {
        let mut p = Pcm::new();
        p.write(7, 0x03);
        assert!(p.write(0x1000, 0xAA));
        p.write(7, 0x04);
        assert!(p.write(0x1000, 0xBB));
        assert_eq!(p.wave_ram()[3 * BANK_SIZE], 0xAA);
        assert_eq!(p.wave_ram()[4 * BANK_SIZE], 0xBB);
        assert_eq!(p.read(0x1000), Some(0xBB));
        p.write(7, 0x03);
        assert_eq!(p.read(0x1000), Some(0xAA));
        assert_eq!(
            p.read(0x10),
            None,
            "the address counters are not answered yet"
        );
    }
}
