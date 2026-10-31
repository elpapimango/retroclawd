//! The sound side: the Z80's memory map and ports, the command and reply latches between the 68000 and the Z80, the
//! NMI, and the YM2610's register file (facts N24-N27). Nothing is played yet.

pub const M1_BANK_RAM: usize = 0x800;

/// The Z80 half of the Neo Geo: its program ROM (the BIOS's `sm1`, or the cartridge's M1), 2 KiB of RAM and the
/// four bank windows.
pub struct Sound {
    /// The BIOS's sound program (`sm1.sm1`).
    pub m1_board: Vec<u8>,
    /// The cartridge's sound program.
    pub m1_cart: Vec<u8>,
    /// True when the cartridge's M1 is selected (REG_CRTFIX), false for the board's (REG_BRDFIX) (fact N14).
    pub use_cart: bool,
    ram: Vec<u8>,
    /// The bank number of the windows at `$F000`, `$E000`, `$C000` and `$8000` (fact N24).
    banks: [u8; 4],
    /// The last command byte the 68000 wrote (read by Z80 port 0) and the Z80's reply (read by the 68000).
    pub command: u8,
    pub reply: u8,
    /// A command is waiting (cleared by the Z80's read of port 0 and by a write to it).
    pending: bool,
    nmi_enabled: bool,
    /// The YM2610's registers, the two address pairs, as the Z80 wrote them (fact N26).
    pub ym: [[u8; 256]; 2],
    ym_addr: [u8; 2],
}

impl Sound {
    pub fn new(m1_board: Vec<u8>, m1_cart: Vec<u8>) -> Sound {
        Sound {
            m1_board,
            m1_cart,
            use_cart: false,
            ram: vec![0; M1_BANK_RAM],
            banks: [0; 4],
            command: 0,
            reply: 0,
            pending: false,
            nmi_enabled: false,
            ym: [[0; 256]; 2],
            ym_addr: [0; 2],
        }
    }

    fn rom(&self) -> &[u8] {
        if self.use_cart && !self.m1_cart.is_empty() {
            &self.m1_cart
        } else {
            &self.m1_board
        }
    }

    fn rom_at(&self, at: usize) -> u8 {
        let rom = self.rom();
        if rom.is_empty() {
            0xFF
        } else {
            rom[at % rom.len()]
        }
    }

    /// The 68000 wrote a sound command (REG_SOUND): it is latched and, with NMIs enabled, interrupts the Z80 (fact N25).
    pub fn send_command(&mut self, v: u8) {
        self.command = v;
        self.pending = true;
    }

    /// The level of the NMI line: a command waits and NMIs are enabled (fact N25).
    pub fn nmi_line(&self) -> bool {
        self.pending && self.nmi_enabled
    }

    pub fn read(&self, addr: u16) -> u8 {
        let a = usize::from(addr);
        match a {
            0x0000..=0x7FFF => self.rom_at(a),
            0x8000..=0xBFFF => self.rom_at(usize::from(self.banks[3]) * 0x4000 + (a - 0x8000)),
            0xC000..=0xDFFF => self.rom_at(usize::from(self.banks[2]) * 0x2000 + (a - 0xC000)),
            0xE000..=0xEFFF => self.rom_at(usize::from(self.banks[1]) * 0x1000 + (a - 0xE000)),
            0xF000..=0xF7FF => self.rom_at(usize::from(self.banks[0]) * 0x800 + (a - 0xF000)),
            _ => self.ram[a - 0xF800],
        }
    }

    pub fn write(&mut self, addr: u16, v: u8) {
        if addr >= 0xF800 {
            self.ram[usize::from(addr) - 0xF800] = v;
        }
    }

    /// A read of a Z80 port. The port's high byte (register B of an `in r,(c)`) is the bank number for ports 8-B.
    pub fn read_io(&mut self, port: u16) -> u8 {
        match port & 0x0C {
            0x00 => {
                self.pending = false;
                self.command
            }
            0x04 => {
                // the YM2610's status (fact N26): nothing is busy and no timer has run out yet
                0
            }
            0x08 => {
                self.banks[usize::from(port & 3)] = (port >> 8) as u8;
                0
            }
            _ => 0xFF,
        }
    }

    pub fn write_io(&mut self, port: u16, v: u8) {
        match port & 0xFF {
            0x00 => self.pending = false,
            p @ 0x04..=0x07 => {
                let pair = usize::from((p >> 1) & 1);
                if p & 1 == 0 {
                    self.ym_addr[pair] = v;
                } else {
                    self.ym[pair][usize::from(self.ym_addr[pair])] = v;
                }
            }
            0x08..=0x0B => self.nmi_enabled = true,
            0x0C => self.reply = v,
            0x18 => self.nmi_enabled = false,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound() -> Sound {
        let mut m1 = vec![0u8; 0x1_0000];
        for (i, b) in m1.iter_mut().enumerate() {
            *b = (i / 0x800) as u8; // the 2 KiB page number
        }
        Sound::new(m1, Vec::new())
    }

    #[test]
    fn the_windows_show_the_banks_a_port_read_selected() {
        let mut s = sound();
        assert_eq!(s.read(0x0000), 0);
        assert_eq!(s.read(0x7FFF), 0x0F);
        // window 3 (16 KiB units): bank 1 starts at 0x4000, page 8
        s.read_io(0x0100 | 0x0B);
        assert_eq!(s.read(0x8000), 8);
        // window 0 (2 KiB units): bank 5 is page 5
        s.read_io(0x0500 | 0x08);
        assert_eq!(s.read(0xF000), 5);
        s.write(0xF800, 0x42);
        assert_eq!(s.read(0xF800), 0x42);
    }

    #[test]
    fn a_command_raises_the_nmi_only_while_enabled_and_port_zero_takes_it() {
        let mut s = sound();
        s.send_command(7);
        assert!(!s.nmi_line(), "NMIs are off after reset");
        s.write_io(0x08, 0);
        assert!(s.nmi_line());
        assert_eq!(s.read_io(0x00), 7);
        assert!(!s.nmi_line(), "reading port 0 acknowledges");
        s.send_command(8);
        s.write_io(0x18, 0);
        assert!(!s.nmi_line());
    }

    #[test]
    fn the_reply_port_and_the_ym_pairs_store_what_the_z80_writes() {
        let mut s = sound();
        s.write_io(0x0C, 0x55);
        assert_eq!(s.reply, 0x55);
        s.write_io(0x04, 0x27);
        s.write_io(0x05, 0x3F);
        s.write_io(0x06, 0x01);
        s.write_io(0x07, 0x02);
        assert_eq!(s.ym[0][0x27], 0x3F);
        assert_eq!(s.ym[1][0x01], 0x02);
    }
}
