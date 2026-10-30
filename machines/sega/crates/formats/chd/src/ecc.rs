//! The P and Q parity bytes of a CD-ROM data sector (ECMA-130 annex A). A CD codec stores a sector whose sync pattern and parity are
//! the expected ones without them, and this puts them back.
//!
//! The sector's bytes 12 to 2075 (header, user data, EDC and 8 reserved bytes) form 24 rows of 86 bytes. A P vector is one
//! column of 24 bytes; a Q vector is a diagonal of 43 bytes; each vector gets two parity bytes from a Reed-Solomon code over
//! GF(2^8) (polynomial 0x11D): the XOR of the vector (A), and the vector evaluated by Horner's rule at the field's generator (B).

/// The 12 bytes that start every raw data sector.
pub const SYNC: [u8; 12] = [
    0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0,
];

const P_START: usize = 0x81C;
const P_BYTES: usize = 86;
const P_LEN: usize = 24;
const Q_START: usize = P_START + 2 * P_BYTES;
const Q_BYTES: usize = 52;
const Q_LEN: usize = 43;
/// The parity covers sector bytes 12..2076.
const COVERED: usize = 12;
/// A Q vector steps 88 bytes and wraps at 26 rows of 86.
const Q_STEP: usize = 88;
const Q_WRAP: usize = 26 * 86;
/// Where the mode byte is.
const MODE: usize = 15;

/// Multiplication by the generator of GF(2^8) with x^8 = x^4 + x^3 + x^2 + 1.
fn times_generator(x: u8) -> u8 {
    let doubled = u16::from(x) << 1;
    (if doubled & 0x100 != 0 {
        doubled ^ 0x11D
    } else {
        doubled
    }) as u8
}

/// Multiplication by 1 / (generator + 1), which turns the two sums of a vector into its first parity byte.
fn times_inverse_g_plus_one(x: u8) -> u8 {
    // 0xF4 is the inverse of 0x03 (generator + 1) in this field; multiply by shift and add
    let (mut acc, mut a, mut b) = (0u8, x, 0xF4u8);
    while b != 0 {
        if b & 1 != 0 {
            acc ^= a;
        }
        a = times_generator(a);
        b >>= 1;
    }
    acc
}

/// The byte `offset` into the covered area; in a mode 2 sector the four header bytes count as zero.
fn covered_byte(sector: &[u8; 2352], offset: usize) -> u8 {
    if sector[MODE] == 2 && offset < 4 {
        0
    } else {
        sector[COVERED + offset]
    }
}

/// The two parity bytes of the vector whose members are `members` (offsets into the covered area).
fn parity(sector: &[u8; 2352], members: impl Iterator<Item = usize>) -> (u8, u8) {
    let (mut horner, mut sum) = (0u8, 0u8);
    for offset in members {
        let byte = covered_byte(sector, offset);
        horner = times_generator(horner ^ byte);
        sum ^= byte;
    }
    let first = times_inverse_g_plus_one(times_generator(horner) ^ sum);
    (first, sum ^ first)
}

/// Writes the P and Q parity of a raw data sector into it.
pub(crate) fn generate(sector: &mut [u8; 2352]) {
    for column in 0..P_BYTES {
        let (a, b) = parity(sector, (0..P_LEN).map(|row| column + P_BYTES * row));
        sector[P_START + column] = a;
        sector[P_START + P_BYTES + column] = b;
    }
    for vector in 0..Q_BYTES {
        let start = 86 * (vector / 2) + vector % 2;
        let (a, b) = parity(sector, (0..Q_LEN).map(|k| (start + Q_STEP * k) % Q_WRAP));
        sector[Q_START + vector] = a;
        sector[Q_START + Q_BYTES + vector] = b;
    }
}

/// Whether a sector has the sync pattern and the parity a codec would have taken out.
#[cfg(any(test, feature = "builder"))]
pub(crate) fn is_regenerable(sector: &[u8; 2352]) -> bool {
    let mut copy = *sector;
    copy[..12].copy_from_slice(&SYNC);
    generate(&mut copy);
    copy == *sector
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> [u8; 2352] {
        let mut s = [0u8; 2352];
        s[..12].copy_from_slice(&SYNC);
        s[12..16].copy_from_slice(&[0x00, 0x02, 0x00, 1]);
        for (i, b) in s[16..2064].iter_mut().enumerate() {
            *b = (i as u32 * 31 + 7) as u8;
        }
        s
    }

    #[test]
    fn the_field_arithmetic_is_the_discs() {
        assert_eq!(times_generator(0x80), 0x1D);
        assert_eq!(times_generator(0x01), 0x02);
        // 0xF4 is the inverse of generator + 1: (g + 1) * 0xF4 = 1
        assert_eq!(times_inverse_g_plus_one(0x03), 0x01);
        assert_eq!(times_inverse_g_plus_one(0), 0);
    }

    #[test]
    fn generated_parity_makes_both_syndromes_of_every_vector_zero() {
        let mut s = sample();
        generate(&mut s);
        // a codeword (data..., first parity, second parity) has zero sum and zero Horner value at the generator
        let check = |members: Vec<usize>, p1: usize, p2: usize| {
            let (mut sum, mut horner) = (0u8, 0u8);
            for byte in members.into_iter().map(|o| s[12 + o]).chain([s[p1], s[p2]]) {
                sum ^= byte;
                horner = times_generator(horner ^ byte);
            }
            assert_eq!((sum, horner), (0, 0));
        };
        for c in 0..P_BYTES {
            check(
                (0..P_LEN).map(|r| c + 86 * r).collect(),
                P_START + c,
                P_START + P_BYTES + c,
            );
        }
        for v in 0..Q_BYTES {
            let start = 86 * (v / 2) + v % 2;
            check(
                (0..Q_LEN).map(|k| (start + Q_STEP * k) % Q_WRAP).collect(),
                Q_START + v,
                Q_START + Q_BYTES + v,
            );
        }
    }

    #[test]
    fn a_sector_is_recognised_only_with_its_own_parity() {
        let mut s = sample();
        assert!(!is_regenerable(&s));
        generate(&mut s);
        assert!(is_regenerable(&s));
        s[100] ^= 1;
        assert!(!is_regenerable(&s));
    }

    #[test]
    fn a_mode_2_sector_ignores_its_header_in_the_parity() {
        let mut a = sample();
        a[15] = 2;
        let mut b = a;
        b[12..16].copy_from_slice(&[0, 0, 0, 2]); // another address; the mode byte stays 2
        b[12] = 0x12;
        b[13] = 0x34;
        generate(&mut a);
        generate(&mut b);
        assert_eq!(a[P_START..], b[P_START..]);
    }
}
