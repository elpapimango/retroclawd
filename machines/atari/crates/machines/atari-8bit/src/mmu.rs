//! Stock Atari 800 / XL / XE memory routing; no CPU, PIA or ROM ownership.
//!
//! Hardware source: Avery Lee, Altirra Hardware Reference Manual, 2026-01-02,
//! <https://www.virtualdub.org/downloads/Altirra%20Hardware%20Reference%20Manual.pdf>,
//! sections 2.1, 2.5-2.7 and chapter 14, PORTB ($D301), p. 454.
//! The caller handles I/O and cartridges before consulting this decoder.
//! `$D000-$D7FF` always returns `Unmapped`, never shadow RAM.
//!
//! Supply PIA **driven pins**, not PORTB register readback (which may be DDR).
//! The PIA already owns output/DDR state, so the MMU must not serialize another
//! copy of those registers. `MmuState` is a separate, versioned profile extension;
//! it does not change the existing Atari 800 machine state layout.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    Atari800,
    Atari800Xl,
    Atari65Xe,
    Atari130Xe,
}

impl Profile {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Atari800 => "atari-800",
            Self::Atari800Xl => "atari-800xl",
            Self::Atari65Xe => "atari-65xe",
            Self::Atari130Xe => "atari-130xe",
        }
    }

    /// Physical RAM allocation, including RAM hidden by ROMs and I/O.
    /// Altirra 2.6-2.7: XL/XE expose 62K; stock 130XE has 64K + 64K.
    /// Atari800 retains this machine's established 48K configuration.
    pub const fn ram_size(self) -> usize {
        match self {
            Self::Atari800 => 48 * 1024,
            Self::Atari800Xl | Self::Atari65Xe => 64 * 1024,
            Self::Atari130Xe => 128 * 1024,
        }
    }

    /// Contiguous firmware image size, not the size of its visible windows.
    /// Altirra 2.1: 10K on 400/800; 16K on XL/XE.
    pub const fn os_size(self) -> usize {
        match self {
            Self::Atari800 => 10 * 1024,
            _ => 16 * 1024,
        }
    }

    pub const fn basic_size(self) -> usize {
        8 * 1024
    }

    pub const fn is_xl_xe(self) -> bool {
        !matches!(self, Self::Atari800)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Cpu,
    Antic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mapping {
    Ram(usize),
    Os(usize),
    Basic(usize),
    Unmapped,
}

/// Resolve the memory-control pins from ORB and DDRB, not PIA readback.
///
/// Altirra 2.6-2.7: memory-connected input pins are pulled high on stock
/// machines. Unused pin decay (2.5) is not modeled here and cannot affect
/// this decoder: XL/65XE ignore bits 2-6 and 130XE ignores bit 6.
pub const fn effective_portb(output: u8, direction: u8) -> u8 {
    output | !direction
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MmuState {
    pub version: u8,
    pub profile: Profile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mmu {
    profile: Profile,
}

impl Mmu {
    pub const fn new(profile: Profile) -> Self {
        Self { profile }
    }

    pub const fn profile(self) -> Profile {
        self.profile
    }

    pub const fn state(self) -> MmuState {
        MmuState {
            version: 1,
            profile: self.profile,
        }
    }

    /// Validate all memory lengths before the caller commits a machine restore.
    /// A missing optional BASIC image is allowed, but an incorrect one is not.
    pub fn validate_memory(
        self,
        ram_len: usize,
        os_len: usize,
        basic_len: Option<usize>,
    ) -> Result<(), &'static str> {
        if ram_len != self.profile.ram_size() {
            return Err("RAM size does not match Atari memory profile");
        }
        if os_len != self.profile.os_size() {
            return Err("OS ROM size does not match Atari memory profile");
        }
        if basic_len.is_some_and(|len| len != self.profile.basic_size()) {
            return Err("BASIC ROM must be 8192 bytes");
        }
        Ok(())
    }

    /// Construct a candidate without mutating the running machine.
    /// Profile identity is strict even between the equally sized XL and 65XE.
    pub fn from_state(
        state: &MmuState,
        expected_profile: Profile,
        ram_len: usize,
        os_len: usize,
        basic_len: Option<usize>,
    ) -> Result<Self, &'static str> {
        if state.version != 1 {
            return Err("unsupported Atari MMU state version");
        }
        if state.profile != expected_profile {
            return Err("Atari MMU state profile mismatch");
        }
        let candidate = Self::new(state.profile);
        candidate.validate_memory(ram_len, os_len, basic_len)?;
        Ok(candidate)
    }

    /// Decode reads using the current effective PORTB pins.
    ///
    /// On an 800, BASIC is an optional cartridge and PORTB does not bank ROM.
    /// On XL/XE, BASIC is a built-in ROM selected by bit 1 even if its image
    /// was not supplied. In that case the caller returns open bus for Basic;
    /// ROM selection still blocks writes, as it does on physical hardware.
    pub const fn map_read(
        self,
        address: u16,
        access: Access,
        portb: u8,
        basic_present: bool,
    ) -> Mapping {
        let addr = address as usize;
        if !self.profile.is_xl_xe() {
            return match address {
                0xa000..=0xbfff if basic_present => Mapping::Basic(addr - 0xa000),
                0x0000..=0xbfff => Mapping::Ram(addr),
                0xd800..=0xffff => Mapping::Os(addr - 0xd800),
                _ => Mapping::Unmapped,
            };
        }

        // Altirra 2.6: OS and self-test share the OS-enable condition.
        // Chapter 14 PORTB: self-test is the image's $D000-$D7FF portion.
        // It wins over extended RAM (2.7, "Self-test ROM conflict").
        if portb & 1 != 0 {
            match address {
                0x5000..=0x57ff if portb & 0x80 == 0 => {
                    return Mapping::Os(0x1000 + addr - 0x5000);
                }
                0xc000..=0xcfff | 0xd800..=0xffff => {
                    return Mapping::Os(addr - 0xc000);
                }
                _ => {}
            }
        }
        if (0xa000 <= address && address <= 0xbfff) && portb & 2 == 0 {
            return Mapping::Basic(addr - 0xa000);
        }
        if 0xd000 <= address && address <= 0xd7ff {
            return Mapping::Unmapped;
        }

        // Altirra 2.7 and chapter 14 PORTB: CPU bit 4 and ANTIC bit 5
        // independently select the same bank specified by bits 2-3.
        if matches!(self.profile, Profile::Atari130Xe) && 0x4000 <= address && address <= 0x7fff {
            let enable = match access {
                Access::Cpu => 0x10,
                Access::Antic => 0x20,
            };
            if portb & enable == 0 {
                return Mapping::Ram(
                    0x10000 + ((portb as usize >> 2) & 3) * 0x4000 + addr - 0x4000,
                );
            }
        }
        Mapping::Ram(addr)
    }

    /// Writable physical RAM index, or None for ROM, I/O and absent memory.
    /// Altirra 2.6 "Writes to ROM": no write-through into underlying RAM.
    pub const fn map_write(
        self,
        address: u16,
        access: Access,
        portb: u8,
        basic_present: bool,
    ) -> Option<usize> {
        match self.map_read(address, access, portb, basic_present) {
            Mapping::Ram(index) => Some(index),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILES: [Profile; 4] = [
        Profile::Atari800,
        Profile::Atari800Xl,
        Profile::Atari65Xe,
        Profile::Atari130Xe,
    ];

    #[test]
    fn atari800_keeps_its_original_map_for_every_portb_value() {
        let mmu = Mmu::new(Profile::Atari800);
        for portb in 0..=255 {
            for access in [Access::Cpu, Access::Antic] {
                assert_eq!(
                    mmu.map_read(0x5000, access, portb, true),
                    Mapping::Ram(0x5000)
                );
                assert_eq!(mmu.map_read(0xa000, access, portb, true), Mapping::Basic(0));
                assert_eq!(
                    mmu.map_read(0xa000, access, portb, false),
                    Mapping::Ram(0xa000)
                );
                assert_eq!(
                    mmu.map_read(0xbfff, access, portb, false),
                    Mapping::Ram(0xbfff)
                );
                assert_eq!(
                    mmu.map_read(0xc000, access, portb, false),
                    Mapping::Unmapped
                );
                assert_eq!(mmu.map_read(0xd800, access, portb, false), Mapping::Os(0));
                assert_eq!(
                    mmu.map_read(0xffff, access, portb, false),
                    Mapping::Os(10239)
                );
                assert_eq!(mmu.map_write(0xa000, access, portb, true), None);
            }
        }
    }

    #[test]
    fn acid800_xl_banking_masks_and_blocked_write_through() {
        // Acid800 1.2 source/mmu_xlbanking.s tests these PORTB values at
        // $C000, $F000, $A000, $5000, in that bit order (1 = writable).
        // The test's ROM complement writes must not alter hidden RAM.
        for profile in [Profile::Atari800Xl, Profile::Atari65Xe, Profile::Atari130Xe] {
            let mmu = Mmu::new(profile);
            for (portb, expected) in [
                (0xc2, 0x0f),
                (0xf3, 0x03),
                (0xf0, 0x0d),
                (0x72, 0x0f),
                (0x73, 0x02),
                (0x43, 0x02),
            ] {
                let mut mask = 0;
                for address in [0xc000, 0xf000, 0xa000, 0x5000] {
                    mask = (mask << 1)
                        | u8::from(mmu.map_write(address, Access::Cpu, portb, true).is_some());
                }
                assert_eq!(mask, expected, "{profile:?}, PORTB=${portb:02X}");
            }
            let mut ram = vec![0x35; profile.ram_size()];
            for address in [0xc000, 0xf000, 0xa000, 0x5000] {
                if let Some(index) = mmu.map_write(address, Access::Cpu, 0x71, true) {
                    ram[index] ^= 0xff;
                }
                assert_eq!(
                    mmu.map_read(address, Access::Cpu, 0xfe, true),
                    Mapping::Ram(address as usize)
                );
                assert_eq!(ram[address as usize], 0x35);
            }
        }
    }

    #[test]
    fn xe_cpu_and_antic_access_each_of_four_banks_independently() {
        let mmu = Mmu::new(Profile::Atari130Xe);
        let mut ram = vec![0; Profile::Atari130Xe.ram_size()];
        for bank in 0..4 {
            let select = bank << 2;
            let cpu_only = 0xe3 | select;
            let antic_only = 0xd3 | select;
            let both = 0xc3 | select;
            let neither = 0xf3 | select;
            let physical = 0x10000 + usize::from(bank) * 0x4000;
            let write = mmu.map_write(0x4000, Access::Cpu, cpu_only, true).unwrap();
            ram[write] = bank + 1;
            for (portb, cpu, antic) in [
                (cpu_only, physical, 0x4000),
                (antic_only, 0x4000, physical),
                (both, physical, physical),
                (neither, 0x4000, 0x4000),
            ] {
                assert_eq!(
                    mmu.map_read(0x4000, Access::Cpu, portb, true),
                    Mapping::Ram(cpu)
                );
                assert_eq!(
                    mmu.map_read(0x4000, Access::Antic, portb, true),
                    Mapping::Ram(antic)
                );
                assert_eq!(
                    mmu.map_read(0x7fff, Access::Antic, portb, true),
                    Mapping::Ram(antic + 0x3fff)
                );
                assert_eq!(
                    mmu.map_read(0x3fff, Access::Cpu, portb, true),
                    Mapping::Ram(0x3fff)
                );
                assert_eq!(
                    mmu.map_read(0x8000, Access::Antic, portb, true),
                    Mapping::Ram(0x8000)
                );
            }
        }
        for bank in 0..4 {
            assert_eq!(ram[0x10000 + bank * 0x4000], bank as u8 + 1);
        }
        assert_eq!(ram[0x4000], 0);
    }

    #[test]
    fn rom_boundaries_selftest_priority_and_permanent_io_hole() {
        for profile in [Profile::Atari800Xl, Profile::Atari65Xe, Profile::Atari130Xe] {
            let mmu = Mmu::new(profile);
            for access in [Access::Cpu, Access::Antic] {
                assert_eq!(mmu.map_read(0xc000, access, 0xff, true), Mapping::Os(0));
                assert_eq!(
                    mmu.map_read(0xcfff, access, 0xff, true),
                    Mapping::Os(0x0fff)
                );
                assert_eq!(
                    mmu.map_read(0xd800, access, 0xff, true),
                    Mapping::Os(0x1800)
                );
                assert_eq!(
                    mmu.map_read(0xffff, access, 0xff, true),
                    Mapping::Os(0x3fff)
                );
                assert_eq!(
                    mmu.map_read(0x5000, access, 0x43, true),
                    Mapping::Os(0x1000)
                );
                assert_eq!(
                    mmu.map_read(0x57ff, access, 0x43, true),
                    Mapping::Os(0x17ff)
                );
                assert_eq!(mmu.map_write(0x57ff, access, 0x43, true), None);
                assert_eq!(
                    mmu.map_read(0x5000, access, 0x72, true),
                    Mapping::Ram(0x5000)
                );
                assert_eq!(
                    mmu.map_read(0x4fff, access, 0x73, true),
                    Mapping::Ram(0x4fff)
                );
                assert_eq!(
                    mmu.map_read(0x5800, access, 0x73, true),
                    Mapping::Ram(0x5800)
                );
                // An omitted XL BASIC image does not create write-through RAM.
                assert_eq!(
                    mmu.map_read(0xbfff, access, 0xfd, false),
                    Mapping::Basic(0x1fff)
                );
                assert_eq!(mmu.map_write(0xbfff, access, 0xfd, false), None);
                for portb in 0..=255 {
                    for address in [0xd000, 0xd301, 0xd7ff] {
                        assert_eq!(
                            mmu.map_read(address, access, portb, true),
                            Mapping::Unmapped
                        );
                        assert_eq!(mmu.map_write(address, access, portb, true), None);
                    }
                }
            }
        }
    }

    #[test]
    fn ddr_changes_banking_even_without_a_new_output_latch_write() {
        let mmu = Mmu::new(Profile::Atari130Xe);
        let output = 0;
        let reset_pins = effective_portb(output, 0);
        assert_eq!(
            mmu.map_read(0xc000, Access::Cpu, reset_pins, true),
            Mapping::Os(0)
        );
        assert_eq!(
            mmu.map_read(0xa000, Access::Cpu, reset_pins, true),
            Mapping::Ram(0xa000)
        );
        assert_eq!(
            mmu.map_read(0x5000, Access::Antic, reset_pins, true),
            Mapping::Ram(0x5000)
        );
        let driven_pins = effective_portb(output, 0xff);
        assert_eq!(
            mmu.map_read(0xc000, Access::Cpu, driven_pins, true),
            Mapping::Ram(0xc000)
        );
        assert_eq!(
            mmu.map_read(0xa000, Access::Cpu, driven_pins, true),
            Mapping::Basic(0)
        );
        assert_eq!(
            mmu.map_read(0x4000, Access::Cpu, driven_pins, true),
            Mapping::Ram(0x10000)
        );
        let mixed_pins = effective_portb(output, 0x10);
        assert_eq!(
            mmu.map_read(0x4000, Access::Cpu, mixed_pins, true),
            Mapping::Ram(0x1c000)
        );
        assert_eq!(
            mmu.map_read(0x4000, Access::Antic, mixed_pins, true),
            Mapping::Ram(0x4000)
        );
        for profile in [Profile::Atari800Xl, Profile::Atari65Xe] {
            assert_eq!(
                Mmu::new(profile).map_read(0x4000, Access::Cpu, 0, true),
                Mapping::Ram(0x4000)
            );
        }
    }

    #[test]
    fn every_mapping_stays_inside_its_profile_memory() {
        for profile in PROFILES {
            let mmu = Mmu::new(profile);
            for portb in 0..=255 {
                for access in [Access::Cpu, Access::Antic] {
                    // Include both sides of every decoding boundary.
                    for address in [
                        0, 0x3fff, 0x4000, 0x4fff, 0x5000, 0x57ff, 0x5800, 0x7fff, 0x8000, 0x9fff,
                        0xa000, 0xbfff, 0xc000, 0xcfff, 0xd000, 0xd7ff, 0xd800, 0xffff,
                    ] {
                        match mmu.map_read(address, access, portb, true) {
                            Mapping::Ram(index) => assert!(index < profile.ram_size()),
                            Mapping::Os(index) => assert!(index < profile.os_size()),
                            Mapping::Basic(index) => assert!(index < profile.basic_size()),
                            Mapping::Unmapped => {}
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn separate_state_roundtrips_and_rejects_invalid_candidates() {
        for profile in PROFILES {
            let mmu = Mmu::new(profile);
            let bytes =
                bincode::serde::encode_to_vec(mmu.state(), bincode::config::standard()).unwrap();
            let (saved, used): (MmuState, usize) =
                bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
            assert_eq!(used, bytes.len());
            assert_eq!(
                Mmu::from_state(
                    &saved,
                    profile,
                    profile.ram_size(),
                    profile.os_size(),
                    Some(8192)
                ),
                Ok(mmu)
            );
            assert!(
                Mmu::from_state(
                    &saved,
                    profile,
                    profile.ram_size() - 1,
                    profile.os_size(),
                    None
                )
                .is_err()
            );
            assert!(
                Mmu::from_state(
                    &saved,
                    profile,
                    profile.ram_size(),
                    profile.os_size() + 1,
                    None
                )
                .is_err()
            );
            assert!(
                Mmu::from_state(
                    &saved,
                    profile,
                    profile.ram_size(),
                    profile.os_size(),
                    Some(8191)
                )
                .is_err()
            );
            let mut invalid = saved.clone();
            invalid.version = 2;
            assert!(
                Mmu::from_state(
                    &invalid,
                    profile,
                    profile.ram_size(),
                    profile.os_size(),
                    None
                )
                .is_err()
            );
            invalid = saved;
            invalid.profile = if profile == Profile::Atari800Xl {
                Profile::Atari65Xe
            } else {
                Profile::Atari800Xl
            };
            assert!(
                Mmu::from_state(
                    &invalid,
                    profile,
                    profile.ram_size(),
                    profile.os_size(),
                    None
                )
                .is_err()
            );
            assert_eq!(mmu.state().profile, profile);
        }
    }
}
