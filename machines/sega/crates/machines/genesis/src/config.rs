//! What a machine is built from, and the master-clock constants of its regions.

/// The console's region: it decides the video standard (fact rows C1-C9) and the version register ([O] P01).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Region {
    /// Japan, NTSC (domestic).
    Japan,
    /// The Americas, NTSC (overseas).
    Usa,
    /// Europe, PAL (overseas).
    Europe,
}

impl Region {
    pub fn is_pal(self) -> bool {
        self == Region::Europe
    }

    /// The version register, `$A10001`: bit 7 overseas, bit 6 PAL, bit 5 no expansion unit (fact I6, [O] P01: a plain
    /// console reads `$A0` in the Americas, `$E0` in Europe and `$20` in Japan; the version number in bits 3-0 is 0).
    pub fn version_register(self) -> u8 {
        match self {
            Region::Japan => 0x20,
            Region::Usa => 0xA0,
            Region::Europe => 0xE0,
        }
    }

    /// The master clock in Hz (fact C1).
    pub fn master_hz(self) -> u64 {
        if self.is_pal() {
            53_203_424
        } else {
            53_693_175
        }
    }

    /// Lines per frame (fact C8 and C9).
    pub fn lines(self) -> u32 {
        if self.is_pal() { 313 } else { 262 }
    }

    /// The region a console picks for a cartridge whose header lists `regions` ([O] P01b: Genesis Plus GX, PicoDrive and
    /// BlastEm all prefer the Americas over Japan and Europe; for Japan together with Europe Genesis Plus GX chooses Japan
    /// and the other two Europe: Genesis Plus GX is followed). A header listing nothing gives the Americas.
    pub fn from_header(regions: md_cart::Regions) -> Region {
        if regions.usa {
            Region::Usa
        } else if regions.japan {
            Region::Japan
        } else if regions.europe {
            Region::Europe
        } else {
            Region::Usa
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Region::Japan => "japan",
            Region::Usa => "usa",
            Region::Europe => "europe",
        }
    }

    pub fn parse(text: &str) -> Option<Region> {
        match text.to_ascii_lowercase().as_str() {
            "japan" | "jp" | "j" | "ntsc-j" => Some(Region::Japan),
            "usa" | "us" | "u" | "ntsc" | "ntsc-u" => Some(Region::Usa),
            "europe" | "eu" | "e" | "pal" => Some(Region::Europe),
            _ => None,
        }
    }
}

/// Master clocks per scanline (fact C7).
pub const LINE_CLOCKS: u64 = 3420;
/// The 68000 clock is the master clock divided by 7 (fact C2) and the Z80's by 15 (fact C3).
pub const M68K_DIVIDER: u64 = 7;
pub const Z80_DIVIDER: u64 = 15;

/// Master clocks per frame.
pub fn frame_clocks(region: Region) -> u64 {
    u64::from(region.lines()) * LINE_CLOCKS
}

#[derive(Clone, Debug)]
pub struct Config {
    /// The cartridge ROM, as the 68000 sees it from `$000000` (see `md-cart`).
    pub rom: Vec<u8>,
    pub region: Region,
    /// What is plugged into the two controller ports.
    pub pads: [crate::io::PadKind; 2],
    /// The FM chip: the discrete YM2612 of the first consoles or the YM3438 core of the later ones.
    pub fm_chip: ym2612::Chip,
}

impl Config {
    pub fn new(rom: Vec<u8>, region: Region) -> Self {
        Self {
            rom,
            region,
            pads: [crate::io::PadKind::Six; 2],
            fm_chip: ym2612::Chip::Ym2612,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_lengths_are_the_documented_ones() {
        assert_eq!(frame_clocks(Region::Usa), 896_040);
        assert_eq!(frame_clocks(Region::Japan), 896_040);
        assert_eq!(frame_clocks(Region::Europe), 1_070_460);
    }

    #[test]
    fn the_version_register_follows_the_observed_values() {
        assert_eq!(Region::Usa.version_register(), 0xA0);
        assert_eq!(Region::Europe.version_register(), 0xE0);
        assert_eq!(Region::Japan.version_register(), 0x20);
    }

    #[test]
    fn the_header_picks_the_region_like_genesis_plus_gx() {
        let r = |j, u, e| {
            Region::from_header(md_cart::Regions {
                japan: j,
                usa: u,
                europe: e,
            })
        };
        assert_eq!(r(true, true, true), Region::Usa);
        assert_eq!(r(true, false, true), Region::Japan);
        assert_eq!(r(false, false, true), Region::Europe);
        assert_eq!(r(true, false, false), Region::Japan);
        assert_eq!(r(false, false, false), Region::Usa);
    }

    #[test]
    fn region_names_round_trip() {
        for r in [Region::Japan, Region::Usa, Region::Europe] {
            assert_eq!(Region::parse(r.name()), Some(r));
        }
        assert_eq!(Region::parse("pal"), Some(Region::Europe));
        assert_eq!(Region::parse("mars"), None);
    }
}
