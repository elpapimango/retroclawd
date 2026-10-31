//! The NEO-CMC protection chips on later cartridges: the sprite data on the board is scrambled and the chip unscrambles it as the
//! LSPC reads it. Here the whole sprite region is unscrambled once at load time, and the fix layer, which these boards do not hold in
//! a chip of its own, is taken out of the end of the unscrambled sprite data.
//!
//! Fact N49 (the one documented exception to the clean-room rule, approved by the owner): the algorithm and the tables in
//! `cmc_tables.rs` come from MAME's `prot_cmc.cpp` (BSD-3-Clause; copyright S. Smith, David Haywood, Fabio Priuli); see `CREDITS.md`.

use crate::cmc_tables::*;

/// The two chip generations (they differ in their tables only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Cmc42,
    Cmc50,
}

/// One cartridge's chip and its sprite key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cmc {
    pub chip: Chip,
    pub key: u8,
}

/// The chip of a game, found by the number at the start of its file names (the NGH number).
pub fn for_ngh(ngh: u32) -> Option<Cmc> {
    let (chip, key) = match ngh {
        70 => (Chip::Cmc42, 0xbd),  // Zupapa!
        252 => (Chip::Cmc42, 0x07), // Ganryu
        260 => (Chip::Cmc42, 0xff), // Nightmare in the Dark
        261 => (Chip::Cmc42, 0xfe), // Sengoku 3
        262 => (Chip::Cmc50, 0x1e), // The King of Fighters 2001
        _ => return None,
    };
    Some(Cmc { chip, key })
}

struct Tables {
    type0_t03: &'static [u8; 256],
    type0_t12: &'static [u8; 256],
    type1_t03: &'static [u8; 256],
    type1_t12: &'static [u8; 256],
    a8_15_1: &'static [u8; 256],
    a8_15_2: &'static [u8; 256],
    a16_23_1: &'static [u8; 256],
    a16_23_2: &'static [u8; 256],
    a0_7: &'static [u8; 256],
}

fn tables(chip: Chip) -> Tables {
    match chip {
        Chip::Cmc42 => Tables {
            type0_t03: &KOF99_TYPE0_T03,
            type0_t12: &KOF99_TYPE0_T12,
            type1_t03: &KOF99_TYPE1_T03,
            type1_t12: &KOF99_TYPE1_T12,
            a8_15_1: &KOF99_ADDRESS_8_15_XOR1,
            a8_15_2: &KOF99_ADDRESS_8_15_XOR2,
            a16_23_1: &KOF99_ADDRESS_16_23_XOR1,
            a16_23_2: &KOF99_ADDRESS_16_23_XOR2,
            a0_7: &KOF99_ADDRESS_0_7_XOR,
        },
        Chip::Cmc50 => Tables {
            type0_t03: &KOF2000_TYPE0_T03,
            type0_t12: &KOF2000_TYPE0_T12,
            type1_t03: &KOF2000_TYPE1_T03,
            type1_t12: &KOF2000_TYPE1_T12,
            a8_15_1: &KOF2000_ADDRESS_8_15_XOR1,
            a8_15_2: &KOF2000_ADDRESS_8_15_XOR2,
            a16_23_1: &KOF2000_ADDRESS_16_23_XOR1,
            a16_23_2: &KOF2000_ADDRESS_16_23_XOR2,
            a0_7: &KOF2000_ADDRESS_0_7_XOR,
        },
    }
}

/// Unscrambles the sprite region in place. The size must be a power of two and a multiple of 4 (the real boards are); anything
/// else is left alone and `false` is returned.
pub fn decrypt_sprites(rom: &mut [u8], cmc: Cmc) -> bool {
    let len = rom.len();
    if len < 4 || !len.is_power_of_two() {
        return false;
    }
    let t = tables(cmc.chip);
    let words = len / 4;
    // data xor
    let mut buf = vec![0u8; len];
    let pair = |r: &[u8],
                rpos: usize,
                i0: usize,
                i1: usize,
                hi: &[u8; 256],
                lo: &[u8; 256],
                t1: &[u8; 256],
                invert: bool|
     -> (u8, u8) {
        let (c0, c1) = (r[4 * rpos + i0], r[4 * rpos + i1]);
        let tmp = t1[(rpos & 0xff) ^ t.a0_7[(rpos >> 8) & 0xff] as usize];
        let xor0 = (hi[(rpos >> 8) & 0xff] & 0xfe) | (tmp & 1);
        let xor1 = (tmp & 0xfe) | (lo[(rpos >> 8) & 0xff] & 1);
        if invert {
            (c1 ^ xor0, c0 ^ xor1)
        } else {
            (c0 ^ xor0, c1 ^ xor1)
        }
    };
    for rpos in 0..words {
        let (a, d) = pair(
            rom,
            rpos,
            0,
            3,
            t.type0_t03,
            t.type0_t12,
            t.type1_t03,
            (rpos >> 8) & 1 != 0,
        );
        buf[4 * rpos] = a;
        buf[4 * rpos + 3] = d;
        let inv = ((rpos >> 16) ^ t.a16_23_2[(rpos >> 8) & 0xff] as usize) & 1 != 0;
        let (b, c) = pair(rom, rpos, 1, 2, t.type0_t12, t.type0_t03, t.type1_t12, inv);
        buf[4 * rpos + 1] = b;
        buf[4 * rpos + 2] = c;
    }
    // address xor
    for rpos in 0..words {
        let mut b = rpos;
        b ^= cmc.key as usize;
        b ^= (t.a8_15_1[(b >> 16) & 0xff] as usize) << 8;
        b ^= (t.a8_15_2[b & 0xff] as usize) << 8;
        b ^= (t.a16_23_1[b & 0xff] as usize) << 16;
        b ^= (t.a16_23_2[(b >> 8) & 0xff] as usize) << 16;
        b ^= t.a0_7[(b >> 8) & 0xff] as usize;
        b &= words - 1;
        rom[4 * rpos..4 * rpos + 4].copy_from_slice(&buf[4 * b..4 * b + 4]);
    }
    true
}

/// Output bit 15 takes input bit `order[0]`, output bit 14 takes `order[1]`, and so on down to bit 0.
fn bitswap16(v: u32, order: [u32; 16]) -> u32 {
    order
        .iter()
        .fold(0, |acc, &src| (acc << 1) | ((v >> src) & 1))
}

const M1_BLOCKS: [[u32; 16]; 8] = [
    [15, 14, 10, 7, 1, 2, 3, 8, 0, 12, 11, 13, 6, 9, 5, 4],
    [7, 1, 8, 11, 15, 9, 2, 3, 5, 13, 4, 14, 10, 0, 6, 12],
    [8, 6, 14, 3, 10, 7, 15, 1, 4, 0, 2, 5, 13, 11, 12, 9],
    [2, 8, 15, 9, 3, 4, 11, 7, 13, 6, 0, 10, 1, 12, 14, 5],
    [1, 13, 6, 15, 14, 3, 8, 10, 9, 4, 7, 12, 5, 2, 0, 11],
    [11, 15, 3, 4, 7, 0, 9, 2, 6, 14, 12, 1, 8, 5, 10, 13],
    [10, 5, 13, 8, 6, 15, 1, 14, 11, 9, 3, 0, 12, 7, 4, 2],
    [9, 3, 7, 0, 2, 12, 4, 11, 14, 10, 5, 8, 15, 13, 1, 6],
];

fn m1_scramble(address: usize, key: u32) -> usize {
    let block = (address >> 16) & 7;
    let mut aux = (address & 0xffff) as u32;
    aux ^= bitswap16(key, [12, 0, 2, 4, 8, 15, 7, 13, 10, 1, 3, 6, 11, 9, 14, 5]);
    // the table lists the source bit of output bit 0 first: reverse it into most significant first
    let mut order = M1_BLOCKS[block];
    order.reverse();
    aux = bitswap16(aux, order);
    aux ^= u32::from(M1_ADDRESS_0_7_XOR[((aux >> 8) & 0xff) as usize]);
    aux ^= u32::from(M1_ADDRESS_8_15_XOR[(aux & 0xff) as usize]) << 8;
    aux = bitswap16(aux, [7, 15, 14, 6, 5, 13, 12, 4, 11, 3, 10, 2, 9, 1, 8, 0]);
    ((block as u32) << 16 | aux) as usize
}

/// The Z80 program of a CMC50 board: the key is the 16-bit sum of the first 64 KiB, and the 512 KiB (the file, padded with zeros)
/// is read back through an address scramble.
pub fn decrypt_m1(m: &[u8]) -> Vec<u8> {
    let mut rom = vec![0u8; 0x80000];
    let n = m.len().min(rom.len());
    rom[..n].copy_from_slice(&m[..n]);
    let key = rom[..0x10000]
        .iter()
        .fold(0u32, |sum, &b| (sum + u32::from(b)) & 0xffff);
    (0..rom.len()).map(|i| rom[m1_scramble(i, key)]).collect()
}

/// The fix layer: it is stored in the last bytes of the unscrambled sprite data, in a different tile order.
pub fn fix_from_sprites(rom: &[u8], fix_len: usize) -> Vec<u8> {
    // whole 32-byte tiles only: the size comes from the (untrusted) set, and the tile order reaches 31 bytes past `i`
    let fix_len = fix_len & !0x1f;
    let Some(start) = rom.len().checked_sub(fix_len) else {
        return Vec::new();
    };
    let src = &rom[start..];
    (0..fix_len)
        .map(|i| src[(i & !0x1f) + ((i & 7) << 2) + ((!i & 8) >> 2) + ((i & 0x10) >> 4)])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_games_are_found_by_their_number() {
        assert_eq!(
            for_ngh(261),
            Some(Cmc {
                chip: Chip::Cmc42,
                key: 0xfe
            })
        );
        assert_eq!(for_ngh(262).map(|c| c.chip), Some(Chip::Cmc50));
        assert_eq!(for_ngh(5), None);
    }

    #[test]
    fn the_fix_layer_is_a_permutation_of_the_last_bytes() {
        let rom: Vec<u8> = (0..=255u8).cycle().take(1024).collect();
        let fix = fix_from_sprites(&rom, 64);
        let mut a: Vec<u8> = fix.clone();
        let mut b: Vec<u8> = rom[1024 - 64..].to_vec();
        a.sort();
        b.sort();
        assert_eq!(a, b);
        // output byte 0 comes from source byte 2 and byte 1 from byte 6 of its 32-byte tile
        assert_eq!(fix[0], rom[1024 - 64 + 2]);
        assert_eq!(fix[1], rom[1024 - 64 + 6]);
        assert_eq!(fix_from_sprites(&rom, 4096), Vec::<u8>::new());
    }

    #[test]
    fn the_m1_scramble_is_a_permutation_inside_each_64k_block() {
        for block in 0..8usize {
            let mut seen = vec![false; 0x10000];
            for a in 0..0x10000usize {
                let t = m1_scramble((block << 16) | a, 0x1234);
                assert_eq!(t >> 16, block);
                assert!(!seen[t & 0xffff]);
                seen[t & 0xffff] = true;
            }
        }
        assert_eq!(decrypt_m1(&[1, 2, 3]).len(), 0x80000);
    }

    #[test]
    fn a_fix_size_that_is_not_whole_tiles_is_rounded_down_and_never_panics() {
        let rom = vec![1u8; 1024];
        assert_eq!(fix_from_sprites(&rom, 33).len(), 32);
        assert_eq!(fix_from_sprites(&rom, 31).len(), 0);
        assert_eq!(fix_from_sprites(&rom, 1000).len(), 992);
    }

    #[test]
    fn a_size_that_is_not_a_power_of_two_is_left_alone() {
        let mut rom = vec![7u8; 12];
        let c = for_ngh(70).unwrap();
        assert!(!decrypt_sprites(&mut rom, c));
        assert_eq!(rom, vec![7u8; 12]);
    }

    #[test]
    fn decryption_is_deterministic_and_changes_the_data() {
        let mut a: Vec<u8> = (0..4096u32).map(|i| (i * 7 + 3) as u8).collect();
        let orig = a.clone();
        let mut b = a.clone();
        let c = for_ngh(260).unwrap();
        assert!(decrypt_sprites(&mut a, c));
        assert!(decrypt_sprites(&mut b, c));
        assert_eq!(a, b);
        assert_ne!(a, orig);
    }
}
