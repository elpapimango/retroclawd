use super::Gtia;
use serde::{Deserialize, Serialize};

/// Timing extension to the original serialized GTIA registers and input state.
/// Save alongside `Gtia`, then restore before resuming any chip clocks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GtiaGraphicsState {
    effective: [u8; 32],
    pending: [[Option<u8>; 32]; 6],
    shifts: [u8; 8],
    stretch: [u8; 8],
    clock: u16,
    pub(crate) collisions: [u8; 16],
    output: [[u8; 2]; 2],
}

impl GtiaGraphicsState {
    pub fn validate(&self) -> Result<(), String> {
        if self.clock >= 228
            || self.stretch.iter().any(|&s| s > 3)
            || self.shifts[4..].iter().any(|&s| s & 0x3f != 0)
            || self.collisions.iter().any(|&c| c > 15)
            || self.output.iter().flatten().any(|&mask| mask > 15)
            || (0..4).any(|p| self.collisions[12 + p] & (1 << p) != 0)
        {
            return Err("invalid GTIA graphics state".into());
        }
        for (delay, registers) in self.pending.iter().enumerate() {
            for (register, value) in registers.iter().enumerate() {
                if value.is_some() && delay > write_delay(register) {
                    return Err("invalid GTIA pending register write".into());
                }
            }
        }
        Ok(())
    }
}

// Altirra Hardware Reference 2026-01-02 §6.10, Table 18.
// URL in crate documentation. Delays are measured from a write before the
// next color clock; the first tick after a write is elapsed clock zero.
fn write_delay(index: usize) -> usize {
    match index {
        0..=7 => 5,
        // Size changes reach the stretch machine after three clocks. The
        // two output clocks complete the five-clock visible delay (§6.10).
        // HPOS writes instead target the comparator's own deadline.
        8..=12 => 3,
        // Graphics latch to shifter: one clock, then two output clocks.
        13..=17 => 1,
        18..=26 => 1,
        27 => 2,
        _ => 0,
    }
}

impl Gtia {
    pub fn save_graphics(&self) -> GtiaGraphicsState {
        self.graphics.clone()
    }

    /// Reject an invalid extension before changing the running chip.
    pub fn restore_graphics(&mut self, state: GtiaGraphicsState) -> Result<(), String> {
        state.validate()?;
        self.graphics = state;
        Ok(())
    }

    /// Explicit v1-v3 fallback: retain register latches, clear collisions and
    /// shifters, and start at raster cycle zero. Old states had no PM timing.
    pub fn initialize_legacy_graphics(&mut self) {
        self.graphics = GtiaGraphicsState {
            effective: self.registers,
            ..GtiaGraphicsState::default()
        };
    }

    pub(super) fn graphics_write(&mut self, index: usize, value: u8) {
        if index == 30 {
            // §6.6: HITCLR clears latches, not the shifting graphics.
            self.graphics.collisions.fill(0);
        } else {
            self.graphics.pending[write_delay(index)][index] = Some(value);
        }
    }
}

impl Gtia {
    /// Begin raster cycle zero. Never clear the continuously running shifters.
    pub fn begin_scanline(&mut self) {
        self.graphics.clock = 0;
    }

    /// ANTIC's missile bus transfer (normally raster cycle 0, §4.13).
    /// VDELAY masks even-line loads independently for each two-bit missile.
    pub fn dma_missiles(&mut self, value: u8, odd_line: bool) {
        if self.registers[29] & 1 == 0 {
            return;
        }
        let mut data = self.registers[17];
        for m in 0..4 {
            if odd_line || self.registers[28] & (1 << m) == 0 {
                let mask = 3 << (m * 2);
                data = (data & !mask) | (value & mask);
            }
        }
        self.registers[17] = data;
        self.graphics_write(17, data);
    }

    /// ANTIC's player bus transfer (normally raster cycles 2-5, §4.13).
    /// CPU GRAFP writes do not use these GRACTL/VDELAY gates (§6.5).
    pub fn dma_player(&mut self, index: usize, value: u8, odd_line: bool) -> Result<(), String> {
        if index >= 4 {
            return Err("GTIA player index exceeds 3".into());
        }
        if self.registers[29] & 2 != 0 && (odd_line || self.registers[28] & (0x10 << index) == 0) {
            self.registers[13 + index] = value;
            self.graphics_write(13 + index, value);
        }
        Ok(())
    }

    /// Clock one color clock and return its two half-color-clock palette codes.
    ///
    /// Call 228 times per line, including blank, twice per ANTIC raster cycle.
    /// At cycle 65 HPOS is $80 (§6.10); cycle zero covers $E2/$E3, cycle one
    /// $00/$01. Thus the 384-pixel crop starting at HPOS $20 begins at call 34
    /// (raster cycle 17). Positions >= $E4 never trigger. `visible_line` is
    /// ANTIC's actual vertical-blank gate (normally lines 8-247, §6.6).
    ///
    /// Playfield codes match `color`: 0 background, 1-4 PF0-PF3, 5 hires
    /// zero and 6 hires one. Supply both half-pixels together: hires collision
    /// input is their OR, while priority always sees PF2 (§6.8).
    pub fn tick_color_clock(&mut self, playfield: [u8; 2], visible_line: bool) -> [u8; 2] {
        let s = &mut self.graphics;
        for (index, value) in s.pending[0].iter().enumerate() {
            if let Some(value) = value {
                s.effective[index] = *value;
            }
        }
        s.pending.rotate_left(1);
        s.pending[5].fill(None);
        let hpos = ((s.clock + 226) % 228) as u8;
        s.clock = (s.clock + 1) % 228;
        let mut players = 0;
        let mut missiles = 0;
        for object in 0..8 {
            let player = object < 4;
            let index = object & 3;
            // The comparator/shifter precedes the collision/video stage by
            // two color clocks. Keep the HPOS coordinate at the output stage.
            // Independent proof: Acid800 gtia_pmresize.s runtest/testpat1-7;
            // flattening these stages changes every 4x->1x test pattern.
            if (hpos + 2) % 228 == s.effective[object] {
                let data = if player {
                    s.effective[13 + index]
                } else {
                    ((s.effective[17] >> (index * 2)) & 3) << 6
                };
                // §6.5: forcing a nonzero stretch state to zero also clocks
                // the old bits once before OR-loading the new graphics.
                // Acid800 gtia_pmoverlap.s, wide_data_4 ($65 -> $78).
                if s.stretch[object] != 0 {
                    s.shifts[object] <<= 1;
                }
                s.shifts[object] |= data;
                s.stretch[object] = 0;
            }
            if s.shifts[object] & 0x80 != 0 {
                if player {
                    players |= 1 << index;
                } else {
                    missiles |= 1 << index;
                }
            }
            let size = if player {
                s.effective[8 + index] & 3
            } else {
                (s.effective[12] >> (index * 2)) & 3
            };
            // §6.5: real two-bit state machine, including size-%10 lockup.
            s.stretch[object] = (s.stretch[object] + 1) & size;
            if s.stretch[object] == 0 {
                s.shifts[object] <<= 1;
            }
        }
        let shifted = [players, missiles];
        [players, missiles] = s.output[0];
        s.output[0] = s.output[1];
        s.output[1] = shifted;
        if !visible_line || !(0x22..=0xdd).contains(&hpos) {
            players = 0;
            missiles = 0;
        } else {
            let pf = if playfield.iter().any(|&c| c == 5 || c == 6) {
                if playfield.contains(&6) { 4 } else { 0 }
            } else {
                match playfield[0] {
                    1..=4 => 1 << (playfield[0] - 1),
                    _ => 0,
                }
            };
            for index in 0..4 {
                if missiles & (1 << index) != 0 {
                    s.collisions[index] |= pf;
                    s.collisions[8 + index] |= players;
                }
                if players & (1 << index) != 0 {
                    s.collisions[4 + index] |= pf;
                    s.collisions[12 + index] |= players & !(1 << index);
                }
            }
        }
        playfield.map(|code| self.compose(code, players, missiles))
    }

    fn compose(&self, code: u8, mut players: u8, missiles: u8) -> u8 {
        let r = &self.graphics.effective;
        let prior = r[27];
        let fifth = prior & 0x10 != 0 && missiles != 0;
        if prior & 0x10 == 0 {
            players |= missiles;
        }
        let pf = match code {
            1..=4 => 1 << (code - 1),
            5 | 6 => 4,
            _ => 0,
        } | if fifth { 8 } else { 0 };
        let p01 = players & 3 != 0;
        let p23 = players & 12 != 0;
        let pf01 = pf & 3 != 0;
        let pf23 = pf & 12 != 0;
        let pri0 = prior & 1 != 0;
        let pri1 = prior & 2 != 0;
        let pri2 = prior & 4 != 0;
        let pri3 = prior & 8 != 0;
        let multi = prior & 0x20 != 0;
        // §6.7: evaluate the independent output-enable equations, rather than
        // choosing a single winner. This retains mode-zero OR and black holes.
        let sp01 = !(pf01 && (pri2 || pri3) || pri2 && pf23);
        let sp23 = !(p01 || pf23 && (pri1 || pri2) || pf01 && !pri0);
        let sf3 = pf & 8 != 0 && !(p23 && (pri0 || pri3) || p01 && !pri2);
        let sf01 = !(p23 && pri0 || p01 && (pri0 || pri1) || sf3);
        let sf2 = pf & 4 != 0 && !(p23 && (pri0 || pri3) || p01 && !pri2 || sf3);
        let mut color = 0;
        for (index, enabled) in [
            players & 1 != 0 && sp01,
            players & 2 != 0 && sp01 && (players & 1 == 0 || multi),
            players & 4 != 0 && sp23,
            players & 8 != 0 && sp23 && (players & 4 == 0 || multi),
        ]
        .into_iter()
        .enumerate()
        {
            if enabled {
                color |= r[18 + index];
            }
        }
        for (index, enabled) in [pf & 1 != 0 && sf01, pf & 2 != 0 && sf01, sf2, sf3]
            .into_iter()
            .enumerate()
        {
            if enabled {
                color |= r[22 + index];
            }
        }
        if !p01 && !p23 && !pf01 && !pf23 {
            color = r[26];
        }
        if code == 6 {
            color = (color & 0xf0) | (r[23] & 0x0e);
        }
        color & 0xfe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_extension_is_rejected_without_mutation() {
        let mut g = Gtia::new(false);
        g.write(0, 0x80);
        g.write(13, 0xff);
        for _ in 0..140 {
            g.tick_color_clock([1; 2], true);
        }
        let original = g.save_graphics();
        for case in 0..7 {
            let mut corrupt = original.clone();
            match case {
                0 => corrupt.clock = 228,
                1 => corrupt.stretch[0] = 4,
                2 => corrupt.shifts[4] = 1,
                3 => corrupt.collisions[0] = 16,
                4 => corrupt.collisions[12] = 1,
                5 => corrupt.pending[5][18] = Some(0x42),
                _ => corrupt.output[0][0] = 16,
            }
            assert!(g.restore_graphics(corrupt).is_err());
            assert_eq!(g.save_graphics(), original);
        }
    }

    #[test]
    fn checkpoint_keeps_pending_writes_collision_latches_and_running_stretch() {
        let mut g = Gtia::new(false);
        g.write(0, 0x80);
        g.write(8, 3);
        g.write(13, 0x81);
        for _ in 0..133 {
            g.tick_color_clock([2; 2], true);
        }
        g.write(8, 2);
        g.write(0, 0x90);
        let state = g.save_graphics();
        let mut restored = Gtia::new(false);
        restored.registers = g.registers;
        restored.restore_graphics(state).unwrap();
        for clock in 0..700 {
            let pf = if clock & 1 == 0 { [5, 6] } else { [2; 2] };
            assert_eq!(
                restored.tick_color_clock(pf, clock < 100),
                g.tick_color_clock(pf, clock < 100)
            );
            assert_eq!(restored.save_graphics(), g.save_graphics());
        }
        restored.initialize_legacy_graphics();
        assert_eq!(restored.graphics.effective, restored.registers);
        assert_eq!(restored.graphics.clock, 0);
        assert_eq!(restored.graphics.shifts, [0; 8]);
        assert_eq!(restored.graphics.collisions, [0; 16]);
    }
}
