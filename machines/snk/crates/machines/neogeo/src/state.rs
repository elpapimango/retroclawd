//! Save states: the `RCST` envelope (the shape the Atari and Sega families use) around a bincode payload of what changes
//! while the machine runs: both CPUs, the bus (RAM, video RAM, palette, backup RAM, registers, the program ROM, which some
//! boards write), the sound side and the YM2610. The BIOS, the sprite, fix and sample ROMs are not saved: a state carries a
//! CRC of the BIOS and of the cartridge's fix and sound programs and loads only into the same machine. The input is untrusted:
//! the envelope is checked completely and the payload decoded before anything is applied, so a bad state leaves the machine
//! as it was.

use crate::Bus;
use crate::system::NeoGeo;
use mc68000::Cpu;

const MAGIC: &[u8; 4] = b"RCST";
const MACHINE_ID: &[u8] = b"neogeo";
/// The payload layout: bump when a field is added, removed or reordered.
const SCHEMA: u16 = 1;
/// The most a state may hold (the largest program ROMs are under 10 MB).
pub const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;

#[derive(serde::Serialize)]
struct PayloadRef<'a> {
    cpu: &'a Cpu,
    z80: &'a z80::Cpu,
    z80_clock: u64,
    nmi_was_high: bool,
    bus: &'a Bus,
}

#[derive(serde::Deserialize)]
struct Payload {
    cpu: Cpu,
    z80: z80::Cpu,
    z80_clock: u64,
    nmi_was_high: bool,
    bus: Bus,
}

/// The machine a state belongs to: the BIOS, the cartridge's fix ROM and its sound program.
fn identity(m: &NeoGeo) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(&m.bus.bios);
    h.update(&m.graphics.s);
    h.update(&m.bus.sound.m1_cart);
    h.finalize()
}

/// Encodes the machine's state.
pub fn save(m: &NeoGeo) -> Vec<u8> {
    let payload = PayloadRef {
        cpu: &m.cpu,
        z80: &m.z80,
        z80_clock: m.z80_clock,
        nmi_was_high: m.nmi_was_high,
        bus: &m.bus,
    };
    let body = bincode::serde::encode_to_vec(&payload, bincode::config::standard())
        .expect("a state always encodes");
    let mut out = Vec::with_capacity(body.len() + 32);
    out.extend_from_slice(MAGIC);
    out.push(MACHINE_ID.len() as u8);
    out.extend_from_slice(MACHINE_ID);
    out.extend_from_slice(&SCHEMA.to_le_bytes());
    out.extend_from_slice(&identity(m).to_le_bytes());
    out.extend_from_slice(&(body.len() as u64).to_le_bytes());
    out.extend_from_slice(&crc32fast::hash(&body).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Replaces the machine's state with `bytes`; on an error nothing changed.
pub fn load(m: &mut NeoGeo, bytes: &[u8]) -> Result<(), String> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let s = bytes
            .get(at..at + n)
            .ok_or_else(|| "the state file is cut short".to_string())?;
        at += n;
        Ok(s)
    };
    if take(4)? != MAGIC {
        return Err("not a retroclawd state file".to_string());
    }
    let id_len = take(1)?[0] as usize;
    if take(id_len)? != MACHINE_ID {
        return Err("the state is from another machine".to_string());
    }
    let schema = u16::from_le_bytes(take(2)?.try_into().unwrap());
    if schema != SCHEMA {
        return Err(format!(
            "the state has layout {schema}, this build reads layout {SCHEMA}"
        ));
    }
    if u32::from_le_bytes(take(4)?.try_into().unwrap()) != identity(m) {
        return Err("the state was saved with another BIOS or cartridge".to_string());
    }
    let len = u64::from_le_bytes(take(8)?.try_into().unwrap());
    if len > MAX_STATE_BYTES as u64 {
        return Err(format!(
            "the state claims {len} bytes of payload (the limit is {MAX_STATE_BYTES})"
        ));
    }
    let crc = u32::from_le_bytes(take(4)?.try_into().unwrap());
    let body = take(len as usize)?;
    if crc32fast::hash(body) != crc {
        return Err("the state file is damaged (checksum)".to_string());
    }
    let limit = bincode::config::standard().with_limit::<{ MAX_STATE_BYTES }>();
    let (p, used): (Payload, usize) = bincode::serde::decode_from_slice(body, limit)
        .map_err(|e| format!("the state file is damaged: {e}"))?;
    if used != body.len() {
        return Err("the state file is damaged: bytes after the payload".to_string());
    }
    check(&p.bus, &m.bus)?;
    check_z80_clock(p.z80_clock, p.bus.clock)?;
    p.bus
        .sound
        .ym
        .check_state()
        .map_err(|e| format!("the state file is damaged: {e}"))?;
    let mut bus = p.bus;
    // what the state does not hold stays with this machine
    let old = &mut m.bus;
    bus.bios = std::mem::take(&mut old.bios);
    bus.board = old.board;
    bus.sound.m1_board = std::mem::take(&mut old.sound.m1_board);
    bus.sound.m1_cart = std::mem::take(&mut old.sound.m1_cart);
    bus.sound.v_a = std::mem::take(&mut old.sound.v_a);
    bus.sound.v_b = std::mem::take(&mut old.sound.v_b);
    bus.lenient = old.lenient;
    m.bus = bus;
    m.cpu = p.cpu;
    m.z80 = p.z80;
    m.z80_clock = p.z80_clock;
    m.nmi_was_high = p.nmi_was_high;
    Ok(())
}

/// The sizes the machine indexes without checks must be the ones it was built with.
fn check(new: &Bus, old: &Bus) -> Result<(), String> {
    let same = new.p.len() == old.p.len()
        && new.work.len() == old.work.len()
        && new.palette.len() == old.palette.len()
        && new.backup.len() == old.backup.len()
        && new.vram.len() == old.vram.len()
        && new.pvc_ram.len() == old.pvc_ram.len()
        && new.sound.ram.len() == old.sound.ram.len();
    if !same {
        return Err("the state file is damaged: a memory has the wrong size".to_string());
    }
    // the counters the run loop adds to: far from overflowing, and the fractions below their divisors
    let counters_ok = new.clock < 1 << 60
        && new.anim_frames < 1 << 60
        && new.timer_fraction < 4
        && new.watchdog_last.is_none_or(|w| w <= new.clock);
    if counters_ok {
        Ok(())
    } else {
        Err("the state file is damaged: a counter is out of range".to_string())
    }
}

/// The most the Z80 may run ahead of the 68000 (one long instruction, in master clocks) or lag behind it (it is brought
/// up to date at every 68000 step; a frame is far more than a real state can hold), so a state cannot make the first
/// catch-up run for long.
const Z80_LEAD: u64 = 64 * 6;
const Z80_LAG: u64 = crate::system::FRAME_CLOCKS;

fn check_z80_clock(z80_clock: u64, clock: u64) -> Result<(), String> {
    if z80_clock <= clock + Z80_LEAD && clock - z80_clock.min(clock) <= Z80_LAG {
        Ok(())
    } else {
        Err("the state file is damaged: the Z80's clock is far from the 68000's".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::Graphics;

    /// A machine with a made-up BIOS that loops writing a counter to work RAM, and nothing in the cartridge.
    pub(super) fn machine() -> NeoGeo {
        // reset vectors: SSP $10F300, PC $C00402; at $402: addq.w #1,$100000 ; bra.s *-6 (in 68000 order, then
        // byte-swapped as the sets store it)
        let mut bios = vec![0u8; crate::BIOS_SIZE];
        bios[0..8].copy_from_slice(&[0x00, 0x10, 0xF3, 0x00, 0x00, 0xC0, 0x04, 0x02]);
        let prog = [0x52, 0x79, 0x00, 0x10, 0x00, 0x00, 0x60, 0xF8];
        bios[0x402..0x402 + prog.len()].copy_from_slice(&prog);
        for w in bios.as_chunks_mut::<2>().0 {
            w.swap(0, 1);
        }
        let bus = Bus::new(
            bios,
            vec![0; 0x10_0000],
            vec![0; 0x2_0000],
            vec![0; 0x2_0000],
        );
        let graphics = Graphics {
            sfix: vec![0; 0x2_0000],
            lo: vec![0; 0x1_0000],
            s: vec![1, 2, 3],
            c: Vec::new(),
        };
        NeoGeo::new(bus, graphics)
    }

    fn fingerprint(m: &NeoGeo) -> (u32, Vec<u8>, u64) {
        (m.cpu.pc, m.bus.work[..16].to_vec(), m.bus.clock)
    }

    #[test]
    fn a_state_resumes_exactly() {
        let mut m = machine();
        for _ in 0..5000 {
            m.step();
        }
        let saved = save(&m);
        for _ in 0..3000 {
            m.step();
        }
        let after = fingerprint(&m);
        assert_ne!(after.1, vec![0; 16], "the program ran");
        load(&mut m, &saved).unwrap();
        for _ in 0..3000 {
            m.step();
        }
        assert_eq!(fingerprint(&m), after);
        assert!(!m.bus.bios.is_empty(), "the BIOS stays");
    }

    #[test]
    fn bad_states_are_refused_and_change_nothing() {
        let mut m = machine();
        for _ in 0..100 {
            m.step();
        }
        let good = save(&m);
        let before = fingerprint(&m);
        let mut cases: Vec<(Vec<u8>, &str)> = vec![
            (Vec::new(), "cut short"),
            (b"XXXX".to_vec(), "not a retroclawd"),
            (good[..good.len() - 1].to_vec(), "cut short"),
        ];
        let mut flipped = good.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0xFF;
        cases.push((flipped, "checksum"));
        let mut other = good.clone();
        other[13] ^= 1; // the identity CRC
        cases.push((other, "another BIOS or cartridge"));
        let mut huge = good.clone();
        huge[17..25].copy_from_slice(&u64::MAX.to_le_bytes());
        cases.push((huge, "limit"));
        for (bytes, why) in cases {
            let e = load(&mut m, &bytes).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
            assert_eq!(fingerprint(&m), before);
        }
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;

    /// A state whose payload decodes but holds a value the chip would index with past its table is refused.
    #[test]
    fn values_out_of_range_are_refused() {
        let mut m = super::tests::machine();
        let good = save(&m);
        load(&mut m, &good).unwrap();
        // a clock near the end of u64: re-encode a state with it
        m.bus.clock = u64::MAX - 5;
        let bad = save(&m);
        m.bus.clock = 0;
        let e = load(&mut m, &bad).unwrap_err();
        assert!(
            e.contains("wrong size") || e.contains("out of range"),
            "{e}"
        );
        assert_eq!(m.bus.clock, 0);
    }
}
