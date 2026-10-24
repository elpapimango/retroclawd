use machine::{Addr, AudioFormat, FrameRate, Machine, RunStats};

/// Width and height of the frame until the VDP draws one.
const BLANK_WIDTH: u32 = 320;
const BLANK_HEIGHT: u32 = 224;

/// The NTSC master clock in Hz and the master clocks of one NTSC frame (262 lines of 3420), both unsourced until
/// `docs/hardware-facts.md` has their rows (Task 2).
const NTSC_MASTER_HZ: u64 = 53_693_175;
const NTSC_FRAME_CLOCKS: u32 = 262 * 3420;

pub struct Genesis {
    master: u64,
}

impl Genesis {
    pub fn new() -> Self {
        Self { master: 0 }
    }
}

impl Default for Genesis {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine for Genesis {
    fn clock_hz(&self) -> u64 {
        NTSC_MASTER_HZ
    }

    fn frame_hz(&self) -> u64 {
        NTSC_MASTER_HZ / u64::from(NTSC_FRAME_CLOCKS)
    }

    fn frame_rate(&self) -> FrameRate {
        FrameRate {
            num: NTSC_MASTER_HZ as u32,
            den: NTSC_FRAME_CLOCKS,
        }
    }

    fn audio_format(&self) -> AudioFormat {
        AudioFormat {
            channels: 2,
            sample_rate: 53_267,
        }
    }

    fn run(&mut self, running: &mut dyn FnMut(&dyn Machine) -> bool) -> RunStats {
        machine::Clock::new(self.frame_hz()).run(self, running)
    }

    fn step(&mut self) -> u32 {
        self.master += 1;
        1
    }

    fn halted(&self) -> bool {
        false
    }

    fn pc(&self) -> Addr {
        0
    }

    fn registers(&self) -> Vec<(&'static str, u64)> {
        Vec::new()
    }

    fn render_frame(&self) -> Option<(u32, u32, Vec<u8>)> {
        Some((
            BLANK_WIDTH,
            BLANK_HEIGHT,
            vec![0; (BLANK_WIDTH * BLANK_HEIGHT * 3) as usize],
        ))
    }

    fn read_memory(&self, _addr: Addr) -> u8 {
        0xFF
    }

    fn load(&mut self, _addr: Addr, _bytes: &[u8]) -> usize {
        0
    }

    fn set_pc(&mut self, _addr: Addr) {}

    fn save_state(&self) -> Vec<u8> {
        Vec::new()
    }

    fn load_state(&mut self, _bytes: &[u8]) -> Result<(), String> {
        Err("the Genesis has no save state yet".to_string())
    }

    fn reset(&mut self) {
        self.master = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_rate_is_the_exact_ntsc_ratio() {
        let g = Genesis::new();
        assert_eq!(g.frame_rate().num, 53_693_175);
        assert_eq!(g.frame_rate().den, 896_040);
        assert_eq!(g.audio_format().channels, 2);
    }
}
