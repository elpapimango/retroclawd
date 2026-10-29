//! The CD drive's microcontroller as the gate array's CDD port sees it: ten status nibbles in, ten command nibbles out, a
//! level 4 interrupt every 1/75 s while the host clock runs (facts E1-E6 in `docs/hardware-facts.md`).
//!
//! Slice 3a: the drive has no disc (or its tray is open); a disc comes with slice 3b.

/// Sub-CPU clocks per second (fact C10) and drive status periods per second (the CD frame rate).
const SUB_CLOCK_HZ: u64 = 12_500_000;
const PERIODS_PER_SECOND: u64 = 75;

/// Drive status nibbles (RS0), from the drive firmware's own list.
pub mod status {
    pub const STOP: u8 = 0x0;
    pub const DOOR_OPEN: u8 = 0x5;
    pub const NO_DISC: u8 = 0xB;
}

/// Drive command nibbles (TC1).
pub mod command {
    pub const NOP: u8 = 0x0;
    pub const STOP: u8 = 0x1;
    pub const REPORT_REQUEST: u8 = 0x2;
    pub const DOOR_CLOSE: u8 = 0xC;
    pub const DOOR_OPEN: u8 = 0xD;
}

#[derive(Clone, Debug)]
pub struct Cdd {
    /// Status nibbles RS0-RS9 as the sub-CPU reads them.
    rs: [u8; 10],
    /// Command nibbles TC0-TC9 as the sub-CPU wrote them.
    tc: [u8; 10],
    hock: bool,
    /// The host wrote the last command nibble since the drive last answered.
    command_pending: bool,
    drive_status: u8,
    /// Sub clocks times 75, not yet a whole period.
    acc: u64,
}

impl Default for Cdd {
    fn default() -> Self {
        Self::new()
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
            drive_status: status::NO_DISC,
            acc: 0,
        }
    }

    /// Rebuilds the status packet from the drive state.
    fn update_status(&mut self) {
        self.rs = [0; 10];
        self.rs[0] = self.drive_status;
        self.rs[9] = checksum(&self.rs);
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

    /// The drive works on the pending command and builds the answer.
    fn execute(&mut self) {
        if !self.command_pending {
            return; // the same packet again until the host sends a command
        }
        self.command_pending = false;
        match self.tc[1] {
            command::DOOR_OPEN => self.drive_status = status::DOOR_OPEN,
            command::DOOR_CLOSE => self.drive_status = status::NO_DISC,
            // with no disc every other command leaves the drive as it is
            command::NOP | command::STOP | command::REPORT_REQUEST => {}
            _ => {}
        }
        self.update_status();
    }

    /// The status packet is delivered now (the host clock is running and a period has come): the interrupt is due.
    pub fn deliver(&mut self) {
        self.execute();
    }

    /// Advances by `sub_clocks`; returns how many 1/75 s periods ended, to deliver while the host clock runs.
    pub fn advance(&mut self, sub_clocks: u64) -> u64 {
        self.acc += sub_clocks * PERIODS_PER_SECOND;
        let periods = self.acc / SUB_CLOCK_HZ;
        self.acc %= SUB_CLOCK_HZ;
        periods
    }
}
