//! The CD controller (Sanyo LC89510, which is an LC8951 with the Sega CD's glue) as the sub-CPU sees it through the gate array:
//! `$FF8004` selects a register and the data destination, `$FF8006` is the data port of the selected register (facts E7-E10 in
//! `docs/hardware-facts.md`).
//!
//! Slice 3a keeps the register file: what the sub-CPU writes can be read back where the chip lets it, the interface status is
//! idle, and the registers that only mean something while a sector is being decoded (headers, status bytes, block pointer) are
//! not answered (`None`) until slice 3b brings the decoder.

/// A register of the chip, by its number R0-R15.
#[derive(Clone, Debug, Default)]
pub struct Cdc {
    /// The address register: which register `$FF8006` reads and writes. 4 bits.
    ar: u8,
    /// The data destination bits DD2-DD0 of `$FF8004` (fact C13).
    dest: u8,
    ifctrl: u8,
    dbc: u16,
    dac: u16,
    wa: u16,
    pt: u16,
    ctrl0: u8,
    ctrl1: u8,
    sbout: u8,
    r14: u8,
}

impl Cdc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn destination(&self) -> u8 {
        self.dest
    }

    /// `$FF8004` as read: end of transfer, data set ready and upper byte ready are 0 (no transfer), then DD2-DD0.
    pub fn mode_word(&self) -> u16 {
        u16::from(self.dest & 7) << 8
    }

    /// A word written to `$FF8004`: the destination in bits 10-8 and the register address in bits 3-0 (fact C13).
    pub fn write_mode(&mut self, v: u16) {
        self.dest = ((v >> 8) & 7) as u8;
        self.ar = (v & 0xF) as u8;
    }

    /// A byte written to the low half of `$FF8004`: the register address.
    pub fn write_address(&mut self, v: u8) {
        self.ar = v & 0xF;
    }

    /// The reset register: the command, interface, control 0 and control 1 registers go to zero (fact E9).
    fn reset(&mut self) {
        self.ifctrl = 0;
        self.ctrl0 = 0;
        self.ctrl1 = 0;
        self.sbout = 0;
    }

    fn step_address(&mut self) {
        // the address register counts through R1-R15 and wraps to R0; an access to R0 does not move it (fact E7)
        if self.ar != 0 {
            self.ar = (self.ar + 1) & 0xF;
        }
    }

    /// Reads the selected register and advances the address. `None`: the register means nothing before a sector is decoded.
    pub fn read(&mut self) -> Option<u8> {
        let value = match self.ar {
            0 => Some(0),    // COMIN: no command bytes from the host
            1 => Some(0xFF), // IFSTAT: nothing busy, no interrupt, bit 4 reads one (fact E8)
            2 => Some(self.dbc as u8),
            3 => Some(0xF0 | ((self.dbc >> 8) & 0xF) as u8),
            10 => Some(self.wa as u8),
            11 => Some((self.wa >> 8) as u8),
            _ => None,
        };
        self.step_address();
        value
    }

    /// Writes the selected register and advances the address.
    pub fn write(&mut self, v: u8) {
        match self.ar {
            0 => self.sbout = v,
            1 => self.ifctrl = v,
            2 => self.dbc = (self.dbc & 0xFF00) | u16::from(v),
            3 => self.dbc = (self.dbc & 0x00FF) | (u16::from(v & 0x0F) << 8),
            4 => self.dac = (self.dac & 0xFF00) | u16::from(v),
            5 => self.dac = (self.dac & 0x00FF) | (u16::from(v) << 8),
            6 | 7 => {} // DTTRG, DTACK: no transfer to start or acknowledge yet
            8 => self.wa = (self.wa & 0xFF00) | u16::from(v),
            9 => self.wa = (self.wa & 0x00FF) | (u16::from(v) << 8),
            10 => self.ctrl0 = v,
            11 => self.ctrl1 = v,
            12 => self.pt = (self.pt & 0xFF00) | u16::from(v),
            13 => self.pt = (self.pt & 0x00FF) | (u16::from(v) << 8),
            14 => self.r14 = v,
            15 => self.reset(),
            _ => {}
        }
        self.step_address();
    }

    pub fn address(&self) -> u8 {
        self.ar
    }

    pub fn control(&self) -> (u8, u8) {
        (self.ctrl0, self.ctrl1)
    }
}
