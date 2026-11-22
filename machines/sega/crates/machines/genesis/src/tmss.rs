//! Trademark security (TMSS): the lock on the VDP and the boot ROM that maps over the cartridge (facts T1-T5).
//!
//! * Until the four bytes `SEGA` are in the TMSS register (`$A14000-$A14003`) the VDP does not answer: an access hangs the
//!   machine (Genesis Plus GX, which models the lock this way; the real bus never gets its acknowledge).
//! * A boot ROM of 2 KB, when there is one, shows at the start of the address space until bit 0 of `$A14101` is set; then the
//!   cartridge does. The register reads back that bit.
//! * Without a boot ROM the registers the boot ROM would have set are set when the machine starts (Genesis Plus GX).

use crate::config::Tmss as TmssConfig;
use serde::{Deserialize, Serialize};

/// The size of the boot ROM; it repeats through the first 64 KB.
const BOOT_ROM_BYTES: usize = 2048;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tmss {
    enabled: bool,
    #[serde(skip)]
    boot_rom: Vec<u8>,
    register: [u8; 4],
    /// Bit 0 of `$A14101`: the cartridge shows at the start of the address space (else the boot ROM, when there is one).
    cartridge: bool,
}

impl Tmss {
    pub fn new(config: Option<&TmssConfig>) -> Self {
        let boot_rom = config
            .and_then(|c| c.boot_rom.clone())
            .filter(|rom| rom.len() == BOOT_ROM_BYTES)
            .unwrap_or_default();
        Self {
            enabled: config.is_some(),
            cartridge: boot_rom.is_empty(),
            boot_rom,
            register: [0; 4],
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The VDP is shut out: TMSS is there and nobody wrote `SEGA`.
    pub fn vdp_locked(&self) -> bool {
        self.enabled && &self.register != b"SEGA"
    }

    pub fn write_register(&mut self, index: u32, value: u8) {
        self.register[(index & 3) as usize] = value;
    }

    pub fn map_cartridge(&mut self, cartridge: bool) {
        if !self.boot_rom.is_empty() {
            self.cartridge = cartridge;
        }
    }

    pub fn cartridge_mapped(&self) -> bool {
        self.cartridge
    }

    /// A byte of the boot ROM when it is the one showing at `addr`.
    pub fn boot_rom_byte(&self, addr: u32) -> Option<u8> {
        if self.cartridge || self.boot_rom.is_empty() || addr >= 0x1_0000 {
            return None;
        }
        Some(self.boot_rom[addr as usize % BOOT_ROM_BYTES])
    }

    /// Gives a TMSS state loaded from a saved state its boot ROM back.
    pub fn adopt(&mut self, mut state: Tmss) {
        std::mem::swap(&mut state.boot_rom, &mut self.boot_rom);
        // a state cannot show a boot ROM the machine does not have
        if state.boot_rom.is_empty() {
            state.cartridge = true;
        }
        *self = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_rom() -> Tmss {
        Tmss::new(Some(&TmssConfig {
            boot_rom: Some((0..2048).map(|i| (i % 251) as u8).collect()),
        }))
    }

    #[test]
    fn without_tmss_nothing_is_locked() {
        let t = Tmss::new(None);
        assert!(!t.vdp_locked());
        assert!(t.boot_rom_byte(0).is_none());
    }

    #[test]
    fn the_vdp_is_locked_until_sega_is_written() {
        let mut t = Tmss::new(Some(&TmssConfig { boot_rom: None }));
        assert!(t.vdp_locked());
        for (i, b) in b"SEG".iter().enumerate() {
            t.write_register(i as u32, *b);
        }
        assert!(t.vdp_locked(), "three of the four letters");
        t.write_register(3, b'A');
        assert!(!t.vdp_locked());
        t.write_register(3, b'X');
        assert!(t.vdp_locked(), "writing something else locks it again");
    }

    #[test]
    fn the_boot_rom_shows_until_the_cartridge_is_mapped() {
        let mut t = with_rom();
        assert!(!t.cartridge_mapped());
        assert_eq!(t.boot_rom_byte(5), Some(5));
        assert_eq!(t.boot_rom_byte(2048 + 5), Some(5), "it repeats");
        assert_eq!(t.boot_rom_byte(0x1_0000), None);
        t.map_cartridge(true);
        assert!(t.cartridge_mapped());
        assert_eq!(t.boot_rom_byte(5), None);
        t.map_cartridge(false);
        assert_eq!(t.boot_rom_byte(5), Some(5));
    }

    #[test]
    fn a_boot_rom_of_the_wrong_size_is_ignored() {
        let t = Tmss::new(Some(&TmssConfig {
            boot_rom: Some(vec![0; 100]),
        }));
        assert!(t.enabled());
        assert!(t.boot_rom_byte(0).is_none());
        assert!(t.cartridge_mapped());
    }

    #[test]
    fn a_state_cannot_bring_a_boot_rom_the_machine_lacks() {
        let mut none = Tmss::new(Some(&TmssConfig { boot_rom: None }));
        let mut other = with_rom();
        other.map_cartridge(false);
        none.adopt(other);
        assert!(none.cartridge_mapped());
        assert!(none.boot_rom_byte(0).is_none());
    }
}
