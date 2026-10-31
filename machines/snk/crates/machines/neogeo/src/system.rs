//! The two processors and the bus together, advanced in master clocks (24 MHz): the 68000 at master / 2, the Z80 at
//! master / 6 (fact N4), the Z80 run lazily up to the 68000's clock.

use crate::Bus;
use mc68000::Cpu;

const Z80_DIVIDER: u64 = 6;

pub struct NeoGeo {
    pub cpu: Cpu,
    pub z80: z80::Cpu,
    pub bus: Bus,
    /// Master clocks the Z80 has run up to.
    z80_clock: u64,
    nmi_was_high: bool,
}

impl NeoGeo {
    /// Resets the 68000 from the vector table the bus shows at `$000000`.
    pub fn new(mut bus: Bus) -> NeoGeo {
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        NeoGeo {
            cpu,
            z80: z80::Cpu::new(),
            bus,
            z80_clock: 0,
            nmi_was_high: false,
        }
    }

    /// Runs one 68000 instruction, then the Z80 up to the same time. Returns the master clocks that passed.
    pub fn step(&mut self) -> u64 {
        self.cpu.set_ipl(self.bus.ipl());
        let clocks = u64::from(self.cpu.step(&mut self.bus)) * 2;
        self.bus.advance(clocks);
        self.catch_up_z80();
        clocks
    }

    fn catch_up_z80(&mut self) {
        while self.z80_clock < self.bus.clock {
            // the NMI line is edge triggered
            let high = self.bus.sound.nmi_line();
            if high && !self.nmi_was_high {
                self.z80.request_nmi();
            }
            self.nmi_was_high = high;
            let t = self.z80.step(&mut self.bus);
            self.z80_clock += u64::from(t.max(1)) * Z80_DIVIDER;
        }
    }
}
