//! The 68000's address space (fact rows M1-M15 in `docs/hardware-facts.md`). Every access advances the machine's master
//! clock by one 68000 bus cycle; devices are caught up to that clock when they are touched.

use crate::cart::Cart;
use crate::config::{Config, M68K_DIVIDER, Region};
use crate::io::Io;
use crate::sound::Sound;
use crate::tmss::Tmss;
use crate::z80sys::Z80Sys;
use mc68000::{BusFault, FunctionCode, IackResponse, M68kBus, Size};
use vdp::{DmaBus, Vdp};

/// Master clocks of one 68000 bus cycle (4 CPU clocks).
pub const BUS_CYCLE: u64 = 4 * M68K_DIVIDER;

/// The external bus is refreshed every 128 CPU cycles and the 68000 waits 2 cycles for it (fact M16).
pub const REFRESH_INTERVAL: u64 = 128 * M68K_DIVIDER;
pub const REFRESH_DELAY: u64 = 2 * M68K_DIVIDER;

pub const RAM_SIZE: usize = 0x1_0000;

/// An access to an address no fact covers: the machine stops with this report instead of inventing a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    pub write: bool,
    pub addr: u32,
    pub size: Size,
    pub value: u32,
    pub pc: u32,
}

impl Stop {
    pub fn report(&self) -> String {
        let width = if self.size == Size::Byte {
            "byte"
        } else {
            "word"
        };
        if self.write {
            format!(
                "stop: write {width} ${:06X} = ${:X} (pc ${:06X}): no fact covers this address",
                self.addr, self.value, self.pc
            )
        } else {
            format!(
                "stop: read {width} ${:06X} (pc ${:06X}): no fact covers this address",
                self.addr, self.pc
            )
        }
    }
}

pub struct Bus {
    /// Master clocks since power on.
    pub now: u64,
    pub region: Region,
    pub cart: Cart,
    pub tmss: Tmss,
    /// Set when the guest has just switched changed battery RAM out (a good moment to save it).
    sram_flush: bool,
    ram: Box<[u8; RAM_SIZE]>,
    pub vdp: Vdp,
    pub io: Io,
    pub z80: Z80Sys,
    pub sound: Sound,
    stop: Option<Stop>,
    /// The PC of the instruction being executed, for reports.
    pub pc: u32,
    /// The master clock at which the next bus refresh holds the 68000.
    pub refresh_at: u64,
    /// Work RAM pokes of the cheat codes, written once a frame.
    pub(crate) pokes: Vec<(u32, u16, bool)>,
    /// Master clocks the 68000 owes for the Z80's accesses to its bus, paid at the next catch-up.
    pub(crate) z80_delay: u64,
    /// The 68000's bus is held by a VDP DMA until this master clock (the Z80 cannot use it before).
    pub(crate) dma_hold_until: u64,
}

/// What a VDP DMA transfer can read: the cartridge and work RAM (fact V14). Anything else stops the machine.
struct DmaMemory<'a> {
    cart: &'a Cart,
    ram: &'a [u8; RAM_SIZE],
    unmapped: Option<u32>,
}

impl DmaBus for DmaMemory<'_> {
    fn read_word(&mut self, addr: u32) -> u16 {
        let addr = addr & 0x00FF_FFFE;
        let byte = |this: &Self, a: u32| -> u8 {
            match a {
                0..=0x3F_FFFF => this.cart.read(a),
                0xE0_0000..=0xFF_FFFF => this.ram[(a & 0xFFFF) as usize],
                _ => 0,
            }
        };
        if !matches!(addr, 0..=0x3F_FFFF | 0xE0_0000..=0xFF_FFFF) && self.unmapped.is_none() {
            self.unmapped = Some(addr);
        }
        u16::from(byte(self, addr)) << 8 | u16::from(byte(self, addr | 1))
    }
}

impl Bus {
    pub fn new(config: &Config) -> Self {
        let header = md_cart::parse_header(&config.rom).0;
        let mut cart = Cart::new(config.rom.clone(), &header);
        for c in &config.cheats {
            if let crate::cheats::Cheat::RomWord { addr, data } = *c {
                cart.patch_word(addr, data);
            }
        }
        if let Some(upper) = &config.lock_on
            && header.overseas_name.contains("SONIC & KNUCKLES")
        {
            cart.lock_on(upper.clone(), &md_cart::parse_header(upper).0);
        }
        Self {
            now: 0,
            region: config.region,
            cart,
            sram_flush: false,
            ram: Box::new([0; RAM_SIZE]),
            vdp: Vdp::new(config.region.is_pal()),
            io: Io::new(
                config.region,
                config.pads[0],
                config.pads[1],
                config.tmss.is_some(),
            ),
            tmss: Tmss::new(config.tmss.as_ref()),
            z80: Z80Sys::new(),
            sound: Sound::new(config.fm_chip),
            stop: None,
            pc: 0,
            refresh_at: REFRESH_INTERVAL,
            z80_delay: 0,
            dma_hold_until: 0,
            pokes: config
                .cheats
                .iter()
                .filter_map(|c| match *c {
                    crate::cheats::Cheat::Poke { addr, data, word } => Some((addr, data, word)),
                    _ => None,
                })
                .collect(),
        }
    }

    /// The start of an external bus cycle of the 68000: a refresh that came due holds it first.
    fn bus_cycle(&mut self) {
        if self.now >= self.refresh_at {
            self.refresh_at = self.now + REFRESH_INTERVAL;
            self.now += REFRESH_DELAY;
        }
        self.now += BUS_CYCLE;
    }

    /// Writes the cheat codes' work RAM values (once a frame).
    pub(crate) fn apply_pokes(&mut self) {
        for i in 0..self.pokes.len() {
            let (addr, data, word) = self.pokes[i];
            let at = (addr & 0xFFFF) as usize;
            if word {
                self.ram[at & !1] = (data >> 8) as u8;
                self.ram[(at & !1) + 1] = data as u8;
            } else {
                self.ram[at] = data as u8;
            }
        }
    }

    pub fn stop(&self) -> Option<&Stop> {
        self.stop.as_ref()
    }

    pub fn take_stop(&mut self) -> Option<Stop> {
        self.stop.take()
    }

    /// A byte as the debugger sees it: no side effects, `$FF` where nothing is mapped.
    pub fn peek(&self, addr: u32) -> u8 {
        let addr = addr & 0x00FF_FFFF;
        match addr {
            0..=0x3F_FFFF => self.rom_byte(addr),
            0xE0_0000..=0xFF_FFFF => self.ram[(addr & 0xFFFF) as usize],
            // the Z80's RAM as the debugger sees it, whoever has the bus
            0xA0_0000..=0xA0_3FFF => self.z80.ram[(addr & 0x1FFF) as usize],
            _ => 0xFF,
        }
    }

    /// A byte of the Z80's address space as the debugger sees it: no side effects.
    pub fn z80_peek(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x3FFF => self.z80.ram[usize::from(addr & 0x1FFF)],
            0x8000..=0xFFFF => {
                let a = u32::from(self.z80.bank) << 15 | u32::from(addr & 0x7FFF);
                if a < 0x40_0000 {
                    self.rom_byte(a)
                } else {
                    0xFF
                }
            }
            _ => 0xFF,
        }
    }

    pub fn ram(&self) -> &[u8] {
        &self.ram[..]
    }

    pub fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram[..]
    }

    fn rom_byte(&self, addr: u32) -> u8 {
        self.cart.read(addr)
    }

    /// Whether battery RAM was just switched out with changes in it; clears the flag.
    pub fn take_sram_flush(&mut self) -> bool {
        std::mem::take(&mut self.sram_flush)
    }

    fn fail(&mut self, write: bool, addr: u32, size: Size, value: u32) {
        if self.stop.is_none() {
            self.stop = Some(Stop {
                write,
                addr,
                size,
                value,
                pc: self.pc,
            });
        }
    }

    /// A word to the VDP control port; a DMA from the bus holds the 68000 until it is done.
    /// Whether a VDP access must hang the machine because the TMSS lock is shut (fact T2): records the stop.
    fn vdp_locked(&mut self, write: bool, addr: u32, size: Size, value: u32) -> bool {
        if self.tmss.vdp_locked() {
            self.fail(write, addr, size, value);
            true
        } else {
            false
        }
    }

    fn vdp_control_write(&mut self, value: u16) {
        self.catch_up();
        let mut src = DmaMemory {
            cart: &self.cart,
            ram: &self.ram,
            unmapped: None,
        };
        let until = self.vdp.write_control(value, self.now, &mut src);
        let unmapped = src.unmapped;
        if until > self.now {
            self.dma_hold_until = until;
            self.now = until;
        }
        if let Some(a) = unmapped {
            self.fail(false, a, Size::Word, 0);
        }
    }

    /// A word read from one of the VDP ports; `None` for the ports no fact covers yet.
    fn vdp_read_word(&mut self, addr: u32) -> Option<u16> {
        self.catch_up();
        self.vdp_read_word_at(addr, self.now)
    }

    /// The VDP port read at master clock `t` (the VDP has been brought up to `t` by the caller).
    fn vdp_read_word_at(&mut self, addr: u32, t: u64) -> Option<u16> {
        match addr & 0x1E {
            0x00 | 0x02 => Some(self.vdp.read_data()),
            0x04 | 0x06 => Some(self.vdp.read_status(t)),
            0x08 | 0x0A | 0x0C | 0x0E => Some(self.vdp.read_hv(t)),
            _ => None,
        }
    }

    /// The Z80 reads a VDP port: it was brought up to the Z80's clock before the instruction.
    pub(crate) fn vdp_word_for_z80(&mut self, addr: u32) -> u16 {
        let t = self.z80_access_clock();
        self.vdp.advance(t);
        self.vdp_read_word_at(addr, t).unwrap_or(0xFFFF)
    }

    pub(crate) fn vdp_byte_write_for_z80(&mut self, addr: u32, value: u8) {
        let t = self.z80_access_clock();
        self.vdp.advance(t);
        let word = u16::from(value) << 8 | u16::from(value);
        match addr & 0x1E {
            0x00 | 0x02 => {
                self.vdp.write_data(word, t);
            }
            0x04 | 0x06 => {
                let mut src = DmaMemory {
                    cart: &self.cart,
                    ram: &self.ram,
                    unmapped: None,
                };
                // A DMA the Z80 starts holds the 68000, not the Z80 (the time it takes is not modelled yet).
                self.vdp.write_control(word, t, &mut src);
            }
            _ => {}
        }
    }

    /// Brings the Z80 and the VDP up to the 68000's clock: the Z80 first, because it must see the VDP as it was.
    pub fn catch_up(&mut self) {
        self.run_z80(self.now);
        if let Some((position, enabled)) = self.io.gun(self.vdp.width(), self.vdp.active_lines()) {
            self.vdp.set_gun(Some(position), enabled);
        }
        self.now += std::mem::take(&mut self.z80_delay);
        self.vdp.advance(self.now);
    }

    pub(crate) fn rom_byte_for_z80(&self, addr: u32) -> u8 {
        self.rom_byte(addr)
    }

    fn vdp_write_word(&mut self, addr: u32, value: u16) -> bool {
        match addr & 0x1E {
            0x00 | 0x02 => {
                self.catch_up();
                let until = self.vdp.write_data(value, self.now);
                if until > self.now {
                    // the 68000 waits for a free FIFO entry, to the next 68000 clock
                    self.now = until.div_ceil(M68K_DIVIDER) * M68K_DIVIDER;
                }
                true
            }
            0x04 | 0x06 => {
                self.vdp_control_write(value);
                true
            }
            _ => false,
        }
    }

    /// A byte of the Z80's address space as the 68000 sees it at `$A00000-$A0FFFF` (fact Z1, probe P05): only while it owns
    /// the Z80's bus and the Z80 is running or just reset; otherwise nothing answers and the data bus floats high.
    fn z80_area_read(&mut self, addr: u32) -> u8 {
        self.catch_up();
        if !self.z80.ram_open_to_68k(self.now) {
            return 0xFF;
        }
        match addr & 0xFFFF {
            a @ 0..=0x3FFF => self.z80.ram[(a & 0x1FFF) as usize],
            a @ 0x4000..=0x5FFF => self.sound.ym_read(self.now, a & 3),
            _ => 0xFF,
        }
    }

    fn z80_area_write(&mut self, addr: u32, value: u8) {
        self.catch_up();
        if !self.z80.ram_open_to_68k(self.now) {
            return;
        }
        match addr & 0xFFFF {
            a @ 0..=0x3FFF => self.z80.ram[(a & 0x1FFF) as usize] = value,
            a @ 0x4000..=0x5FFF => self.sound.ym_write(self.now, a & 3, value),
            0x6000..=0x60FF => self.z80.write_bank(value),
            _ => {}
        }
    }

    fn read_byte(&mut self, addr: u32) -> Option<u8> {
        match addr {
            0..=0x3F_FFFF => Some(
                self.tmss
                    .boot_rom_byte(addr)
                    .unwrap_or_else(|| self.cart.read(addr)),
            ),
            0xA0_0000..=0xA0_FFFF => Some(self.z80_area_read(addr)),
            0xA1_4101 => Some(u8::from(self.tmss.cartridge_mapped())),
            0xA1_1000 | 0xA1_1001 | 0xA1_1101 | 0xA1_1201 => Some(0),
            0xA1_1100 => {
                self.catch_up();
                Some(self.z80.read_bus_request(self.now))
            }
            0xA1_1200 => Some(self.z80.read_reset()),
            0xA1_0000..=0xA1_001F => Some(self.io.read(((addr >> 1) & 0xF) as usize, self.now)),
            0xE0_0000..=0xFF_FFFF => Some(self.ram[(addr & 0xFFFF) as usize]),
            _ => None,
        }
    }

    fn write_byte(&mut self, addr: u32, value: u8) -> bool {
        match addr {
            0xA0_0000..=0xA0_FFFF => {
                self.z80_area_write(addr, value);
                true
            }
            // The bus request and reset registers decode the high byte (bit 0 of the even address); the memory mode
            // register, TMSS and the cartridge mapper registers take writes without effect for now (facts M8, M10-M12).
            0xA1_1100 => {
                self.catch_up();
                self.z80.write_bus_request(value & 1 != 0, self.now);
                true
            }
            0xA1_1200 => {
                self.catch_up();
                self.z80.write_reset(value & 1 != 0, self.now);
                true
            }
            0xA1_1000 | 0xA1_1001 | 0xA1_1101 | 0xA1_1201 => true,
            0xA1_3000..=0xA1_30FF => {
                if self.cart.write_register(addr, value) {
                    self.sram_flush = true;
                }
                true
            }
            0xA1_4000..=0xA1_4003 => {
                self.tmss.write_register(addr & 3, value);
                true
            }
            0xA1_4101 => {
                self.tmss.map_cartridge(value & 1 != 0);
                true
            }
            // The I/O chip decodes the odd (low) byte of each register; an even byte write does nothing (fact I2, [H?]).
            0xA1_0000..=0xA1_001F => {
                if addr & 1 != 0 {
                    self.io.write(((addr >> 1) & 0xF) as usize, value, self.now);
                }
                true
            }
            // Writes in the cartridge area reach battery RAM when it is mapped and writable (fact H3), else nothing.
            0..=0x3F_FFFF => {
                self.cart.write(addr, value);
                true
            }
            0xE0_0000..=0xFF_FFFF => {
                self.ram[(addr & 0xFFFF) as usize] = value;
                true
            }
            _ => false,
        }
    }
}

impl M68kBus for Bus {
    fn read(&mut self, addr: u32, size: Size, _fc: FunctionCode) -> Result<u32, BusFault> {
        self.bus_cycle();
        let value = match size {
            // A byte read from a VDP port reads the whole word and returns the half the address names ([O] P03).
            Size::Byte | Size::Word
                if (0xC0_0000..0xC0_0020).contains(&addr)
                    && self.vdp_locked(false, addr, size, 0) =>
            {
                Some(0)
            }
            Size::Byte if (0xC0_0000..0xC0_0010).contains(&addr) => self
                .vdp_read_word(addr & !1)
                .map(|w| u32::from(if addr & 1 == 0 { w >> 8 } else { w & 0xFF })),
            // a J-Cart's pads (and the EEPROM's data line on D7) answer in the top half of the cartridge area
            Size::Byte | Size::Word
                if self.cart.has_jcart() && (0x38_0000..0x40_0000).contains(&addr) =>
            {
                self.catch_up();
                let pads = self.io.jcart_read(self.now) | u16::from(self.cart.eeprom_sda_bit7());
                Some(match size {
                    Size::Word => u32::from(pads),
                    _ if addr & 1 != 0 => u32::from(pads & 0xFF),
                    _ => u32::from(pads >> 8),
                })
            }
            Size::Byte => self.read_byte(addr).map(u32::from),
            Size::Word if (0xC0_0000..0xC0_0020).contains(&addr) => {
                self.vdp_read_word(addr).map(u32::from)
            }
            // The Z80 area is 8 bits wide: a word read shows the even byte in both halves ([O] P05).
            Size::Word if (0xA0_0000..=0xA0_FFFF).contains(&addr) => {
                let b = u32::from(self.z80_area_read(addr));
                Some(b << 8 | b)
            }
            Size::Word => {
                let hi = self.read_byte(addr);
                let lo = self.read_byte(addr | 1);
                hi.zip(lo).map(|(h, l)| u32::from(h) << 8 | u32::from(l))
            }
        };
        match value {
            Some(v) => Ok(v),
            None => {
                self.fail(false, addr, size, 0);
                Ok(0)
            }
        }
    }

    fn write(
        &mut self,
        addr: u32,
        size: Size,
        value: u32,
        _fc: FunctionCode,
    ) -> Result<(), BusFault> {
        self.bus_cycle();
        let ok = match size {
            Size::Byte | Size::Word
                if self.cart.has_jcart() && (0x38_0000..0x40_0000).contains(&addr) =>
            {
                self.catch_up();
                self.io.jcart_write(value as u16, self.now);
                true
            }
            Size::Byte | Size::Word
                if (0xC0_0000..0xC0_0020).contains(&addr)
                    && self.vdp_locked(true, addr, size, value) =>
            {
                true
            }
            // A byte written to a VDP port is the word with the byte in both halves ([O] P03); the PSG takes bytes at odd
            // addresses ($C00011-$C00017, not emulated until Task 16).
            Size::Byte if (0xC0_0000..0xC0_0008).contains(&addr) => {
                let b = value as u8;
                self.vdp_write_word(addr & !1, u16::from(b) << 8 | u16::from(b))
            }
            Size::Byte if matches!(addr, 0xC0_0011 | 0xC0_0013 | 0xC0_0015 | 0xC0_0017) => {
                self.sound.psg_write(self.now, value as u8);
                true
            }
            Size::Byte => self.write_byte(addr, value as u8),
            // The PSG answers at the odd byte of its word addresses: a word write reaches it with its low byte.
            Size::Word if matches!(addr, 0xC0_0010 | 0xC0_0012 | 0xC0_0014 | 0xC0_0016) => {
                self.sound.psg_write(self.now, value as u8);
                true
            }
            Size::Word if (0xC0_0000..0xC0_0020).contains(&addr) => {
                self.vdp_write_word(addr, value as u16)
            }
            // A word write to the Z80 area stores its high byte at the even address ([O] P05).
            Size::Word if (0xA0_0000..=0xA0_FFFF).contains(&addr) => {
                self.z80_area_write(addr & !1, (value >> 8) as u8);
                true
            }
            Size::Word if addr < 0x40_0000 && self.cart.write_word(addr, value as u16) => true,
            Size::Word => {
                self.write_byte(addr, (value >> 8) as u8) && self.write_byte(addr | 1, value as u8)
            }
        };
        if !ok {
            self.fail(true, addr, size, value);
        }
        Ok(())
    }

    fn iack(&mut self, level: u8) -> IackResponse {
        // The VDP and the external devices answer with an autovector.
        if matches!(level, 2 | 4 | 6) {
            self.catch_up();
            self.vdp.acknowledge();
        }
        IackResponse::Autovector
    }

    fn sync(&mut self, internal_clocks: u32) {
        self.now += u64::from(internal_clocks) * M68K_DIVIDER;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus(rom: Vec<u8>) -> Bus {
        Bus::new(&Config::new(rom, Region::Usa))
    }

    fn rd(b: &mut Bus, addr: u32, size: Size) -> u32 {
        b.read(addr, size, FunctionCode::SUPERVISOR_DATA).unwrap()
    }

    fn wr(b: &mut Bus, addr: u32, size: Size, v: u32) {
        b.write(addr, size, v, FunctionCode::SUPERVISOR_DATA)
            .unwrap();
    }

    #[test]
    fn ram_is_mirrored_every_64k_up_to_the_end_of_the_space() {
        let mut b = bus(vec![0; 4]);
        wr(&mut b, 0xFF_0010, Size::Word, 0x1234);
        assert_eq!(rd(&mut b, 0xE0_0010, Size::Word), 0x1234);
        assert_eq!(rd(&mut b, 0xF3_0010, Size::Word), 0x1234);
        wr(&mut b, 0xE1_0011, Size::Byte, 0x99);
        assert_eq!(rd(&mut b, 0xFF_0010, Size::Word), 0x1299);
        assert!(b.stop().is_none());
    }

    #[test]
    fn rom_reads_are_big_endian_words_and_bytes() {
        let mut b = bus(vec![0x12, 0x34, 0x56, 0x78]);
        assert_eq!(rd(&mut b, 0, Size::Word), 0x1234);
        assert_eq!(rd(&mut b, 2, Size::Word), 0x5678);
        assert_eq!(rd(&mut b, 1, Size::Byte), 0x34);
    }

    #[test]
    fn rom_writes_are_ignored() {
        let mut b = bus(vec![0x12, 0x34]);
        wr(&mut b, 0, Size::Word, 0xFFFF);
        assert_eq!(rd(&mut b, 0, Size::Word), 0x1234);
        assert!(b.stop().is_none());
    }

    #[test]
    fn a_small_rom_repeats_in_the_cartridge_area() {
        let mut b = bus(vec![0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(rd(&mut b, 0x1_0000, Size::Word), 0xAABB);
        assert_eq!(rd(&mut b, 0x3F_FFFE, Size::Word), 0xCCDD);
    }

    #[test]
    fn unmapped_access_stops_the_machine_with_a_report() {
        let mut b = bus(vec![0; 4]);
        b.pc = 0x1234;
        assert_eq!(rd(&mut b, 0x50_0000, Size::Word), 0);
        let stop = b.stop().unwrap().clone();
        assert_eq!(stop.addr, 0x50_0000);
        assert!(!stop.write);
        assert!(stop.report().contains("read word $500000"));
        assert!(stop.report().contains("pc $001234"));
        // the first stop is kept
        wr(&mut b, 0x60_0000, Size::Byte, 1);
        assert_eq!(b.stop().unwrap().addr, 0x50_0000);
    }

    #[test]
    fn the_version_register_names_the_region() {
        for (region, want) in [
            (Region::Usa, 0xA0),
            (Region::Europe, 0xE0),
            (Region::Japan, 0x20),
        ] {
            let mut b = Bus::new(&Config::new(vec![0; 4], region));
            assert_eq!(rd(&mut b, 0xA1_0001, Size::Byte), want);
        }
    }

    #[test]
    fn the_vdp_ports_take_words_and_the_data_comes_back() {
        let mut b = bus(vec![0; 4]);
        wr(&mut b, 0xC0_0004, Size::Word, 0x8104);
        wr(&mut b, 0xC0_0004, Size::Word, 0x8F02);
        wr(&mut b, 0xC0_0004, Size::Word, 0x4000);
        wr(&mut b, 0xC0_0004, Size::Word, 0x0000);
        wr(&mut b, 0xC0_0000, Size::Word, 0xCAFE);
        wr(&mut b, 0xC0_0004, Size::Word, 0x0000);
        wr(&mut b, 0xC0_0004, Size::Word, 0x0000);
        assert_eq!(rd(&mut b, 0xC0_0000, Size::Word), 0xCAFE);
        assert!(rd(&mut b, 0xC0_0004, Size::Word) & 0x200 != 0);
        assert!(b.stop().is_none());
    }

    #[test]
    fn a_dma_from_work_ram_holds_the_68000_and_fills_vram() {
        let mut b = bus(vec![0; 4]);
        wr(&mut b, 0xFF_0000, Size::Word, 0x1111);
        wr(&mut b, 0xFF_0002, Size::Word, 0x2222);
        for v in [0x8174u16, 0x8F02, 0x9302, 0x9400, 0x9500, 0x9600, 0x977F] {
            wr(&mut b, 0xC0_0004, Size::Word, u32::from(v));
        }
        // word address of $FF0000 is $7F8000: registers 21/22/23 = $00, $80, $7F
        wr(&mut b, 0xC0_0004, Size::Word, 0x9500);
        wr(&mut b, 0xC0_0004, Size::Word, 0x9680);
        wr(&mut b, 0xC0_0004, Size::Word, 0x977F);
        let before = b.now;
        wr(&mut b, 0xC0_0004, Size::Word, 0x4000);
        wr(&mut b, 0xC0_0004, Size::Word, 0x0080);
        assert!(b.now > before + 2 * BUS_CYCLE, "the 68000 waited");
        assert_eq!(&b.vdp.vram()[0..4], &[0x11, 0x11, 0x22, 0x22]);
        assert!(b.stop().is_none());
    }

    #[test]
    fn a_dma_from_an_unmapped_source_stops_the_machine() {
        let mut b = bus(vec![0; 4]);
        for v in [0x8174u16, 0x8F02, 0x9301, 0x9400, 0x9500, 0x9600, 0x9748] {
            wr(&mut b, 0xC0_0004, Size::Word, u32::from(v));
        }
        wr(&mut b, 0xC0_0004, Size::Word, 0x4000);
        wr(&mut b, 0xC0_0004, Size::Word, 0x0080);
        assert!(b.stop().is_some());
    }

    #[test]
    fn the_interrupt_acknowledge_clears_the_vdp_request() {
        let mut b = bus(vec![0; 4]);
        wr(&mut b, 0xC0_0004, Size::Word, 0x8174);
        b.now = 224 * vdp::LINE_CLOCKS + 900;
        b.vdp.advance(b.now);
        assert_eq!(b.vdp.irq_level(), 6);
        assert_eq!(b.iack(6), IackResponse::Autovector);
        assert_eq!(b.vdp.irq_level(), 0);
    }

    #[test]
    fn the_hv_counter_ports_answer() {
        let mut b = bus(vec![0; 4]);
        b.now = 100 * vdp::LINE_CLOCKS + 2000;
        let hv = rd(&mut b, 0xC0_0008, Size::Word);
        assert_eq!(hv >> 8, 100);
        assert_eq!(rd(&mut b, 0xC0_000C, Size::Word) >> 8, 100);
        assert!(b.stop().is_none());
    }

    #[test]
    fn a_byte_written_to_the_data_port_fills_both_halves_of_the_word() {
        let mut b = bus(vec![0; 4]);
        for v in [0x8104u32, 0x8F02, 0x4000, 0x0000] {
            wr(&mut b, 0xC0_0004, Size::Word, v);
        }
        wr(&mut b, 0xC0_0000, Size::Byte, 0x12);
        wr(&mut b, 0xC0_0001, Size::Byte, 0x34);
        assert_eq!(&b.vdp.vram()[0..4], &[0x12, 0x12, 0x34, 0x34]);
    }

    #[test]
    fn a_byte_read_of_a_vdp_port_returns_the_half_the_address_names() {
        let mut b = bus(vec![0; 4]);
        let hi = rd(&mut b, 0xC0_0004, Size::Byte);
        let lo = rd(&mut b, 0xC0_0005, Size::Byte);
        assert_eq!(hi, 0x02, "the FIFO empty flag is bit 9");
        assert_eq!(lo & 0x08, 0x08, "vertical blanking: the display is off");
        b.now = 77 * vdp::LINE_CLOCKS;
        assert_eq!(rd(&mut b, 0xC0_0008, Size::Byte), 77);
    }

    #[test]
    fn each_bus_cycle_costs_four_68000_clocks_of_master_time() {
        let mut b = bus(vec![0; 4]);
        rd(&mut b, 0, Size::Word);
        assert_eq!(b.now, 28);
        b.sync(3);
        assert_eq!(b.now, 28 + 21);
    }
}
