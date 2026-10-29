//! The CD drive's microcontroller as the gate array's CDD port sees it: ten status nibbles in, ten command nibbles out, a
//! level 4 interrupt every 1/75 s while the host clock runs (facts E1-E6 in `docs/hardware-facts.md`).
//!
//! The port itself (the packets, the checksum, the handshake) lives here; what the drive does with a command is a `Drive`, which
//! the machine plugs in: `NoDisc` here, a disc-backed one in the machine crate.

use crate::cdc::Cdc;

/// Sub clocks per second (fact C10) and drive periods per second (the CD frame rate).
const SUB_CLOCK_HZ: u64 = 12_500_000;
const PERIODS_PER_SECOND: u64 = 75;

/// Drive status nibbles (RS0), from the drive firmware's own list.
pub mod status {
    pub const STOP: u8 = 0x0;
    pub const PLAY: u8 = 0x1;
    pub const SEEK: u8 = 0x2;
    pub const SCAN: u8 = 0x3;
    pub const PAUSE: u8 = 0x4;
    pub const DOOR_OPEN: u8 = 0x5;
    pub const CHECKSUM_ERROR: u8 = 0x6;
    pub const COMMAND_ERROR: u8 = 0x7;
    pub const FUNCTION_ERROR: u8 = 0x8;
    pub const TOC_READ: u8 = 0x9;
    pub const NO_DISC: u8 = 0xB;
}

/// Drive command nibbles (TC0).
pub mod command {
    pub const NOP: u8 = 0x0;
    pub const STOP: u8 = 0x1;
    pub const REPORT_REQUEST: u8 = 0x2;
    /// Starts playing or reading at a time: an audio track is played, a data track is read (fact E17).
    pub const READ: u8 = 0x3;
    pub const SEEK: u8 = 0x4;
    pub const PAUSE: u8 = 0x6;
    /// Resumes after a pause, at the time given when there is one.
    pub const PLAY: u8 = 0x7;
    pub const FORWARD: u8 = 0x8;
    pub const REVERSE: u8 = 0x9;
    pub const TRACK_SKIP: u8 = 0xA;
    pub const TRACK_CUE: u8 = 0xB;
    pub const DOOR_CLOSE: u8 = 0xC;
    pub const DOOR_OPEN: u8 = 0xD;
}

/// What sits in the drive. It answers commands and delivers sectors to the controller.
pub trait Drive: Send {
    /// A command arrived (TC0-TC9); returns the status values RS0-RS8 (the port adds the checksum as RS9).
    fn command(&mut self, tc: &[u8; 10]) -> [u8; 9];
    /// A period of 1/75 s passed: the drive may hand a sector to the controller.
    fn period(&mut self, cdc: &mut Cdc);
    /// Whether audio is being played: bit 8 of the control register (data or music) reads 0 then and 1 otherwise.
    fn music_playing(&self) -> bool {
        false
    }
    /// Moves the audio the drive played since the last call to `out`: interleaved left and right 16-bit samples at 44.1 kHz
    /// (588 stereo frames for each period of playing, fact E17).
    fn take_audio(&mut self, _out: &mut Vec<i16>) {}
    fn box_clone(&self) -> Box<dyn Drive>;
}

impl Clone for Box<dyn Drive> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}

/// An empty drive: every command is answered with "no disc", the tray commands move the tray.
#[derive(Clone, Debug)]
pub struct NoDisc {
    status: u8,
}

impl NoDisc {
    pub fn new() -> Self {
        Self {
            status: status::NO_DISC,
        }
    }
}

impl Default for NoDisc {
    fn default() -> Self {
        Self::new()
    }
}

impl Drive for NoDisc {
    fn command(&mut self, tc: &[u8; 10]) -> [u8; 9] {
        match tc[0] {
            command::DOOR_OPEN => self.status = status::DOOR_OPEN,
            command::DOOR_CLOSE => self.status = status::NO_DISC,
            _ => {}
        }
        let mut rs = [0; 9];
        rs[0] = self.status;
        rs
    }

    fn period(&mut self, _cdc: &mut Cdc) {}

    fn box_clone(&self) -> Box<dyn Drive> {
        Box::new(self.clone())
    }
}

#[derive(Clone)]
pub struct Cdd {
    /// Status nibbles RS0-RS9 as the sub-CPU reads them.
    rs: [u8; 10],
    /// Command nibbles TC0-TC9 as the sub-CPU wrote them.
    tc: [u8; 10],
    hock: bool,
    /// The host wrote the last command nibble since the drive last answered.
    command_pending: bool,
    drive: Box<dyn Drive>,
    /// Sub clocks times 75, not yet a whole period.
    acc: u64,
    /// Debugging aid: the commands received and the answers given.
    log: Vec<([u8; 10], [u8; 10])>,
    /// CD audio played and not yet taken, interleaved left and right.
    audio: Vec<i16>,
}

impl Default for Cdd {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Cdd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cdd")
            .field("rs", &self.rs)
            .field("tc", &self.tc)
            .field("hock", &self.hock)
            .finish()
    }
}

/// The checksum nibble of a packet: the sum of the first nine nibbles, inverted, low four bits (fact E2).
pub fn checksum(nibbles: &[u8; 10]) -> u8 {
    let sum: u32 = nibbles[..9].iter().map(|n| u32::from(*n & 0xF)).sum();
    (!sum & 0xF) as u8
}

impl Cdd {
    pub fn new() -> Self {
        // power on: an all-zero packet with its checksum, whatever the drive holds (fact E3); the drive state shows with the
        // answer to the first command
        let mut rs = [0; 10];
        rs[9] = checksum(&rs);
        Cdd {
            rs,
            tc: [0; 10],
            hock: false,
            command_pending: false,
            drive: Box::new(NoDisc::new()),
            acc: 0,
            log: Vec::new(),
            audio: Vec::new(),
        }
    }

    /// Plugs in another drive (a disc drive); the port keeps its state.
    pub fn set_drive(&mut self, drive: Box<dyn Drive>) {
        self.drive = drive;
    }

    pub fn hock(&self) -> bool {
        self.hock
    }

    /// Sets the host clock enable; returns `true` when this starts the communication (0 to 1).
    pub fn set_hock(&mut self, on: bool) -> bool {
        let rising = on && !self.hock;
        self.hock = on;
        rising
    }

    /// Bit 8 of `$FF8036`: 1 when the drive is not playing music (stopped, paused or reading data; fact E16).
    pub fn data_mode(&self) -> bool {
        !self.drive.music_playing()
    }

    pub fn rs(&self, i: usize) -> u8 {
        self.rs[i]
    }

    pub fn tc(&self, i: usize) -> u8 {
        self.tc[i]
    }

    /// The sub-CPU wrote command nibble `i`; the last nibble (9) starts the drive working on the packet.
    pub fn write_tc(&mut self, i: usize, nibble: u8) {
        self.tc[i] = nibble & 0xF;
        if i == 9 {
            self.command_pending = true;
        }
    }

    /// The status packet is delivered now: the drive works on a pending command first and builds the answer.
    pub fn deliver(&mut self) {
        if !self.command_pending {
            return; // the same packet again until the host sends a command
        }
        self.command_pending = false;
        let rs = self.drive.command(&self.tc);
        self.rs[..9].copy_from_slice(&rs);
        self.rs[9] = checksum(&self.rs);
        // the report requests and no-ops the BIOS repeats all the time would fill the log
        if self.log.len() < 4000 && !matches!(self.tc[0], command::REPORT_REQUEST | command::NOP) {
            self.log.push((self.tc, self.rs));
        }
    }

    /// The commands received and the answers given so far (a debugging aid).
    pub fn log(&self) -> &[([u8; 10], [u8; 10])] {
        &self.log
    }

    /// Advances by `sub_clocks`; returns how many 1/75 s periods ended.
    pub fn advance(&mut self, sub_clocks: u64) -> u64 {
        self.acc += sub_clocks * PERIODS_PER_SECOND;
        let periods = self.acc / SUB_CLOCK_HZ;
        self.acc %= SUB_CLOCK_HZ;
        periods
    }

    /// One period of the drive itself (sector delivery), whether or not the host clock runs.
    pub fn drive_period(&mut self, cdc: &mut Cdc) {
        self.drive.period(cdc);
        self.drive.take_audio(&mut self.audio);
    }

    /// The CD audio played since the last call (interleaved left and right samples at 44.1 kHz).
    pub fn take_audio(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.audio)
    }
}
