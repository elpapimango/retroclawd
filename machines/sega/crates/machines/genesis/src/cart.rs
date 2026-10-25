//! The cartridge: the ROM, the bank mapper of the big cartridges and battery RAM. Facts H1-H4 in `docs/hardware-facts.md`.
//!
//! * The ROM repeats every power-of-two size in the 4 MB cartridge area.
//! * A ROM over 4 MB (or a header that says `SEGA SSF`) is paged by the Sega mapper: bank registers at `$A130F3-$A130FF`
//!   choose which 512 KB page of the ROM each 512 KB bank of `$080000-$3FFFFF` shows; bank 0 always shows page 0
//!   (Plutiedev "Going beyond 4MB").
//! * Battery RAM is declared in the header (`RA`, a type byte, `$20`, start and end): the type says whether the data
//!   is on the even bytes, the odd bytes or both. Register `$A130F1` bit 0 maps it over the ROM, bit 1 write-protects it;
//!   when the ROM ends below the RAM's start the RAM is there whatever the register says (Plutiedev "Saving progress with SRAM").

use crate::eeprom::{self, Eeprom};
use md_cart::SramInfo;
use serde::{Deserialize, Serialize};

/// The largest battery RAM kept (the header can claim a range of up to 2 MB).
const MAX_SRAM_BYTES: usize = 256 * 1024;
const MAPPER_PAGE: usize = 0x8_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lane {
    /// Both bytes of every word.
    Word,
    /// The odd bytes (the data lines D7-D0).
    Odd,
    /// The even bytes (D15-D8).
    Even,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sram {
    start: u32,
    end: u32,
    lane: Lane,
    pub data: Vec<u8>,
    /// Register `$A130F1` bit 0.
    mapped: bool,
    /// Register `$A130F1` bit 1.
    write_protected: bool,
    /// The header says the memory is battery backed (type bit 6).
    pub battery: bool,
    /// The guest wrote to it since it was loaded or last saved.
    pub dirty: bool,
}

impl Sram {
    fn new(info: SramInfo) -> Self {
        let lane = match info.kind {
            0xF8 | 0xB8 => Lane::Odd,
            0xF0 | 0xB0 => Lane::Even,
            _ => Lane::Word,
        };
        // Odd-lane memory starts at an odd address and uses every other byte of the range.
        let span = (info.end.saturating_sub(info.start) as usize).saturating_add(1);
        let bytes = match lane {
            Lane::Word => span,
            _ => span.div_ceil(2),
        }
        .clamp(1, MAX_SRAM_BYTES);
        Self {
            start: info.start & 0x00FF_FFFF,
            end: info.end & 0x00FF_FFFF,
            lane,
            data: vec![0xFF; bytes],
            mapped: false,
            write_protected: false,
            battery: info.saves(),
            dirty: false,
        }
    }

    fn covers(&self, addr: u32) -> bool {
        addr >= self.start && addr <= self.end.max(self.start)
    }

    /// The index of the data byte at `addr`, or `None` for an address the memory does not answer on (the other lane).
    fn index(&self, addr: u32) -> Option<usize> {
        let offset = (addr - self.start) as usize;
        let i = match self.lane {
            Lane::Word => offset,
            Lane::Odd => {
                if addr & 1 == 0 {
                    return None;
                }
                (addr as usize - (self.start as usize | 1)) / 2
            }
            Lane::Even => {
                if addr & 1 != 0 {
                    return None;
                }
                (addr as usize - (self.start as usize & !1)) / 2
            }
        };
        (i < self.data.len()).then_some(i)
    }
}

/// Whether the header names Sonic the Hedgehog 3 (the name is padded with spaces to the width of two lines).
fn is_sonic_3(header: &md_cart::Header) -> bool {
    header
        .overseas_name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .contains("SONIC THE HEDGEHOG 3")
}

/// A cartridge locked onto the connector of a Sonic & Knuckles cartridge: its ROM answers from `$200000`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct LockOn {
    #[serde(skip)]
    rom: Vec<u8>,
    #[serde(skip)]
    mask: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cart {
    #[serde(skip)]
    rom: Vec<u8>,
    #[serde(skip)]
    rom_mask: usize,
    /// The smallest size the ROM repeats at: 4 MB for Sonic & Knuckles, whose upper 2 MB is the lock-on socket, not a mirror.
    mirror_floor: usize,
    /// The page each 512 KB bank shows, when the mapper is in use.
    banks: Option<[u8; 8]>,
    pub sram: Option<Sram>,
    lock: Option<LockOn>,
    /// The battery RAM is not mapped at power on although the ROM leaves room for it (Sonic 3 shows its ROM mirror there).
    sram_off_at_power_on: bool,
    /// A serial EEPROM board (the games that save to one have no battery RAM).
    pub eeprom: Option<Eeprom>,
}

impl Cart {
    pub fn new(rom: Vec<u8>, header: &md_cart::Header) -> Self {
        let mirror_floor = if header.overseas_name.contains("SONIC & KNUCKLES") {
            0x40_0000
        } else {
            2
        };
        let size = rom.len().max(mirror_floor).next_power_of_two();
        let mapped = rom.len() > 0x40_0000 || header.console.starts_with("SEGA SSF");
        let eeprom = eeprom::lookup(&header.serial).map(|(b, c)| Eeprom::new(b, c));
        Self {
            rom_mask: size - 1,
            mirror_floor,
            banks: mapped.then_some([0, 1, 2, 3, 4, 5, 6, 7]),
            sram: header.sram.filter(|_| eeprom.is_none()).map(Sram::new),
            eeprom,
            lock: None,
            sram_off_at_power_on: is_sonic_3(header),
            rom,
        }
    }

    /// Locks another cartridge onto this one: its ROM shows from `$200000` and its header's battery RAM replaces ours (fact H11).
    pub fn lock_on(&mut self, rom: Vec<u8>, header: &md_cart::Header) {
        let mask = rom.len().max(2).next_power_of_two() - 1;
        self.sram = header.sram.map(Sram::new);
        self.sram_off_at_power_on = is_sonic_3(header);
        self.lock = Some(LockOn { rom, mask });
    }

    /// A cartridge from a bare ROM with no header facts (tests, headerless programs).
    pub fn from_rom(rom: Vec<u8>) -> Self {
        let size = rom.len().max(2).next_power_of_two();
        Self {
            rom_mask: size - 1,
            mirror_floor: 2,
            banks: None,
            sram: None,
            eeprom: None,
            lock: None,
            sram_off_at_power_on: false,
            rom,
        }
    }

    /// Gives a cartridge loaded from a saved state its ROM back.
    pub fn restore_rom(&mut self, rom: Vec<u8>) {
        let size = rom.len().max(self.mirror_floor.max(2)).next_power_of_two();
        self.rom_mask = size - 1;
        self.rom = rom;
    }

    /// Puts a cartridge state that came from a saved state in place of this one, keeping this cartridge's ROM.
    pub fn adopt(&mut self, mut state: Cart) {
        std::mem::swap(&mut state.rom, &mut self.rom);
        state.rom_mask = self.rom_mask;
        if let (Some(new), Some(old)) = (&mut state.lock, &mut self.lock) {
            std::mem::swap(&mut new.rom, &mut old.rom);
            new.mask = old.mask;
        }
        *self = state;
    }

    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    fn rom_at(&self, addr: u32) -> u8 {
        if let Some(l) = &self.lock
            && addr >= 0x20_0000
        {
            return l.rom.get(addr as usize & l.mask).copied().unwrap_or(0xFF);
        }
        let a = match self.banks {
            Some(banks) => {
                let page = usize::from(banks[(addr as usize >> 19) & 7]);
                page * MAPPER_PAGE + (addr as usize & (MAPPER_PAGE - 1))
            }
            None => addr as usize,
        };
        // The chip repeats every power-of-two size; a page the ROM does not have reads as an open bus.
        self.rom.get(a & self.rom_mask).copied().unwrap_or(0xFF)
    }

    fn sram_active(&self, s: &Sram) -> bool {
        let rom_end = match &self.lock {
            Some(l) => 0x20_0000 + l.rom.len() as u32,
            None => self.rom.len() as u32,
        };
        s.mapped || (!self.sram_off_at_power_on && rom_end <= s.start)
    }

    /// A byte of the cartridge area (`addr` below `$400000`).
    pub fn read(&self, addr: u32) -> u8 {
        if let Some(e) = &self.eeprom
            && e.visible
            && self.eeprom_covers(addr)
        {
            return e.read_byte(addr).unwrap_or(0);
        }
        if let Some(s) = &self.sram
            && s.covers(addr)
            && self.sram_active(s)
        {
            return match s.index(addr) {
                Some(i) => s.data[i],
                None => 0xFF,
            };
        }
        self.rom_at(addr)
    }

    /// A write into the cartridge area: it reaches battery RAM when that is mapped and writable, else nothing.
    pub fn write(&mut self, addr: u32, value: u8) {
        if self.eeprom_covers(addr) {
            if let Some(e) = &mut self.eeprom {
                e.write_byte(addr, value);
            }
            return;
        }
        let active = self.sram.as_ref().is_some_and(|s| self.sram_active(s));
        if let Some(s) = &mut self.sram
            && active
            && !s.write_protected
            && s.covers(addr)
            && let Some(i) = s.index(addr)
            && s.data[i] != value
        {
            s.data[i] = value;
            s.dirty = true;
        }
    }

    /// Whether the EEPROM answers at `addr`: `$200000-$3FFFFF`, or the lower half of it while an Acclaim 4 MB board has switched
    /// the chip in. Writes reach the chip's board whenever they fall in the area it decodes.
    fn eeprom_covers(&self, addr: u32) -> bool {
        self.eeprom.as_ref().is_some_and(|e| match e.board() {
            eeprom::Board::Acclaim32 => (0x20_0000..0x30_0000).contains(&addr),
            _ => (0x20_0000..0x40_0000).contains(&addr),
        })
    }

    /// A word written into the cartridge area that a board decodes as one write (both byte strobes at once). `false` when no
    /// such board answers there and the word goes as two bytes.
    pub fn write_word(&mut self, addr: u32, value: u16) -> bool {
        if self.eeprom_covers(addr)
            && let Some(e) = &mut self.eeprom
        {
            e.write_word(addr, value);
            return true;
        }
        false
    }

    /// The reset line: boards that remap the cartridge area go back to their power-on mapping.
    pub fn reset(&mut self) {
        if let Some(e) = &mut self.eeprom {
            e.reset();
        }
    }

    /// A write to one of the cartridge registers (`$A130F1`, the bank registers). Returns whether the battery RAM was just
    /// switched out with changes in it (a good moment to save).
    pub fn write_register(&mut self, addr: u32, value: u8) -> bool {
        match addr & 0xFF {
            0xF1 => {
                if let Some(s) = &mut self.sram {
                    let was_mapped = s.mapped;
                    s.mapped = value & 1 != 0;
                    s.write_protected = value & 2 != 0;
                    return was_mapped && !s.mapped && s.dirty;
                }
            }
            r @ (0xF3 | 0xF5 | 0xF7 | 0xF9 | 0xFB | 0xFD | 0xFF) => {
                if let Some(banks) = &mut self.banks {
                    banks[((r - 0xF1) / 2) as usize] = value & 0x3F;
                }
            }
            _ => {}
        }
        false
    }

    pub fn has_mapper(&self) -> bool {
        self.banks.is_some()
    }

    /// The battery RAM's contents and whether it is saved between sessions.
    pub fn sram_data(&self) -> Option<(&[u8], bool)> {
        if let Some(e) = &self.eeprom {
            return Some((&e.data[..], true));
        }
        self.sram.as_ref().map(|s| (&s.data[..], s.battery))
    }

    /// Loads battery RAM from a file's contents (as many bytes as fit).
    pub fn load_sram(&mut self, bytes: &[u8]) {
        if let Some(e) = &mut self.eeprom {
            let n = bytes.len().min(e.data.len());
            e.data[..n].copy_from_slice(&bytes[..n]);
            e.dirty = false;
            return;
        }
        if let Some(s) = &mut self.sram {
            let n = bytes.len().min(s.data.len());
            s.data[..n].copy_from_slice(&bytes[..n]);
            s.dirty = false;
        }
    }

    pub fn sram_dirty(&self) -> bool {
        self.eeprom.as_ref().is_some_and(|e| e.dirty) || self.sram.as_ref().is_some_and(|s| s.dirty)
    }

    pub fn mark_sram_saved(&mut self) {
        if let Some(e) = &mut self.eeprom {
            e.dirty = false;
        }
        if let Some(s) = &mut self.sram {
            s.dirty = false;
        }
    }

    /// Whether a value read back from a saved state is usable.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(banks) = &self.banks
            && banks.iter().any(|b| *b > 0x3F)
        {
            return Err("a mapper bank is out of range".to_string());
        }
        if let Some(e) = &self.eeprom {
            e.validate()?;
        }
        if let Some(s) = &self.sram
            && (s.data.is_empty() || s.data.len() > MAX_SRAM_BYTES)
        {
            return Err("the battery RAM has the wrong size".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(console: &str, sram: Option<SramInfo>) -> md_cart::Header {
        md_cart::Header {
            console: console.to_string(),
            copyright: String::new(),
            domestic_name: String::new(),
            overseas_name: String::new(),
            serial: String::new(),
            checksum: 0,
            devices: String::new(),
            rom_start: 0,
            rom_end: 0,
            ram_start: 0xFF0000,
            ram_end: 0xFFFFFF,
            sram,
            regions: md_cart::Regions::default(),
        }
    }

    fn sram_info(kind: u8, start: u32, end: u32) -> SramInfo {
        SramInfo { kind, start, end }
    }

    #[test]
    fn a_plain_rom_repeats_in_the_cartridge_area() {
        let cart = Cart::new(vec![1, 2, 3, 4], &header("SEGA MEGA DRIVE", None));
        assert_eq!(cart.read(0), 1);
        assert_eq!(cart.read(0x1_0003), 4);
        assert!(!cart.has_mapper());
    }

    #[test]
    fn a_rom_over_4mb_gets_the_mapper_and_the_bank_registers_choose_pages() {
        let mut rom = vec![0u8; 0x50_0000];
        for page in 0..10usize {
            rom[page * 0x8_0000] = page as u8 + 0x10;
        }
        let mut cart = Cart::new(rom, &header("SEGA GENESIS", None));
        assert!(cart.has_mapper());
        // power-on: the first 4 MB as they are
        assert_eq!(cart.read(0x8_0000), 0x11);
        assert_eq!(cart.read(0x38_0000), 0x17);
        // bank 6 ($300000) shows page 8, bank 7 page 9
        cart.write_register(0xFD, 8);
        cart.write_register(0xFF, 9);
        assert_eq!(cart.read(0x30_0000), 0x18);
        assert_eq!(cart.read(0x38_0000), 0x19);
        assert_eq!(cart.read(0), 0x10, "bank 0 stays on page 0");
    }

    #[test]
    fn sonic_and_knuckles_does_not_mirror_into_the_lock_on_socket() {
        let mut h = header("SEGA MEGA DRIVE", None);
        h.overseas_name = "SONIC & KNUCKLES".into();
        let sk = Cart::new(vec![0x11; 0x20_0000], &h);
        assert_eq!(sk.read(0x1F_FFFF), 0x11);
        assert_eq!(sk.read(0x20_0000), 0xFF, "nothing locked on: open bus");
        let plain = Cart::new(vec![0x11; 0x20_0000], &header("SEGA MEGA DRIVE", None));
        assert_eq!(plain.read(0x20_0000), 0x11, "any other 2 MB ROM repeats");
    }

    #[test]
    fn a_header_that_says_ssf_turns_the_mapper_on_for_a_small_rom() {
        let cart = Cart::new(vec![0; 0x10_0000], &header("SEGA SSF", None));
        assert!(cart.has_mapper());
    }

    #[test]
    fn odd_byte_battery_ram_is_there_without_a_lock_when_the_rom_ends_below_it() {
        let mut cart = Cart::new(
            vec![0xAA; 0x8_0000],
            &header(
                "SEGA MEGA DRIVE",
                Some(sram_info(0xF8, 0x20_0001, 0x20_FFFF)),
            ),
        );
        assert_eq!(
            cart.read(0x20_0001),
            0xFF,
            "unwritten battery RAM reads $FF"
        );
        cart.write(0x20_0001, 0x42);
        cart.write(0x20_0003, 0x43);
        assert_eq!(cart.read(0x20_0001), 0x42);
        assert_eq!(cart.read(0x20_0003), 0x43);
        assert_eq!(
            cart.read(0x20_0002),
            0xFF,
            "the even bytes are not connected"
        );
        assert_eq!(cart.sram_data().unwrap().0.len(), 0x8000);
        assert!(cart.sram_dirty());
    }

    #[test]
    fn a_big_rom_shows_its_own_bytes_until_the_lock_register_maps_the_ram() {
        let mut cart = Cart::new(
            vec![0x11; 0x30_0000],
            &header(
                "SEGA MEGA DRIVE",
                Some(sram_info(0xF8, 0x20_0001, 0x20_FFFF)),
            ),
        );
        assert_eq!(cart.read(0x20_0001), 0x11);
        cart.write(0x20_0001, 0x77);
        assert_eq!(
            cart.read(0x20_0001),
            0x11,
            "not mapped: the write goes nowhere"
        );
        cart.write_register(0xF1, 1);
        cart.write(0x20_0001, 0x77);
        assert_eq!(cart.read(0x20_0001), 0x77);
        let saved_now = cart.write_register(0xF1, 0);
        assert!(saved_now, "unlocking after a change is the moment to save");
        assert_eq!(cart.read(0x20_0001), 0x11, "the ROM is back");
    }

    #[test]
    fn write_protect_keeps_the_ram_as_it_is() {
        let mut cart = Cart::new(
            vec![0; 0x10_0000],
            &header(
                "SEGA MEGA DRIVE",
                Some(sram_info(0xE0, 0x20_0000, 0x20_0FFF)),
            ),
        );
        cart.write_register(0xF1, 3);
        cart.write(0x20_0000, 0x55);
        assert_eq!(cart.read(0x20_0000), 0xFF);
        cart.write_register(0xF1, 1);
        cart.write(0x20_0000, 0x55);
        assert_eq!(cart.read(0x20_0000), 0x55);
        cart.write(0x20_0001, 0x66);
        assert_eq!(cart.read(0x20_0001), 0x66, "16-bit RAM has both bytes");
        assert_eq!(cart.sram_data().unwrap().0.len(), 0x1000);
    }

    #[test]
    fn loading_battery_ram_fills_it_and_clears_the_changed_flag() {
        let mut cart = Cart::new(
            vec![0; 0x8_0000],
            &header(
                "SEGA MEGA DRIVE",
                Some(sram_info(0xF8, 0x20_0001, 0x20_0FFF)),
            ),
        );
        cart.write(0x20_0001, 1);
        assert!(cart.sram_dirty());
        cart.load_sram(&[9, 8, 7]);
        assert!(!cart.sram_dirty());
        assert_eq!(cart.read(0x20_0001), 9);
        assert_eq!(cart.read(0x20_0005), 7);
    }

    #[test]
    fn an_absurd_header_range_is_clamped() {
        let cart = Cart::new(
            vec![0; 0x1000],
            &header("SEGA MEGA DRIVE", Some(sram_info(0xE0, 0, 0xFF_FFFF))),
        );
        assert!(cart.sram_data().unwrap().0.len() <= MAX_SRAM_BYTES);
        assert!(cart.validate().is_ok());
    }
}
