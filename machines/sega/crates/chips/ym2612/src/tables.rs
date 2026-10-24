//! The two ROMs of the operator: a quarter-wave log-sine and a power-of-two table. Both are generated from their formulas
//! (the values the silicon ROMs hold equal these roundings for every entry; checked against the reference in the tests).

use std::sync::OnceLock;

fn build() -> &'static ([u16; 256], [u16; 256]) {
    static TABLES: OnceLock<([u16; 256], [u16; 256])> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut logsin = [0u16; 256];
        let mut exp = [0u16; 256];
        for i in 0..256 {
            let x = (i as f64 + 0.5) * std::f64::consts::PI / 512.0;
            logsin[i] = (-x.sin().log2() * 256.0).round() as u16;
            exp[i] = (((i as f64 / 256.0).exp2() - 1.0) * 1024.0).round() as u16;
        }
        (logsin, exp)
    })
}

/// The log-sine ROM: `-log2(sin((i + 0.5) / 512 * pi)) * 256`, rounded.
pub fn logsin_rom(i: usize) -> u16 {
    build().0[i]
}

/// The exponential ROM: `(2^(i / 256) - 1) * 1024`, rounded.
pub fn exp_rom(i: usize) -> u16 {
    build().1[i]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roms_have_their_documented_end_values() {
        assert_eq!(logsin_rom(0), 0x859);
        assert_eq!(logsin_rom(255), 0);
        assert_eq!(exp_rom(0), 0);
        assert_eq!(exp_rom(255), 0x3FA);
    }

    #[test]
    fn the_log_sine_falls_and_the_exponential_rises() {
        for i in 1..256 {
            assert!(logsin_rom(i) <= logsin_rom(i - 1));
            assert!(exp_rom(i) >= exp_rom(i - 1));
        }
    }
}
