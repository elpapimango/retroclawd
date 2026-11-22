//! The I/O access log: every completed access to an I/O register, in the format of Hatari's `--trace io_all`
//! (`IO write.b $ffff8001 = $0a pc=fc0188`), so a run can be compared line by line with Hatari's. Register values
//! only; ROM and RAM contents never appear.

use mc68000::Size;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoAccess {
    /// The address of the instruction making the access.
    pub pc: u32,
    /// 24-bit address.
    pub addr: u32,
    pub size: Size,
    pub write: bool,
    pub value: u32,
}

impl IoAccess {
    /// The access as Hatari prints it: lower-case hex, the address sign-extended to 32 bits, the PC without leading
    /// zeros.
    pub fn hatari_line(&self) -> String {
        let (kind, value) = match self.size {
            Size::Byte => ("b", format!("{:02x}", self.value & 0xFF)),
            Size::Word => ("w", format!("{:04x}", self.value & 0xFFFF)),
        };
        format!(
            "IO {}.{kind} $ff{:06x} = ${value} pc={:x}",
            if self.write { "write" } else { "read" },
            self.addr & 0xFF_FFFF,
            self.pc & 0xFF_FFFF
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_hatari() {
        let a = IoAccess {
            pc: 0xFC0188,
            addr: 0xFF8001,
            size: Size::Byte,
            write: true,
            value: 0x0A,
        };
        assert_eq!(a.hatari_line(), "IO write.b $ffff8001 = $0a pc=fc0188");
        let b = IoAccess {
            pc: 0xFC66C6,
            addr: 0xFF8240,
            size: Size::Word,
            write: true,
            value: 0x0777,
        };
        assert_eq!(b.hatari_line(), "IO write.w $ffff8240 = $0777 pc=fc66c6");
        // A program in RAM: Hatari prints the PC without leading zeros.
        let c = IoAccess {
            pc: 0x1022,
            addr: 0xFF8240,
            size: Size::Word,
            write: true,
            value: 0x0123,
        };
        assert_eq!(c.hatari_line(), "IO write.w $ffff8240 = $0123 pc=1022");
    }
}
