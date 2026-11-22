//! The CD controller (Sanyo LC89510, an LC8951 with the Sega CD's glue) as the sub-CPU sees it through the gate array:
//! `$FF8004` selects a register and the data destination, `$FF8006` is the data port of the selected register, `$FF8008` the
//! host data port (facts E7-E13 in `docs/hardware-facts.md`).
//!
//! The chip takes the sectors the drive delivers, writes each block from its header on into a 16 KB buffer RAM, and tells the
//! sub-CPU with the decoder interrupt. The sub-CPU reads the header and the block pointer, points the data address at the
//! block, sets the byte count and starts a transfer, which goes to a 16-bit host port (main CPU or sub-CPU reads) or by DMA to
//! the PCM RAM, program RAM or word RAM. Error correction is not modelled: every block arrives intact (the disc images have
//! no errors), and the status registers say so.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// The buffer RAM: 16 KB, addressed by the 14-bit counters.
pub const BUFFER_SIZE: usize = 0x4000;

/// Where a transfer goes (the destination bits DD2-DD0 of `$FF8004`, fact C13).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Destination {
    /// Not a destination; transfers do nothing.
    None,
    /// The main CPU reads the host port.
    MainRead,
    /// The sub-CPU reads the host port.
    SubRead,
    /// DMA into the PCM chip's wave RAM.
    Pcm,
    /// DMA into program RAM.
    ProgramRam,
    /// DMA into word RAM (2M mode).
    WordRam,
}

impl Destination {
    fn from_bits(bits: u8) -> Self {
        match bits & 7 {
            2 => Destination::MainRead,
            3 => Destination::SubRead,
            4 => Destination::Pcm,
            5 => Destination::ProgramRam,
            7 => Destination::WordRam,
            _ => Destination::None,
        }
    }
}

/// A DMA transfer the gate array's owner carries out into memory it owns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dma {
    pub destination: Destination,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cdc {
    ar: u8,
    dest_bits: u8,
    ifctrl: u8,
    dbc: u16,
    dac: u16,
    wa: u16,
    pt: u16,
    ctrl0: u8,
    ctrl1: u8,
    sbout: u8,
    head: [u8; 4],
    ram: Box<[u8]>,
    /// The decoder interrupt flag (IFSTAT bit 5 reads zero while it is set).
    deci: bool,
    /// The data transfer end interrupt flag (IFSTAT bit 6 reads zero while it is set).
    dtei: bool,
    /// Bytes of a finished transfer the host port still has to give out.
    host: VecDeque<u8>,
    /// A transfer that ended is reported by EDT until the next one starts.
    edt: bool,
    dma: Option<Dma>,
    /// The INT output as the gate array last saw it, to find its rising edges.
    int_seen: bool,
}

impl Default for Cdc {
    fn default() -> Self {
        Self::new()
    }
}

impl Cdc {
    pub fn new() -> Self {
        Self {
            ar: 0,
            dest_bits: 0,
            ifctrl: 0,
            dbc: 0,
            dac: 0,
            wa: 0,
            pt: 0,
            ctrl0: 0,
            ctrl1: 0,
            sbout: 0,
            head: [0; 4],
            ram: vec![0; BUFFER_SIZE].into_boxed_slice(),
            deci: false,
            dtei: false,
            host: VecDeque::new(),
            edt: false,
            dma: None,
            int_seen: false,
        }
    }

    /// Whether a state's values are ones the controller can have (a state is untrusted input).
    pub fn validate(&self) -> Result<(), String> {
        if self.ram.len() != BUFFER_SIZE {
            return Err("the CD controller's buffer has the wrong size".to_string());
        }
        if self.ar > 0xF || self.dest_bits > 7 {
            return Err("the CD controller's address or destination is out of range".to_string());
        }
        if self.host.len() > BUFFER_SIZE {
            return Err("the CD controller holds more host bytes than its buffer".to_string());
        }
        if self
            .dma
            .as_ref()
            .is_some_and(|d| d.bytes.len() > BUFFER_SIZE)
        {
            return Err("the CD controller's transfer is longer than its buffer".to_string());
        }
        Ok(())
    }

    pub fn destination(&self) -> Destination {
        Destination::from_bits(self.dest_bits)
    }

    pub fn buffer(&self) -> &[u8] {
        &self.ram
    }

    /// Whether the decoder takes blocks into the buffer: DECEN and WRRQ both set (fact E11).
    pub fn writing(&self) -> bool {
        self.ctrl0 & 0x80 != 0 && self.ctrl0 & 0x04 != 0
    }

    /// `$FF8004` as read: EDT (bit 15), DSR (bit 14) and the destination (bits 10-8). UBR (bit 13) is not modelled.
    pub fn mode_word(&self) -> u16 {
        u16::from(self.edt) << 15
            | u16::from(!self.host.is_empty()) << 14
            | u16::from(self.dest_bits & 7) << 8
    }

    /// A word written to `$FF8004`: the destination in bits 10-8 and the register address in bits 3-0 (fact C13). Writing the
    /// destination resets the DMA circuit.
    pub fn write_mode(&mut self, v: u16) {
        self.dest_bits = ((v >> 8) & 7) as u8;
        self.ar = (v & 0xF) as u8;
        self.host.clear();
        self.edt = false;
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
        self.deci = false;
        self.dtei = false;
    }

    fn step_address(&mut self) {
        // the address register counts through R1-R15 and wraps to R0; an access to R0 does not move it (fact E7)
        if self.ar != 0 {
            self.ar = (self.ar + 1) & 0xF;
        }
    }

    fn ifstat(&self) -> u8 {
        // every bit reads one when idle; an interrupt, a transfer in progress and so on pull theirs to zero (fact E8)
        let mut v = 0xFFu8;
        if self.dtei {
            v &= !0x40;
        }
        if self.deci {
            v &= !0x20;
        }
        if !self.host.is_empty() {
            v &= !0x02; // data transfer in progress: the host has bytes to read
        }
        v
    }

    /// Reads the selected register and advances the address. `None`: the register is not answered.
    pub fn read(&mut self) -> Option<u8> {
        let value = match self.ar {
            0 => Some(0), // COMIN: no command bytes from the host
            1 => Some(self.ifstat()),
            2 => Some(self.dbc as u8),
            3 => Some(if self.dtei { 0x00 } else { 0xF0 } | ((self.dbc >> 8) & 0xF) as u8),
            4..=7 => Some(self.head[usize::from(self.ar - 4)]),
            8 => Some(self.pt as u8),
            9 => Some((self.pt >> 8) as u8),
            10 => Some(self.wa as u8),
            11 => Some((self.wa >> 8) as u8),
            // STAT0: CRC good, no error flags (fact E12)
            12 => Some(0x80),
            // STAT1: nothing corrected or flagged
            13 => Some(0x00),
            // STAT2: the mode and form bits of the block
            14 => Some(0x00),
            // STAT3: reading it clears the decoder interrupt; the status is valid
            15 => {
                self.deci = false;
                Some(0x00)
            }
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
            // DTTRG: starts a data transfer, when the data output is enabled (IFCTRL bit 1)
            6 => {
                if self.ifctrl & 0x02 != 0 {
                    self.start_transfer();
                }
            }
            // DTACK: acknowledges the end of a transfer
            7 => self.dtei = false,
            8 => self.wa = (self.wa & 0xFF00) | u16::from(v),
            9 => self.wa = (self.wa & 0x00FF) | (u16::from(v) << 8),
            10 => self.ctrl0 = v,
            11 => self.ctrl1 = v,
            12 => self.pt = (self.pt & 0xFF00) | u16::from(v),
            13 => self.pt = (self.pt & 0x00FF) | (u16::from(v) << 8),
            14 => {}
            15 => self.reset(),
            _ => {}
        }
        self.step_address();
    }

    /// Takes `DBC + 1` bytes from the buffer at the data address, to the destination (fact E13).
    fn start_transfer(&mut self) {
        let count = usize::from(self.dbc & 0xFFF) + 1;
        let dest = self.destination();
        let mut bytes = Vec::with_capacity(count);
        for i in 0..count {
            bytes.push(self.ram[(usize::from(self.dac) + i) & (BUFFER_SIZE - 1)]);
        }
        self.dac = self.dac.wrapping_add(count as u16);
        // the byte counter runs down past zero when the transfer is over
        self.dbc = 0xFFF;
        self.edt = false;
        match dest {
            Destination::MainRead | Destination::SubRead => self.host = bytes.into(),
            Destination::None => {}
            d => {
                self.dma = Some(Dma {
                    destination: d,
                    bytes,
                });
                self.edt = true;
                self.dtei = true;
            }
        }
    }

    /// The host port: the next two bytes of a transfer (the first in the high byte), when the destination matches `who`.
    pub fn host_word(&mut self, who: Destination) -> Option<u16> {
        if self.destination() != who || self.host.is_empty() {
            return None;
        }
        let hi = self.host.pop_front().unwrap_or(0);
        let lo = self.host.pop_front().unwrap_or(0);
        if self.host.is_empty() {
            self.edt = true;
            self.dtei = true;
        }
        Some(u16::from(hi) << 8 | u16::from(lo))
    }

    /// A DMA transfer waiting to be written into memory the gate array does not own.
    pub fn take_dma(&mut self) -> Option<Dma> {
        self.dma.take()
    }

    /// A block arrives from the drive: written from its header on into the buffer when the decoder is on (facts E11, E14).
    /// `raw` is the whole 2352-byte sector.
    pub fn deliver_sector(&mut self, raw: &[u8; 2352]) {
        if self.ctrl0 & 0x80 == 0 {
            return; // decoder off
        }
        self.head.copy_from_slice(&raw[12..16]);
        if self.ctrl0 & 0x04 != 0 {
            self.pt = self.wa;
            // the block is the 2340 bytes from the header on, then the 12 sync bytes of the next block
            for (i, b) in raw[12..].iter().enumerate() {
                self.ram[(usize::from(self.wa) + i) & (BUFFER_SIZE - 1)] = *b;
            }
            const SYNC: [u8; 12] = [
                0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0,
            ];
            for (i, b) in SYNC.iter().enumerate() {
                self.ram[(usize::from(self.wa) + 2340 + i) & (BUFFER_SIZE - 1)] = *b;
            }
            self.wa = ((usize::from(self.wa) + 2352) & (BUFFER_SIZE - 1)) as u16;
        }
        // the flag shows in IFSTAT whether or not the interrupt is enabled; the enable only gates the INT output
        self.deci = true;
    }

    /// The INT output: low (here `true`) while an enabled interrupt is set (fact E10).
    pub fn int_line(&self) -> bool {
        (self.deci && self.ifctrl & 0x20 != 0) || (self.dtei && self.ifctrl & 0x40 != 0)
    }

    /// `true` once for each rising edge of the INT output.
    pub fn int_edge(&mut self) -> bool {
        let now = self.int_line();
        let edge = now && !self.int_seen;
        self.int_seen = now;
        edge
    }

    pub fn address(&self) -> u8 {
        self.ar
    }

    pub fn control(&self) -> (u8, u8) {
        (self.ctrl0, self.ctrl1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_reg(c: &mut Cdc, reg: u8, v: u8) {
        c.write_address(reg);
        c.write(v);
    }

    fn read_reg(c: &mut Cdc, reg: u8) -> u8 {
        c.write_address(reg);
        c.read().unwrap()
    }

    /// A sector whose header is 00:02:07 mode 1 and whose user data counts up from `base`.
    fn sector(base: u8) -> [u8; 2352] {
        let mut raw = [0u8; 2352];
        raw[1..11].fill(0xFF);
        raw[12..16].copy_from_slice(&[0x00, 0x02, 0x07, 0x01]);
        for (i, b) in raw[16..2064].iter_mut().enumerate() {
            *b = base.wrapping_add(i as u8);
        }
        raw
    }

    #[test]
    fn the_address_register_steps_and_idle_status_reads_all_ones() {
        let mut c = Cdc::new();
        c.write_address(1);
        assert_eq!(c.read(), Some(0xFF), "IFSTAT idle");
        assert_eq!(c.address(), 2);
        c.write_address(15);
        c.write(0);
        assert_eq!(c.address(), 0, "R15 wraps to R0");
        c.write_address(0);
        c.read();
        assert_eq!(c.address(), 0, "R0 does not move it");
    }

    #[test]
    fn a_block_goes_to_the_buffer_from_its_header_and_the_pointers_follow() {
        let mut c = Cdc::new();
        c.deliver_sector(&sector(0)); // decoder off: nothing
        assert!(!c.int_line());
        write_reg(&mut c, 1, 0x20); // DECIEN
        write_reg(&mut c, 10, 0x84); // DECEN and WRRQ
        c.deliver_sector(&sector(0));
        assert_eq!(read_reg(&mut c, 4), 0x00, "HEAD0 minutes");
        assert_eq!(read_reg(&mut c, 5), 0x02);
        assert_eq!(read_reg(&mut c, 6), 0x07);
        assert_eq!(read_reg(&mut c, 7), 0x01, "HEAD3 mode");
        assert_eq!(
            &c.buffer()[..4],
            &[0, 2, 7, 1],
            "the buffer starts at the header"
        );
        assert_eq!(c.buffer()[4], 0, "then the data");
        assert_eq!(
            c.buffer()[2340],
            0,
            "the sync pattern of the next block follows"
        );
        assert_eq!(c.buffer()[2341], 0xFF);
        assert_eq!(read_reg(&mut c, 8), 0, "PT at the block");
        assert_eq!(read_reg(&mut c, 10), (2352 & 0xFF) as u8, "WA after it");
        assert_eq!(read_reg(&mut c, 11), (2352 >> 8) as u8);
        c.deliver_sector(&sector(1));
        assert_eq!(
            read_reg(&mut c, 8),
            (2352 & 0xFF) as u8,
            "PT moves to the second block"
        );
    }

    #[test]
    fn the_decoder_interrupt_needs_its_enable_and_stat3_clears_it() {
        let mut c = Cdc::new();
        write_reg(&mut c, 10, 0x84);
        c.deliver_sector(&sector(0));
        assert!(!c.int_line(), "masked");
        assert_eq!(
            read_reg(&mut c, 1) & 0x20,
            0x00,
            "but IFSTAT shows the flag"
        );
        write_reg(&mut c, 1, 0x20);
        assert!(c.int_line() && c.int_edge());
        assert!(!c.int_edge(), "one edge");
        assert_eq!(read_reg(&mut c, 15), 0x00, "STAT3: valid");
        assert!(!c.int_line());
        assert_eq!(read_reg(&mut c, 1) & 0x20, 0x20);
    }

    #[test]
    fn a_transfer_to_the_sub_cpu_gives_words_and_ends_with_the_interrupt() {
        let mut c = Cdc::new();
        write_reg(&mut c, 1, 0x40 | 0x02); // DTEIEN, DOUTEN
        write_reg(&mut c, 10, 0x84);
        c.deliver_sector(&sector(0));
        c.write_mode(0x0300); // sub read
        write_reg(&mut c, 4, 4); // DAC: the data after the header
        write_reg(&mut c, 5, 0);
        write_reg(&mut c, 2, 7); // 8 bytes
        write_reg(&mut c, 3, 0);
        write_reg(&mut c, 6, 0); // DTTRG
        assert_eq!(c.mode_word() & 0xC000, 0x4000, "data set ready, not ended");
        assert_eq!(read_reg(&mut c, 1) & 0x02, 0, "DTEN low while data waits");
        assert_eq!(
            c.host_word(Destination::MainRead),
            None,
            "not for the main CPU"
        );
        let words: Vec<u16> = (0..4)
            .map(|_| c.host_word(Destination::SubRead).unwrap())
            .collect();
        assert_eq!(words, [0x0001, 0x0203, 0x0405, 0x0607]);
        assert_eq!(c.mode_word() & 0xC000, 0x8000, "ended");
        assert!(c.int_line());
        write_reg(&mut c, 7, 0); // DTACK
        assert!(!c.int_line());
    }

    #[test]
    fn a_dma_transfer_hands_the_bytes_to_the_owner() {
        let mut c = Cdc::new();
        write_reg(&mut c, 1, 0x02);
        write_reg(&mut c, 10, 0x84);
        c.deliver_sector(&sector(0x10));
        c.write_mode(0x0500); // program RAM
        write_reg(&mut c, 4, 4);
        write_reg(&mut c, 5, 0);
        write_reg(&mut c, 2, 0xFF);
        write_reg(&mut c, 3, 0x07); // 2048 bytes
        write_reg(&mut c, 6, 0);
        let dma = c.take_dma().unwrap();
        assert_eq!(dma.destination, Destination::ProgramRam);
        assert_eq!(dma.bytes.len(), 2048);
        assert_eq!(dma.bytes[0], 0x10);
        assert_eq!(dma.bytes[2047], 0x10u8.wrapping_add(2047u16 as u8));
        assert_eq!(c.mode_word() & 0x8000, 0x8000);
        assert!(c.take_dma().is_none());
    }

    #[test]
    fn a_transfer_needs_the_data_output_enable_and_the_reset_register_clears_the_controls() {
        let mut c = Cdc::new();
        write_reg(&mut c, 10, 0x84);
        c.deliver_sector(&sector(0));
        c.write_mode(0x0500);
        write_reg(&mut c, 2, 3);
        write_reg(&mut c, 6, 0);
        assert!(c.take_dma().is_none(), "DOUTEN is off");
        write_reg(&mut c, 1, 0x02);
        write_reg(&mut c, 15, 0); // reset
        write_reg(&mut c, 6, 0);
        assert!(c.take_dma().is_none(), "reset cleared IFCTRL");
        assert_eq!(c.control(), (0, 0));
    }
}
