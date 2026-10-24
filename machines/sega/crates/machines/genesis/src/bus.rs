//! The 68000's address space (fact rows M1-M15 in `docs/hardware-facts.md`). Every access advances the machine's master
//! clock by one 68000 bus cycle; devices are caught up to that clock when they are touched.

use crate::config::{Config, M68K_DIVIDER, Region};
use mc68000::{BusFault, FunctionCode, IackResponse, M68kBus, Size};

/// Master clocks of one 68000 bus cycle (4 CPU clocks).
pub const BUS_CYCLE: u64 = 4 * M68K_DIVIDER;

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
    rom: Vec<u8>,
    /// ROM address mask: the cartridge repeats every power-of-two size (a fact still to be probed, P02).
    rom_mask: usize,
    ram: Box<[u8; RAM_SIZE]>,
    stop: Option<Stop>,
    /// The PC of the instruction being executed, for reports.
    pub pc: u32,
}

impl Bus {
    pub fn new(config: &Config) -> Self {
        let size = config.rom.len().max(2).next_power_of_two();
        Self {
            now: 0,
            region: config.region,
            rom: config.rom.clone(),
            rom_mask: size - 1,
            ram: Box::new([0; RAM_SIZE]),
            stop: None,
            pc: 0,
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
        let i = addr as usize & self.rom_mask;
        self.rom.get(i).copied().unwrap_or(0xFF)
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

    fn read_byte(&mut self, addr: u32) -> Option<u8> {
        match addr {
            0..=0x3F_FFFF => Some(self.rom_byte(addr)),
            0xA1_0001 => Some(self.region.version_register()),
            0xE0_0000..=0xFF_FFFF => Some(self.ram[(addr & 0xFFFF) as usize]),
            _ => None,
        }
    }

    fn write_byte(&mut self, addr: u32, value: u8) -> bool {
        match addr {
            // The cartridge area ignores writes (fact M1); mappers and battery RAM arrive in Phase 4.
            0..=0x3F_FFFF => true,
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
        self.now += BUS_CYCLE;
        let value = match size {
            Size::Byte => self.read_byte(addr).map(u32::from),
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
        self.now += BUS_CYCLE;
        let ok = match size {
            Size::Byte => self.write_byte(addr, value as u8),
            Size::Word => {
                self.write_byte(addr, (value >> 8) as u8) && self.write_byte(addr | 1, value as u8)
            }
        };
        if !ok {
            self.fail(true, addr, size, value);
        }
        Ok(())
    }

    fn iack(&mut self, _level: u8) -> IackResponse {
        // The VDP and the external devices answer with an autovector.
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
    fn each_bus_cycle_costs_four_68000_clocks_of_master_time() {
        let mut b = bus(vec![0; 4]);
        rd(&mut b, 0, Size::Word);
        assert_eq!(b.now, 28);
        b.sync(3);
        assert_eq!(b.now, 28 + 21);
    }
}
