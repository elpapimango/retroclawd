//! CART format and types 1, 2, 12, 13, 14: Atari800 7.2.1 `DOC/cart.txt`,
//! revision `63a381884175e3cbfc745ff00dfb3d70d928eb50`.
//! https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/DOC/cart.txt
//! Checksum is the unsigned sum of ROM bytes (`CARTRIDGE_Checksum`):
//! https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/src/cartridge_info.c
use crate::bounded;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mapper {
    Standard8K,
    Standard16K,
    Xegs32K,
    Xegs64K,
    Xegs128K,
}

impl Mapper {
    pub fn from_car_type(id: u32) -> Result<Self, String> {
        match id {
            1 => Ok(Self::Standard8K),
            2 => Ok(Self::Standard16K),
            12 => Ok(Self::Xegs32K),
            13 => Ok(Self::Xegs64K),
            14 => Ok(Self::Xegs128K),
            _ => Err(format!(
                "unsupported CAR mapper {id}; supported: 1, 2, 12, 13, 14"
            )),
        }
    }
    pub fn car_type(self) -> u32 {
        match self {
            Self::Standard8K => 1,
            Self::Standard16K => 2,
            Self::Xegs32K => 12,
            Self::Xegs64K => 13,
            Self::Xegs128K => 14,
        }
    }
    fn size(self) -> usize {
        match self {
            Self::Standard8K => 8192,
            Self::Standard16K => 16384,
            Self::Xegs32K => 32768,
            Self::Xegs64K => 65536,
            Self::Xegs128K => 131072,
        }
    }
    fn banked(self) -> bool {
        matches!(self, Self::Xegs32K | Self::Xegs64K | Self::Xegs128K)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cartridge {
    mapper: Mapper,
    rom: Vec<u8>,
    bank: u8,
}

impl Cartridge {
    pub fn parse_car(bytes: &[u8]) -> Result<Self, String> {
        bounded(bytes.len())?;
        let header = bytes.get(..16).ok_or("truncated CAR header")?;
        if &header[..4] != b"CART" {
            return Err("CAR requires CART signature".into());
        }
        if header[12..].iter().any(|&byte| byte != 0) {
            return Err("unsupported nonzero CAR reserved bytes".into());
        }
        let mapper = Mapper::from_car_type(u32::from_be_bytes(header[4..8].try_into().unwrap()))?;
        let sum = bytes[16..]
            .iter()
            .fold(0u32, |sum, &byte| sum.wrapping_add(u32::from(byte)));
        if sum != u32::from_be_bytes(header[8..12].try_into().unwrap()) {
            return Err("CAR ROM checksum mismatch".into());
        }
        Self::parse_raw(&bytes[16..], Some(mapper))
    }

    /// Only raw 8K/16K have unambiguous defaults; banked dumps need a mapper.
    pub fn parse_raw(bytes: &[u8], mapper: Option<Mapper>) -> Result<Self, String> {
        bounded(bytes.len())?;
        let mapper = match mapper {
            Some(mapper) => mapper,
            None => match bytes.len() {
                8192 => Mapper::Standard8K,
                16384 => Mapper::Standard16K,
                _ => {
                    return Err(
                        "raw cartridge needs an explicit supported mapper (except 8K/16K)".into(),
                    );
                }
            },
        };
        if bytes.len() != mapper.size() {
            return Err(format!(
                "CAR mapper {} requires {} ROM bytes",
                mapper.car_type(),
                mapper.size()
            ));
        }
        Ok(Self {
            mapper,
            rom: bytes.to_vec(),
            bank: 0,
        })
    }
    pub fn mapper(&self) -> Mapper {
        self.mapper
    }
    pub fn to_car(&self) -> Vec<u8> {
        let mut bytes = b"CART".to_vec();
        bytes.extend_from_slice(&self.mapper.car_type().to_be_bytes());
        let checksum = self
            .rom
            .iter()
            .fold(0u32, |sum, &byte| sum.wrapping_add(u32::from(byte)));
        bytes.extend_from_slice(&checksum.to_be_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&self.rom);
        bytes
    }
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }
    pub fn bank(&self) -> u8 {
        self.bank
    }
    pub fn restore_bank(&mut self, bank: u8) -> Result<(), String> {
        let count = if self.mapper.banked() {
            self.rom.len() / 8192
        } else {
            1
        };
        if usize::from(bank) >= count {
            return Err("invalid cartridge bank".into());
        }
        self.bank = bank;
        Ok(())
    }
    /// XEGS power-on bank is not guaranteed by hardware. Choose zero deterministically.
    pub fn reset(&mut self) {
        self.bank = 0;
    }
    pub fn read(&self, address: u16) -> Option<u8> {
        let address = usize::from(address);
        let offset = match self.mapper {
            Mapper::Standard8K if (0xa000..=0xbfff).contains(&address) => address - 0xa000,
            Mapper::Standard16K if (0x8000..=0xbfff).contains(&address) => address - 0x8000,
            mapper if mapper.banked() && (0x8000..=0xbfff).contains(&address) => {
                let bank = if address < 0xa000 {
                    usize::from(self.bank)
                } else {
                    self.rom.len() / 8192 - 1
                };
                bank * 8192 + (address & 0x1fff)
            }
            _ => return None,
        };
        Some(self.rom[offset])
    }
    /// Only writes on D5 select XEGS banks; reads and bit 7 never disable these types.
    pub fn write_control(&mut self, address: u16, value: u8) {
        if self.mapper.banked() && (0xd500..=0xd5ff).contains(&address) {
            self.bank = value & (self.rom.len() / 8192 - 1) as u8;
        }
    }
}
