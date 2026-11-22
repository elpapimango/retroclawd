//! Cheat codes (fact H13): Game Genie codes, which patch a word of the ROM, and raw `AAAAAA:DDDD` / `AAAAAA:DD` codes, which
//! patch the ROM (an address below `$400000`) or poke work RAM (`$FF0000` and up) once a frame, as a Pro Action Replay does.
//!
//! There is no lock-on cartridge behind this: the Game Genie's own ROM and its menus are not emulated, only what its codes do.

/// What a code does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cheat {
    /// The word at the (even) ROM address reads as `data`.
    RomWord { addr: u32, data: u16 },
    /// A byte or a word is written to work RAM at every frame.
    Poke { addr: u32, data: u16, word: bool },
}

const GENIE_CHARS: &str = "ABCDEFGHJKLMNPRSTVWXYZ0123456789";

/// Decodes a Game Genie code `XXXX-YYYY` into its address and data. The eight characters carry five bits each, scrambled:
///
/// ```text
///   S     C     R     A  -  M     B     L     E
/// 01111 00010 01110 00000 01011 00001 01010 00100
/// ijklm nopIJ KLMNO PABCD EFGHd efgha bcQRS TUVWX
/// ```
///
/// where `ABCDEFGHIJKLMNOPQRSTUVWX` is the address and `abcdefghijklmnop` the data (Pico's documentation of the code format).
pub fn decode_genie(code: &str) -> Option<(u32, u16)> {
    let mut bits = 0u64;
    let mut n = 0;
    for c in code.chars().filter(|c| *c != '-') {
        n += 1;
        if n > 8 {
            return None;
        }
        bits = bits << 5 | GENIE_CHARS.find(c.to_ascii_uppercase())? as u64;
    }
    if n != 8 {
        return None;
    }
    // position 0 is the first bit of the first character
    let at = |pos: u32| -> u64 { bits >> (39 - pos) & 1 };
    // the positions of A..X and of a..p in the diagram
    const ADDRESS: [u32; 24] = [
        16, 17, 18, 19, 20, 21, 22, 23, 8, 9, 10, 11, 12, 13, 14, 15, 32, 33, 34, 35, 36, 37, 38,
        39,
    ];
    const DATA: [u32; 16] = [29, 30, 31, 24, 25, 26, 27, 28, 0, 1, 2, 3, 4, 5, 6, 7];
    let addr = ADDRESS.iter().fold(0u64, |a, &p| a << 1 | at(p));
    let data = DATA.iter().fold(0u64, |a, &p| a << 1 | at(p));
    Some((addr as u32, data as u16))
}

/// Parses a list of codes separated by commas, spaces or semicolons.
pub fn parse(text: &str) -> Result<Vec<Cheat>, String> {
    let mut out = Vec::new();
    for token in text.split([',', ' ', ';']).filter(|t| !t.is_empty()) {
        if let Some((a, d)) = token.split_once(':') {
            let addr = u32::from_str_radix(a, 16)
                .map_err(|_| format!("cheat \"{token}\": bad address"))?;
            let data =
                u16::from_str_radix(d, 16).map_err(|_| format!("cheat \"{token}\": bad data"))?;
            let addr = addr & 0x00FF_FFFF;
            let word = d.len() > 2;
            if addr < 0x40_0000 {
                if !word {
                    return Err(format!(
                        "cheat \"{token}\": a ROM patch is a word (4 digits)"
                    ));
                }
                out.push(Cheat::RomWord {
                    addr: addr & !1,
                    data,
                });
            } else if addr >= 0xE0_0000 {
                out.push(Cheat::Poke { addr, data, word });
            } else {
                return Err(format!(
                    "cheat \"{token}\": the address is neither ROM nor work RAM"
                ));
            }
        } else {
            let (addr, data) = decode_genie(token).ok_or_else(|| {
                format!("cheat \"{token}\": not a Game Genie code (XXXX-YYYY) or AAAAAA:DDDD")
            })?;
            out.push(Cheat::RomWord {
                addr: addr & 0x3F_FFFE,
                data,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_example_decodes() {
        assert_eq!(decode_genie("SCRA-MBLE"), Some((0x059C44, 0xB078)));
        assert_eq!(decode_genie("scramble"), Some((0x059C44, 0xB078)));
    }

    #[test]
    fn bad_codes_are_refused() {
        assert_eq!(decode_genie("SCRA-MBL"), None);
        assert_eq!(decode_genie("SCRA-MBLEE"), None);
        assert_eq!(
            decode_genie("SCRI-MBLE"),
            None,
            "there is no I in the alphabet"
        );
        assert!(parse("zzz").is_err());
        assert!(parse("123456:12").is_err(), "a ROM patch is a word");
        assert!(parse("A00000:1234").is_err(), "not ROM and not RAM");
    }

    #[test]
    fn lists_mix_both_kinds() {
        let c = parse("SCRA-MBLE, FF1234:56 000200:4E71").unwrap();
        assert_eq!(
            c,
            vec![
                Cheat::RomWord {
                    addr: 0x059C44,
                    data: 0xB078
                },
                Cheat::Poke {
                    addr: 0xFF1234,
                    data: 0x56,
                    word: false
                },
                Cheat::RomWord {
                    addr: 0x200,
                    data: 0x4E71
                },
            ]
        );
    }
    #[test]
    fn random_text_never_panics_the_parser() {
        let alphabet = b"ABCDEFGHJKLMNPRSTVWXYZ0123456789-:, ;iIoO";
        let mut seed = 7u32;
        let sweep: usize = std::env::var("RC_SWEEP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        for _ in 0..20_000 * sweep {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let len = (seed >> 24) as usize % 20;
            let mut text = String::new();
            for _ in 0..len {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                text.push(alphabet[(seed >> 8) as usize % alphabet.len()] as char);
            }
            let _ = parse(&text);
            let _ = decode_genie(&text);
        }
    }
}
