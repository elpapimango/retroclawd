//! Save states: the `RCST` envelope (the same shape as the Atari family's) around a bincode payload of everything that
//! changes while the machine runs. The cartridge itself is not saved: a state carries its CRC and loads only into a machine
//! with the same ROM, region and FM chip. All input is untrusted: the envelope is checked completely before anything is
//! applied, so a bad state leaves the machine as it was.

use crate::bus::{Bus, RAM_SIZE};
use crate::cart::Cart;
use crate::config::Region;
use crate::io::Io;
use crate::sound::Sound;
use crate::tmss::Tmss;
use crate::z80sys::Z80Sys;
use mc68000::Cpu;
use serde::{Deserialize, Serialize};
use std::fmt;
use vdp::Vdp;

const MAGIC: &[u8; 4] = b"RCST";
const MACHINE_ID: &[u8] = b"genesis";
/// The payload layout: bump when a field is added, removed or reordered.
const SCHEMA: u16 = 2;
/// The most a state may hold (the machine's RAM and chips come to well under 2 MB).
pub const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum StateError {
    Truncated,
    BadMagic,
    OtherMachine,
    OtherSchema(u16),
    OtherRom,
    OtherRegion,
    OtherChip,
    TooLarge(u64),
    BadChecksum,
    Corrupt(String),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::Truncated => write!(f, "the state file is cut short"),
            StateError::BadMagic => write!(f, "not a retroclawd state file"),
            StateError::OtherMachine => write!(f, "the state is from another machine"),
            StateError::OtherSchema(v) => write!(
                f,
                "the state has layout {v}, this build reads layout {SCHEMA}"
            ),
            StateError::OtherRom => write!(f, "the state was saved with another cartridge"),
            StateError::OtherRegion => write!(f, "the state was saved for another region"),
            StateError::OtherChip => write!(f, "the state was saved with another FM chip"),
            StateError::TooLarge(n) => write!(
                f,
                "the state claims {n} bytes of payload (the limit is {MAX_STATE_BYTES})"
            ),
            StateError::BadChecksum => write!(f, "the state file is damaged (checksum)"),
            StateError::Corrupt(why) => write!(f, "the state file is damaged: {why}"),
        }
    }
}

impl std::error::Error for StateError {}

#[derive(Serialize, Deserialize)]
struct Payload {
    cpu: Cpu,
    now: u64,
    refresh_at: u64,
    ram: Vec<u8>,
    vdp: Vdp,
    io: Io,
    z80: Z80Sys,
    sound: Sound,
    cart: Cart,
    tmss: Tmss,
}

fn region_byte(r: Region) -> u8 {
    match r {
        Region::Japan => 0,
        Region::Usa => 1,
        Region::Europe => 2,
    }
}

fn chip_byte(c: ym2612::Chip) -> u8 {
    match c {
        ym2612::Chip::Ym2612 => 0,
        ym2612::Chip::Ym3438 => 1,
    }
}

/// Encodes the machine's state.
pub fn save(cpu: &Cpu, bus: &Bus, rom_crc: u32) -> Vec<u8> {
    let payload = Payload {
        cpu: cpu.clone(),
        now: bus.now,
        refresh_at: bus.refresh_at,
        ram: bus.ram().to_vec(),
        vdp: bus.vdp.clone(),
        io: bus.io.clone(),
        z80: bus.z80.clone(),
        sound: bus.sound.clone(),
        cart: bus.cart.clone(),
        tmss: bus.tmss.clone(),
    };
    let body = bincode::serde::encode_to_vec(&payload, bincode::config::standard())
        .expect("a state always encodes");
    let mut out = Vec::with_capacity(body.len() + 64);
    out.extend_from_slice(MAGIC);
    out.push(MACHINE_ID.len() as u8);
    out.extend_from_slice(MACHINE_ID);
    out.extend_from_slice(&SCHEMA.to_le_bytes());
    out.extend_from_slice(&rom_crc.to_le_bytes());
    out.push(region_byte(bus.region));
    out.push(chip_byte(bus.sound.ym().kind()));
    out.extend_from_slice(&(body.len() as u64).to_le_bytes());
    out.extend_from_slice(&crc32fast::hash(&body).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Checks and decodes a state; nothing is changed unless this succeeds.
pub fn load(
    bytes: &[u8],
    rom_crc: u32,
    region: Region,
    chip: ym2612::Chip,
) -> Result<Loaded, StateError> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Result<&[u8], StateError> {
        let end = at.checked_add(n).ok_or(StateError::Truncated)?;
        let slice = bytes.get(at..end).ok_or(StateError::Truncated)?;
        at = end;
        Ok(slice)
    };
    if take(4)? != MAGIC {
        return Err(StateError::BadMagic);
    }
    let id_len = usize::from(take(1)?[0]);
    if take(id_len)? != MACHINE_ID {
        return Err(StateError::OtherMachine);
    }
    let schema = u16::from_le_bytes(take(2)?.try_into().unwrap());
    if schema != SCHEMA {
        return Err(StateError::OtherSchema(schema));
    }
    let crc = u32::from_le_bytes(take(4)?.try_into().unwrap());
    if crc != rom_crc {
        return Err(StateError::OtherRom);
    }
    if take(1)?[0] != region_byte(region) {
        return Err(StateError::OtherRegion);
    }
    if take(1)?[0] != chip_byte(chip) {
        return Err(StateError::OtherChip);
    }
    let len = u64::from_le_bytes(take(8)?.try_into().unwrap());
    if len > MAX_STATE_BYTES as u64 {
        return Err(StateError::TooLarge(len));
    }
    let body_crc = u32::from_le_bytes(take(4)?.try_into().unwrap());
    let body = take(len as usize)?;
    if crc32fast::hash(body) != body_crc {
        return Err(StateError::BadChecksum);
    }
    let (payload, used): (Payload, usize) =
        bincode::serde::decode_from_slice(body, bincode::config::standard())
            .map_err(|e| StateError::Corrupt(e.to_string()))?;
    if used != body.len() {
        return Err(StateError::Corrupt("trailing bytes".to_string()));
    }
    if payload.ram.len() != RAM_SIZE {
        return Err(StateError::Corrupt(
            "work RAM has the wrong size".to_string(),
        ));
    }
    if payload.vdp.is_pal() != region.is_pal() {
        return Err(StateError::OtherRegion);
    }
    if payload.refresh_at > payload.now.saturating_add(crate::bus::REFRESH_INTERVAL) {
        return Err(StateError::Corrupt("the bus refresh is too far ahead".to_string()));
    }
    payload.vdp.validate().map_err(StateError::Corrupt)?;
    payload.z80.validate().map_err(StateError::Corrupt)?;
    payload.sound.validate().map_err(StateError::Corrupt)?;
    payload.cart.validate().map_err(StateError::Corrupt)?;
    payload
        .vdp
        .validate_time(payload.now)
        .map_err(StateError::Corrupt)?;
    payload
        .z80
        .validate_time(payload.now)
        .map_err(StateError::Corrupt)?;
    payload
        .sound
        .validate_time(payload.now)
        .map_err(StateError::Corrupt)?;
    Ok(Loaded {
        cpu: payload.cpu,
        now: payload.now,
        refresh_at: payload.refresh_at,
        ram: payload.ram,
        vdp: payload.vdp,
        io: payload.io,
        z80: payload.z80,
        sound: payload.sound,
        cart: payload.cart,
        tmss: payload.tmss,
    })
}

/// A state that passed every check, ready to be put into the machine.
pub struct Loaded {
    pub cpu: Cpu,
    pub now: u64,
    pub refresh_at: u64,
    pub ram: Vec<u8>,
    pub vdp: Vdp,
    pub io: Io,
    pub z80: Z80Sys,
    pub sound: Sound,
    pub cart: Cart,
    pub tmss: Tmss,
}
