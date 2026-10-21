//! The ST memory map as the 68000 sees it.
//!
//! | Range | Contents | Source |
//! |---|---|---|
//! | `$000000-$000007` | the first 8 bytes of the ROM (reset SSP and PC) | TOS documentation 3.5 "The system vectors": "a mirror image of the first eight bytes of the ROM" (https://freemint.github.io/tos.hyp/en/the_system_vectors.html) |
//! | `$000008-` RAM top | RAM | |
//! | `$FA0000-$FBFFFF` | cartridge (128 KB) | Atari Compendium chapter 5 "Cartridges" (http://bus-error.nokturnal.pl/atari_compendium/html/chap5.htm) |
//! | `$FC0000-$FEFFFF` | 192 KB TOS ROM | ST Internals memory map "Start of ROM OS (for versions below 1.4)"; EmuTOS 1.4 192 KB image header (reset PC `$FC0030`) |
//! | `$FF8000-$FFFFFF` | I/O | ST Internals register list (https://atarimuseum.ctrl-alt-rees.com/computers/16bits/STINTERN.htm) |
//!
//! Bus errors (TOS documentation 3.5, bus error vector): in user mode an access to the hardware registers or below
//! `$800`; in supervisor mode an access to nonexistent memory.
//!
//! Unspecified by the sources read so far, chosen deterministically (spec section 9) and to be checked against the
//! Atari hardware documentation: RAM between the configured top and `$3FFFFF` reads `$FF` per byte and ignores
//! writes, without a bus error; writes to `$0-$7` and to the ROM are ignored; an empty cartridge port reads `$FF`;
//! `$400000-$F9FFFF` and `$FF0000-$FF7FFF` are nonexistent (bus error).

use crate::config::Config;
use crate::io::{self, Decode, Device};
use crate::iolog::IoAccess;
use mc68000::{BusFault, FunctionCode, IackResponse, M68kBus, Size};

pub const ROM_BASE: u32 = 0xFC_0000;
/// TOS 1.0x and the EmuTOS 192 KB images.
pub const ROM_SIZE: usize = 196_608;
const RAM_DECODE_END: u32 = 0x40_0000;
const CARTRIDGE: std::ops::Range<u32> = 0xFA_0000..0xFC_0000;
const IO_BASE: u32 = 0xFF_8000;
const SUPERVISOR_ONLY_BELOW: u32 = 0x800;

/// Why the machine stopped at an I/O access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The device is known but not emulated yet.
    NotEmulated(Device),
    /// No hardware fact covers the access (see `io.rs`).
    NoFact,
}

/// The first I/O access the machine cannot answer: the machine stops there (spec section 16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoStop {
    pub reason: StopReason,
    pub addr: u32,
    pub size: Size,
    pub write: bool,
    pub value: u32,
    pub fc: FunctionCode,
    pub at_cycle: u64,
}

#[derive(Debug)]
pub struct StBus {
    ram: Vec<u8>,
    rom: Vec<u8>,
    stop: Option<IoStop>,
    /// Emulated time, kept up to date by the machine, for the stop record.
    pub(crate) now: u64,
    /// The interrupt the machine is asking for (VBL autovector), answered by the acknowledge cycle.
    pub(crate) vbl_pending: bool,
    /// The address of the instruction running, for the I/O log.
    pub(crate) pc: u32,
    io_log: Vec<IoAccess>,
    io_log_limit: u32,
}

/// Where an address falls.
#[derive(Debug)]
enum Region {
    Ram(usize),
    UnfittedRam,
    Rom(usize),
    Cartridge,
    Io,
    None,
}

/// The image must start the CPU inside itself: the reset PC (long 1 of the image, read through the `$0-$7` mirror)
/// even and within `$FC0000-$FEFFFF`. A TOS image starts with `BRA.S` (`$602E` in TOS 1.0x and EmuTOS 1.4); seen as
/// `$2E60`, the image is byte-swapped, a common result of reading the two ROM chips separately.
fn check_reset_pc(rom: &[u8]) -> Result<(), String> {
    if rom[0..2] == [0x2E, 0x60] {
        return Err("this TOS image looks byte-swapped (it starts 2E 60 instead of 60 2E): swap each pair of bytes".into());
    }
    let pc = u32::from_be_bytes([rom[4], rom[5], rom[6], rom[7]]) & 0xFF_FFFF;
    if pc & 1 != 0 {
        return Err(format!(
            "the image's reset PC ${pc:06X} is odd: not a TOS image for the ST"
        ));
    }
    if !(ROM_BASE..ROM_BASE + ROM_SIZE as u32).contains(&pc) {
        return Err(format!(
            "the image's reset PC ${pc:06X} is outside the ROM at $FC0000-$FEFFFF: not a 192 KB TOS image for the ST"
        ));
    }
    Ok(())
}

impl StBus {
    pub fn new(cfg: &Config) -> Result<Self, String> {
        if cfg.rom.len() != ROM_SIZE {
            return Err(format!(
                "the ST takes a {ROM_SIZE}-byte (192 KB) TOS image at $FC0000; this one has {} bytes{}",
                cfg.rom.len(),
                if cfg.rom.len() == 262_144 {
                    " (262144 bytes is a 256 KB image, for the STE, which maps it at $E00000)"
                } else {
                    ""
                }
            ));
        }
        check_reset_pc(&cfg.rom)?;
        Ok(Self {
            ram: vec![0; cfg.ram.bytes()],
            rom: cfg.rom.clone(),
            stop: None,
            now: 0,
            vbl_pending: false,
            pc: 0,
            io_log: Vec::new(),
            io_log_limit: cfg.io_log_limit,
        })
    }

    /// The completed I/O accesses so far (at most `io_log_limit`).
    pub fn io_log(&self) -> &[IoAccess] {
        &self.io_log
    }

    fn log_io(&mut self, addr: u32, size: Size, write: bool, value: u32) {
        if (self.io_log.len() as u32) < self.io_log_limit {
            self.io_log.push(IoAccess {
                pc: self.pc,
                addr,
                size,
                write,
                value,
            });
        }
    }

    /// The first unimplemented I/O access, if there was one.
    pub fn stop(&self) -> Option<&IoStop> {
        self.stop.as_ref()
    }

    fn region(&self, addr: u32) -> Region {
        let addr = addr & 0xFF_FFFF;
        if addr < 8 {
            Region::Rom(addr as usize)
        } else if (addr as usize) < self.ram.len() {
            Region::Ram(addr as usize)
        } else if addr < RAM_DECODE_END {
            Region::UnfittedRam
        } else if CARTRIDGE.contains(&addr) {
            Region::Cartridge
        } else if (ROM_BASE..ROM_BASE + ROM_SIZE as u32).contains(&addr) {
            Region::Rom((addr - ROM_BASE) as usize)
        } else if addr >= IO_BASE {
            Region::Io
        } else {
            Region::None
        }
    }

    /// A byte as the debugger sees it: no device side effects, I/O reads `$FF`.
    pub fn peek(&self, addr: u32) -> u8 {
        match self.region(addr) {
            Region::Ram(i) => self.ram[i],
            Region::Rom(i) => self.rom[i],
            _ => 0xFF,
        }
    }

    /// Stores a byte for the debugger and program loading: RAM only.
    pub fn poke(&mut self, addr: u32, value: u8) -> bool {
        match self.region(addr) {
            Region::Ram(i) => {
                self.ram[i] = value;
                true
            }
            _ => false,
        }
    }

    /// One I/O access through the decode table. `Err` is a bus error; a stop answers `$FF` per byte.
    fn io(
        &mut self,
        addr: u32,
        size: Size,
        write: bool,
        value: u32,
        fc: FunctionCode,
    ) -> Result<u32, BusFault> {
        let ff = match size {
            Size::Byte => 0xFF,
            Size::Word => 0xFFFF,
        };
        let reason = match io::decode(addr, size) {
            Decode::BusError => return Err(BusFault),
            Decode::Open => {
                self.log_io(addr, size, write, if write { value } else { ff });
                return Ok(ff);
            }
            Decode::Device(d) => StopReason::NotEmulated(d),
            Decode::NoFact => StopReason::NoFact,
        };
        if self.stop.is_none() {
            // The stopping access is logged only when it is a write: a read's value would be invented.
            if write {
                self.log_io(addr, size, write, value);
            }
            self.stop = Some(IoStop {
                reason,
                addr,
                size,
                write,
                value,
                fc,
                at_cycle: self.now,
            });
        }
        Ok(ff)
    }

    fn byte(&self, region: &Region) -> u8 {
        match *region {
            Region::Ram(i) => self.ram[i],
            Region::Rom(i) => self.rom[i],
            _ => 0xFF,
        }
    }

    fn user_refused(addr: u32, fc: FunctionCode) -> bool {
        let addr = addr & 0xFF_FFFF;
        !fc.is_supervisor() && !(SUPERVISOR_ONLY_BELOW..IO_BASE).contains(&addr)
    }
}

impl M68kBus for StBus {
    fn read(&mut self, addr: u32, size: Size, fc: FunctionCode) -> Result<u32, BusFault> {
        if Self::user_refused(addr, fc) {
            return Err(BusFault);
        }
        let region = self.region(addr);
        Ok(match region {
            Region::None => return Err(BusFault),
            Region::Io => self.io(addr & 0xFF_FFFF, size, false, 0, fc)?,
            _ => match size {
                Size::Byte => self.byte(&region) as u32,
                Size::Word => {
                    let lo = self.region(addr.wrapping_add(1));
                    u16::from_be_bytes([self.byte(&region), self.byte(&lo)]) as u32
                }
            },
        })
    }

    fn write(
        &mut self,
        addr: u32,
        size: Size,
        value: u32,
        fc: FunctionCode,
    ) -> Result<(), BusFault> {
        if Self::user_refused(addr, fc) {
            return Err(BusFault);
        }
        match self.region(addr) {
            Region::None => return Err(BusFault),
            Region::Io => {
                self.io(addr & 0xFF_FFFF, size, true, value, fc)?;
            }
            Region::Ram(i) => match size {
                Size::Byte => self.ram[i] = value as u8,
                Size::Word => {
                    self.ram[i] = (value >> 8) as u8;
                    if let Some(b) = self.ram.get_mut(i + 1) {
                        *b = value as u8;
                    }
                }
            },
            Region::Rom(_) | Region::UnfittedRam | Region::Cartridge => {}
        }
        Ok(())
    }

    fn iack(&mut self, _level: u8) -> IackResponse {
        self.vbl_pending = false;
        IackResponse::Autovector
    }
}
