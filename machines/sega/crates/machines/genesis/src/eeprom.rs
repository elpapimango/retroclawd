//! Serial EEPROM cartridges (facts H5-H9 in `docs/hardware-facts.md`): a 24Cxx chip on two lines, SCL and SDA, that the game
//! drives through a byte of the cartridge area. The chip follows the I2C protocol of the 24Cxx data sheets; which games have one
//! and how the lines reach the data bus depends on the board, so a table of known games (by serial number) picks the board.
//!
//! Probes cannot reach this part: no emulator offers a way to set the lines, so the behaviour is covered by tests that play
//! the data sheet's sequences against the chip, and by booting the games.

use serde::{Deserialize, Serialize};

/// The chips used and their array: address bytes, size, and the page a write wraps in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Chip {
    X24C01,
    X24C02,
    C24C02,
    C24C04,
    C24C08,
    C24C16,
    C24C65,
}

impl Chip {
    /// Bits of the word address the chip takes: 7 (a single byte with the R/W bit), 8, or 16 (two bytes).
    fn address_bits(self) -> u8 {
        match self {
            Chip::X24C01 => 7,
            Chip::C24C65 => 16,
            _ => 8,
        }
    }

    fn size(self) -> usize {
        match self {
            Chip::X24C01 => 128,
            Chip::X24C02 | Chip::C24C02 => 256,
            Chip::C24C04 => 512,
            Chip::C24C08 => 1024,
            Chip::C24C16 => 2048,
            Chip::C24C65 => 8192,
        }
    }

    fn page(self) -> usize {
        match self {
            Chip::X24C01 | Chip::X24C02 => 4,
            Chip::C24C02 => 8,
            Chip::C24C04 | Chip::C24C08 | Chip::C24C16 => 16,
            Chip::C24C65 => 64,
        }
    }
}

/// How a board connects the chip to the cartridge area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Board {
    /// Electronic Arts: SCL on D6, SDA on D7 in and out, the chip answers in all of `$200000-$3FFFFF`.
    Ea,
    /// Sega: SCL on D1, SDA on D0 in and D0 out, `$200000-$3FFFFF`.
    Sega,
    /// Acclaim, 2 MB games: any write sets SCL from D1 and SDA from D0, SDA is read on D1.
    Acclaim16,
    /// Acclaim, 4 MB games: an odd byte write sets SDA from D0, an even one SCL from D0, a word write chooses with D0 whether
    /// `$200000-$2FFFFF` shows the chip (0) or the ROM (1); the ROM shows again after a reset.
    Acclaim32,
    /// Codemasters: writes anywhere in `$300000-$37FFFF` set SCL from D1 and SDA from D0; SDA is read on D7 in
    /// `$380000-$3FFFFF`, where a J-Cart's pads share the area.
    Codemasters,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
enum State {
    Standby,
    /// A read ended with no acknowledge: nothing happens until a stop.
    WaitStop,
    /// The first byte after a start (the device address, or for the one-byte chip the word address).
    Device,
    WordHigh,
    WordLow,
    Write,
    Read,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Eeprom {
    chip: Chip,
    board: Board,
    pub data: Vec<u8>,
    sda: bool,
    scl: bool,
    old_sda: bool,
    old_scl: bool,
    /// The array address the next byte is read from or written to.
    addr: usize,
    /// Clock edges seen in the byte: 0 after a start, 1-8 the data bits, 9 the acknowledge.
    cycles: u8,
    state: State,
    read: bool,
    device: u16,
    word: u16,
    buffer: u8,
    /// The chip answers reads of `$200000-$2FFFFF` (Acclaim 4 MB boards switch it with the ROM).
    pub visible: bool,
    pub dirty: bool,
}

/// The board and chip a game uses, by the serial number of its header (and, for the Codemasters ROMs that carry none, never).
pub fn lookup(serial: &str) -> Option<(Board, Chip)> {
    const GAMES: &[(&str, Board, Chip)] = &[
        ("T-50176", Board::Ea, Chip::X24C01),
        ("T-50396", Board::Ea, Chip::X24C01),
        ("T-50446", Board::Ea, Chip::X24C01),
        ("T-50516", Board::Ea, Chip::X24C01),
        ("T-50606", Board::Ea, Chip::X24C01),
        ("MK-1215", Board::Sega, Chip::X24C01),
        ("MK-1228", Board::Sega, Chip::X24C01),
        ("G-5538", Board::Sega, Chip::X24C01),
        ("PR-1993", Board::Sega, Chip::X24C01),
        ("00001211", Board::Sega, Chip::X24C01),
        ("00004076", Board::Sega, Chip::X24C01),
        ("G-4524", Board::Sega, Chip::X24C01),
        ("00054503", Board::Sega, Chip::X24C01),
        ("T-81033", Board::Acclaim16, Chip::X24C02),
        ("T-081326", Board::Acclaim16, Chip::X24C02),
        ("T-081276", Board::Acclaim32, Chip::C24C02),
        ("T-81406", Board::Acclaim32, Chip::C24C04),
        ("T-081586", Board::Acclaim32, Chip::C24C16),
        ("T-81476", Board::Acclaim32, Chip::C24C65),
        ("T-81576", Board::Acclaim32, Chip::C24C65),
    ];
    GAMES
        .iter()
        .find(|(id, _, _)| serial.contains(id))
        .map(|&(_, b, c)| (b, c))
}

/// The Codemasters boards: some games carry no serial number, so the first long of the vectors and the header checksum name
/// them. `(has a J-Cart, the chip if any)`; the Brian Lara Cricket games have the chip but no J-Cart.
pub fn lookup_codemasters(
    serial: &str,
    first_long: u32,
    checksum: u16,
) -> Option<(bool, Option<Chip>)> {
    const BY_SERIAL: &[(&str, bool, Option<Chip>)] = &[
        ("T-120106", false, Some(Chip::C24C08)),
        ("T-120096", true, Some(Chip::C24C16)),
        ("T-120146", false, Some(Chip::C24C65)),
        ("T-120066", true, None),
        ("T-123456", true, None),
    ];
    if let Some(&(_, jcart, chip)) = BY_SERIAL.iter().find(|(id, _, _)| serial.contains(id)) {
        return Some((jcart, chip));
    }
    match (first_long, checksum) {
        (0x444E_4C44, 0x168B) => Some((true, Some(Chip::C24C08))),
        (0x444E_4C44, 0x165E) => Some((true, Some(Chip::C24C16))),
        (0xFFFF_FFFC, 0x168B | 0x165E) | (_, 0xDF39)
            if serial.trim().is_empty() || serial.contains("XXXXXXXX") =>
        {
            Some((true, None))
        }
        _ => None,
    }
}

impl Eeprom {
    pub fn new(board: Board, chip: Chip) -> Self {
        Self {
            chip,
            board,
            data: vec![0xFF; chip.size()],
            sda: true,
            scl: true,
            old_sda: true,
            old_scl: true,
            addr: 0,
            cycles: 0,
            state: State::Standby,
            read: false,
            device: 0,
            word: 0,
            buffer: 0,
            visible: board != Board::Acclaim32,
            dirty: false,
        }
    }

    pub fn board(&self) -> Board {
        self.board
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.data.len() != self.chip.size() || self.cycles > 9 {
            return Err("the EEPROM state is out of range".to_string());
        }
        Ok(())
    }

    /// The bus returns to power-on wiring (the Acclaim 4 MB board shows its ROM again).
    pub fn reset(&mut self) {
        if self.board == Board::Acclaim32 {
            self.visible = false;
        }
    }

    /// The line levels the board lets the game write: `(scl bit, sda bit)` of the data byte.
    fn bits(&self) -> (u8, u8) {
        match self.board {
            Board::Ea => (6, 7),
            Board::Sega | Board::Acclaim16 | Board::Codemasters => (1, 0),
            Board::Acclaim32 => (0, 0),
        }
    }

    /// The bit position the board shows SDA at when the game reads.
    fn out_bit(&self) -> u8 {
        match self.board {
            Board::Ea | Board::Codemasters => 7,
            Board::Sega | Board::Acclaim32 => 0,
            Board::Acclaim16 => 1,
        }
    }

    /// A word written to the chip's area (`D15-D0`, only the low byte's bits matter).
    pub fn write_word(&mut self, addr: u32, value: u16) {
        match self.board {
            Board::Acclaim32 => {
                // both byte strobes: the data bit chooses the ROM or the chip for reads
                self.visible = value & 1 == 0;
            }
            Board::Acclaim16 | Board::Codemasters => self.drive(value),
            Board::Ea | Board::Sega => {
                let _ = addr;
                self.drive(value);
            }
        }
    }

    /// A byte written: the boards with byte strobes only take the odd (low) byte; Acclaim's 4 MB board takes both, one per line.
    pub fn write_byte(&mut self, addr: u32, value: u8) {
        match self.board {
            Board::Ea | Board::Sega => {
                if addr & 1 != 0 {
                    self.drive(u16::from(value));
                }
            }
            Board::Acclaim16 | Board::Codemasters => self.drive(u16::from(value)),
            Board::Acclaim32 => {
                if addr & 1 != 0 {
                    self.sda = value & 1 != 0;
                } else {
                    self.scl = value & 1 != 0;
                }
                self.update();
            }
        }
    }

    fn drive(&mut self, value: u16) {
        let (scl, sda) = self.bits();
        self.sda = (value >> sda) & 1 != 0;
        self.scl = (value >> scl) & 1 != 0;
        self.update();
    }

    /// The level of SDA the chip drives (or the line's own level when it lets go), at the board's bit.
    fn sda_out(&self) -> bool {
        if self.state == State::Read {
            if self.cycles < 9 {
                let bit = 8 - u32::from(self.cycles.max(1));
                return (self.data[self.addr] >> bit) & 1 != 0;
            }
        } else if self.cycles == 9 {
            return false; // acknowledge
        }
        self.sda
    }

    /// A word read from the chip's area.
    pub fn read_word(&self) -> u16 {
        u16::from(self.sda_out()) << self.out_bit()
    }

    /// A byte read: only the odd (low) byte carries the data bit; `None` for the other byte (the open bus).
    pub fn read_byte(&self, addr: u32) -> Option<u8> {
        (addr & 1 != 0).then(|| u8::from(self.sda_out()) << self.out_bit())
    }

    fn start_condition(&mut self, old_sda: bool, old_scl: bool) {
        if old_scl && self.scl && old_sda && !self.sda {
            self.cycles = 0;
            if self.chip.address_bits() == 7 {
                self.word = 0;
            } else {
                self.device = 0;
            }
            self.state = State::Device;
        }
    }

    fn stop_condition(&mut self, old_sda: bool, old_scl: bool) {
        if old_scl && self.scl && !old_sda && self.sda {
            self.state = State::Standby;
        }
    }

    fn update(&mut self) {
        let (old_sda, old_scl) = (self.old_sda, self.old_scl);
        let falling = old_scl && !self.scl;
        let rising = !old_scl && self.scl;
        match self.state {
            State::Standby => self.start_condition(old_sda, old_scl),
            State::WaitStop => self.stop_condition(old_sda, old_scl),
            _ => {
                self.start_condition(old_sda, old_scl);
                self.stop_condition(old_sda, old_scl);
                if falling {
                    if self.cycles < 9 {
                        self.cycles += 1;
                    } else {
                        self.byte_done();
                    }
                } else if rising {
                    self.bit_clocked();
                }
            }
        }
        self.old_sda = self.sda;
        self.old_scl = self.scl;
    }

    /// The byte (and its acknowledge) is over.
    fn byte_done(&mut self) {
        self.cycles = 1;
        match self.state {
            State::Device => {
                if self.chip.address_bits() == 7 {
                    self.addr = usize::from(self.word) & (self.chip.size() - 1);
                    self.state = if self.read { State::Read } else { State::Write };
                    self.buffer = 0;
                } else if self.read {
                    self.state = State::Read;
                } else {
                    self.word = 0;
                    self.state = if self.chip.address_bits() == 16 {
                        State::WordHigh
                    } else {
                        State::WordLow
                    };
                }
            }
            State::WordHigh => self.state = State::WordLow,
            State::WordLow => {
                self.addr = (usize::from(self.device) << 8 | usize::from(self.word))
                    & (self.chip.size() - 1);
                self.state = State::Write;
                self.buffer = 0;
            }
            _ => {}
        }
    }

    /// SCL went high: the data bit on SDA is taken (or, at the acknowledge clock, the host's answer).
    fn bit_clocked(&mut self) {
        let c = self.cycles;
        let sda = u16::from(self.sda);
        match self.state {
            State::Device if self.chip.address_bits() == 7 => {
                if c < 8 {
                    self.word |= sda << (7 - c);
                } else if c == 8 {
                    self.read = self.sda;
                }
            }
            State::Device => {
                // 1010 A2 A1 A0 R/W: the three address bits come in at clocks 5-7
                if c > 4 && c < 8 {
                    self.device |= sda << (7 - c);
                } else if c == 8 {
                    self.read = self.sda;
                }
                if c == 8 {
                    // the array bits among them are the chip's high address bits
                    self.device &= if self.chip.address_bits() == 16 {
                        0
                    } else {
                        ((self.chip.size() >> 8).max(1) - 1) as u16
                    };
                }
            }
            State::WordHigh => {
                if c < 9 {
                    self.word |= sda << (16 - c);
                }
            }
            State::WordLow => {
                if c < 9 {
                    let bits = if self.chip.address_bits() == 7 { 7 } else { 8 };
                    if c <= bits {
                        self.word |= sda << (8 - c);
                    }
                }
            }
            State::Read => {
                if c == 9 {
                    if self.sda {
                        self.state = State::WaitStop;
                    } else {
                        self.addr = (self.addr + 1) & (self.chip.size() - 1);
                    }
                }
            }
            State::Write => {
                if c < 9 {
                    self.buffer |= (self.sda as u8) << (8 - c);
                } else {
                    if self.data[self.addr] != self.buffer {
                        self.data[self.addr] = self.buffer;
                        self.dirty = true;
                    }
                    self.buffer = 0;
                    let page = self.chip.page();
                    self.addr = (self.addr & !(page - 1)) | ((self.addr + 1) & (page - 1));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host wired like the Sega board: SCL on D1, SDA on D0, through odd byte writes.
    struct Host(Eeprom);

    impl Host {
        fn new(chip: Chip) -> Self {
            Host(Eeprom::new(Board::Sega, chip))
        }

        fn lines(&mut self, scl: bool, sda: bool) {
            self.0.write_byte(1, u8::from(scl) << 1 | u8::from(sda));
        }

        fn start(&mut self) {
            self.lines(true, true);
            self.lines(true, false);
            self.lines(false, false);
        }

        fn stop(&mut self) {
            self.lines(false, false);
            self.lines(true, false);
            self.lines(true, true);
        }

        /// Clocks one bit out and returns what the chip drives on SDA while SCL is high.
        fn bit(&mut self, sda: bool) -> bool {
            self.lines(false, sda);
            self.lines(true, sda);
            let seen = self.0.read_word() & 1 != 0;
            self.lines(false, sda);
            seen
        }

        /// Sends a byte and returns whether the chip acknowledged it.
        fn send(&mut self, byte: u8) -> bool {
            for i in (0..8).rev() {
                self.bit(byte >> i & 1 != 0);
            }
            !self.bit(true)
        }

        /// Reads a byte, then acknowledges (`more`) or not.
        fn receive(&mut self, more: bool) -> u8 {
            let mut v = 0;
            for _ in 0..8 {
                v = v << 1 | u8::from(self.bit(true));
            }
            self.bit(!more);
            v
        }
    }

    #[test]
    fn a_byte_written_to_the_one_byte_chip_reads_back() {
        let mut h = Host::new(Chip::X24C01);
        h.start();
        assert!(h.send(0x05 << 1), "address 5, write");
        assert!(h.send(0x5A), "the data is acknowledged");
        h.stop();
        assert_eq!(h.0.data[5], 0x5A);
        assert!(h.0.dirty);
        h.start();
        assert!(h.send(0x05 << 1 | 1), "address 5, read");
        assert_eq!(h.receive(false), 0x5A);
        h.stop();
    }

    #[test]
    fn the_two_wire_chips_take_a_device_byte_a_word_address_and_data() {
        let mut h = Host::new(Chip::C24C02);
        h.start();
        assert!(h.send(0xA0));
        assert!(h.send(0x10));
        assert!(h.send(0x11));
        assert!(h.send(0x22));
        h.stop();
        assert_eq!((h.0.data[0x10], h.0.data[0x11]), (0x11, 0x22));
        // a random read: a dummy write sets the address, then a read after a repeated start
        h.start();
        assert!(h.send(0xA0));
        assert!(h.send(0x10));
        h.start();
        assert!(h.send(0xA1));
        assert_eq!(h.receive(true), 0x11);
        assert_eq!(h.receive(false), 0x22);
        h.stop();
    }

    #[test]
    fn a_write_wraps_inside_its_page() {
        let mut h = Host::new(Chip::C24C02);
        h.start();
        h.send(0xA0);
        h.send(0x06); // page of 8: $06, $07, then back to $00
        for v in [1, 2, 3] {
            h.send(v);
        }
        h.stop();
        assert_eq!((h.0.data[6], h.0.data[7], h.0.data[0]), (1, 2, 3));
    }

    #[test]
    fn the_four_kilobit_chip_takes_its_ninth_address_bit_from_the_device_byte() {
        let mut h = Host::new(Chip::C24C04);
        h.start();
        h.send(0xA2); // page bit 1
        h.send(0x20);
        h.send(0x77);
        h.stop();
        assert_eq!(h.0.data[0x120], 0x77);
    }

    #[test]
    fn the_sixteen_bit_chip_takes_two_address_bytes() {
        let mut h = Host::new(Chip::C24C65);
        h.start();
        h.send(0xA0);
        h.send(0x12);
        h.send(0x34);
        h.send(0x99);
        h.stop();
        assert_eq!(h.0.data[0x1234], 0x99);
    }

    #[test]
    fn the_acclaim_boards_wire_the_lines_their_own_way() {
        // 4 MB board: even bytes drive SCL, odd bytes SDA; a word write chooses between chip and ROM
        let mut e = Eeprom::new(Board::Acclaim32, Chip::C24C02);
        assert!(!e.visible);
        e.write_word(0x20_0000, 0);
        assert!(e.visible, "D0 low shows the chip");
        e.write_word(0x20_0000, 1);
        assert!(!e.visible, "D0 high shows the ROM");
        e.reset();
        assert!(!e.visible);
        // 2 MB board: a start condition, written as whole words with SCL on D1 and SDA on D0
        let mut e = Eeprom::new(Board::Acclaim16, Chip::X24C02);
        for w in [3u16, 2, 0] {
            e.write_word(0x20_0000, w);
        }
        assert_eq!(e.state, State::Device, "SDA fell while SCL was high");
        assert_eq!(
            e.read_word(),
            0x0002 & u16::from(e.sda) << 1,
            "SDA is read on D1"
        );
    }

    #[test]
    fn known_games_pick_their_board() {
        assert_eq!(
            lookup("GM T-081326 -00"),
            Some((Board::Acclaim16, Chip::X24C02))
        );
        assert_eq!(lookup("GM T-50446  -00"), Some((Board::Ea, Chip::X24C01)));
        assert_eq!(lookup("GM 00001009-00"), None);
    }
}
