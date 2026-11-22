use super::Gtia;
use serde::{Deserialize, Serialize};

/// Special-mode pipeline, serialized separately from the published v4 graphics
/// extension. Persist both extensions before resuming any color clocks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GtiaVideoState {
    mode: u8,
    pending: [Option<u8>; 5],
    hires: bool,
    hires_command: Option<bool>,
    pixel: u8,
    background_second: bool,
    delayed_pixel: u8,
    delayed_background: bool,
    exit_blank: bool,
}

impl Default for GtiaVideoState {
    fn default() -> Self {
        Self {
            mode: 0,
            pending: [None; 5],
            // The old per-pixel interface identifies hires by codes 5/6.
            // A wired Machine overrides this with the actual ANx HB command.
            hires: true,
            hires_command: None,
            pixel: 0,
            background_second: false,
            delayed_pixel: 0,
            delayed_background: false,
            exit_blank: false,
        }
    }
}

impl GtiaVideoState {
    pub fn validate(&self) -> Result<(), String> {
        if self.mode > 3
            || self.pixel > 15
            || self.delayed_pixel > 15
            || self.pending.iter().flatten().any(|&m| m > 3)
        {
            return Err("invalid GTIA video state".into());
        }
        Ok(())
    }
}

// AHR 2026-01-02 §6.9, Table 17: the low two AN bus bits, not a
// palette index. Codes 5/6 represent the two hires bits in display order.
// https://www.virtualdub.org/downloads/Altirra%20Hardware%20Reference%20Manual.pdf
fn an_pair(pixels: [u8; 2]) -> u8 {
    if pixels.iter().any(|&p| p == 5 || p == 6) {
        (u8::from(pixels[0] == 6) << 1) | u8::from(pixels[1] == 6)
    } else {
        pixels[0].saturating_sub(1) & 3
    }
}

impl Gtia {
    pub fn save_video(&self) -> GtiaVideoState {
        self.video.clone()
    }

    pub fn restore_video(&mut self, state: GtiaVideoState) -> Result<(), String> {
        state.validate()?;
        self.video = state;
        Ok(())
    }

    /// Deterministic v1-v4 fallback. Those files have no special-mode history.
    pub fn initialize_legacy_video(&mut self) {
        self.video = GtiaVideoState {
            mode: self.registers[27] >> 6,
            hires: self.registers[27] & 0xc0 == 0,
            ..GtiaVideoState::default()
        };
    }

    /// Present ANTIC's horizontal-blank AN=010/011 command to the next tick.
    /// AHR §6.8: hires can only be set here; special modes force it off.
    /// Do not call at visible start: a PRIOR write between HB and display
    /// must retain its effect for the remainder of the scan line.
    /// Call before ticking each clock on which ANTIC drives the HB command,
    /// using that command's mode, not a subsequently changed display-list IR.
    /// The tick applies due PRIOR changes before sampling this command.
    pub fn set_hires(&mut self, hires: bool) {
        self.video.hires_command = Some(hires);
    }

    pub(super) fn video_write(&mut self, value: u8) {
        let mode = value >> 6;
        // AHR §6.10: normal priority is independent of GTIA mode selection.
        // Mode 10 begins at +3; modes 9/11 begin at +4. Leaving 9/11 blanks
        // at +3 then enters pseudo-E at +4. The optional, temperature-sensitive
        // extra artifact at +4 is deliberately not synthesized.
        let delay = if mode == 2 { 3 } else { 4 };
        self.video.pending[delay] = Some(mode);
    }

    pub(super) fn clock_video(
        &mut self,
        playfield: [u8; 2],
        next: [u8; 2],
        hpos: u8,
    ) -> ([u8; 2], u8) {
        let s = &mut self.video;
        s.exit_blank = s.pending[1] == Some(0) && matches!(s.mode, 1 | 3);
        if let Some(mode) = s.pending[0] {
            s.mode = mode;
        }
        s.pending.rotate_left(1);
        s.pending[4] = None;
        if let Some(hires) = s.hires_command.take() {
            s.hires = hires;
        }
        if s.mode != 0 {
            s.hires = false;
        }

        // AHR §§4.8, 6.9: nibble boundaries are fixed to horizontal blank,
        // never the playfield edge or HSCROL. Mode 10 adds exactly one clock.
        let old_pixel = s.pixel;
        let old_background = s.background_second;
        if hpos & 1 == 0 {
            s.pixel = (an_pair(playfield) << 2) | an_pair(next);
            s.background_second = next == [0; 2];
        }
        s.delayed_pixel = if hpos & 1 == 0 { old_pixel } else { s.pixel };
        s.delayed_background = if hpos & 1 == 0 {
            old_background
        } else {
            s.background_second
        };

        let normal = if s.exit_blank {
            [0; 2]
        } else if !s.hires && playfield.iter().any(|&p| p == 5 || p == 6) {
            [an_pair(playfield) + 1; 2]
        } else {
            playfield
        };
        let pf = match s.mode {
            0 => match normal {
                [5, 5] => 0,
                [a, b] if a == 6 || b == 6 => 4,
                [a, _] if (1..=4).contains(&a) => 1 << (a - 1),
                _ => 0,
            },
            2 if s.delayed_pixel & 4 != 0 && !s.delayed_background => 1 << (s.delayed_pixel & 3),
            _ => 0,
        };
        (normal, pf)
    }

    pub(super) fn compose_video(&self, normal: [u8; 2], players: u8, missiles: u8) -> [u8; 2] {
        let s = &self.video;
        if s.exit_blank {
            return [self.compose(0, players, missiles); 2];
        }
        if s.mode == 0 {
            return normal.map(|p| self.compose(p, players, missiles));
        }
        if s.mode == 2 {
            let pixel = s.delayed_pixel;
            // Virtual player colors enter priority but never P/M collisions.
            let virtual_player = if pixel < 4 { 1 << pixel } else { 0 };
            let pf = if pixel & 4 != 0 && !s.delayed_background {
                (pixel & 3) + 1
            } else {
                0
            };
            return [self.compose(pf, players | virtual_player, missiles); 2];
        }

        let fifth = self.graphics.effective[27] & 0x10 != 0;
        let color = self.compose(0, players, missiles);
        // AHR §6.9: a real player/non-fifth missile removes the special
        // playfield entirely, even when a priority conflict produces black.
        if players != 0 || missiles != 0 && !fifth {
            return [color; 2];
        }
        let color = if s.mode == 1 {
            color | s.pixel
        } else if s.pixel == 0 {
            color & 0xf0
        } else {
            color | (s.pixel << 4)
        };
        [color; 2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_preserves_mode_transition_nibble_and_hires_history() {
        let mut g = Gtia::new(false);
        g.write(27, 0x80);
        for _ in 0..136 {
            g.tick_color_clock_with_next([5, 6], [6, 5], true);
        }
        g.write(27, 0);
        g.tick_color_clock_with_next([6, 6], [5, 6], true);
        g.set_hires(true);
        let video = g.save_video();
        let graphics = g.save_graphics();
        let mut restored = g.clone();
        restored.initialize_legacy_video();
        restored.restore_video(video).unwrap();
        restored.restore_graphics(graphics).unwrap();
        for clock in 0..500 {
            if clock % 228 == 0 {
                g.set_hires(true);
                restored.set_hires(true);
            }
            if clock == 200 {
                g.write(27, 0xc0);
                restored.write(27, 0xc0);
            }
            assert_eq!(
                g.tick_color_clock_with_next([5, 6], [6, 5], true),
                restored.tick_color_clock_with_next([5, 6], [6, 5], true)
            );
            assert_eq!(g.save_video(), restored.save_video());
            assert_eq!(g.save_graphics(), restored.save_graphics());
        }
    }

    #[test]
    fn corrupt_video_extension_never_mutates_running_chip() {
        let mut g = Gtia::new(false);
        g.write(27, 0xc0);
        let original = g.save_video();
        for index in 0..4 {
            let mut bad = original.clone();
            match index {
                0 => bad.mode = 4,
                1 => bad.pixel = 16,
                2 => bad.delayed_pixel = 16,
                _ => bad.pending[4] = Some(4),
            }
            assert!(g.restore_video(bad).is_err());
            assert_eq!(g.save_video(), original);
        }
        g.initialize_legacy_video();
        assert_eq!(g.save_video().mode, 3);
        assert!(!g.save_video().hires);
    }
}
