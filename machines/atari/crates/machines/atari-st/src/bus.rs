//! The ST memory map as the 68000 sees it.
//!
//! | Range | Contents | Source |
//! |---|---|---|
//! | `$000000-$000007` | the first 8 bytes of the ROM (reset SSP and PC) | TOS documentation 3.5 "The system vectors": "a mirror image of the first eight bytes of the ROM" (https://freemint.github.io/tos.hyp/en/the_system_vectors.html) |
//! | `$000008-` RAM top | RAM | |
//! | `$FA0000-$FBFFFF` | cartridge (128 KB) | Atari Compendium chapter 5 "Cartridges" (http://bus-error.nokturnal.pl/atari_compendium/html/chap5.htm) |
//! | `$FC0000-$FEFFFF` | 192 KB TOS ROM | ST Internals memory map "Start of ROM OS (for versions below 1.4)"; EmuTOS 1.4 192 KB image header (reset PC `$FC0030`) |
//! | `$FF8000-$FFFFFF` | I/O | ST Internals register list (https://atarimuseum.ctrl-alt-rees.com/computers/16bits/STINTERN.htm) |
//!
//! Bus errors (TOS documentation 3.5, bus error vector): in user mode an access to the hardware registers or below
//! `$800`; in supervisor mode an access to nonexistent memory.
//!
//! Unspecified by the sources read so far, chosen deterministically (spec section 9) and to be checked against the
//! Atari hardware documentation: RAM between the configured top and `$3FFFFF` reads `$FF` per byte and ignores
//! writes, without a bus error; writes to `$0-$7` and to the ROM are ignored; an empty cartridge port reads `$FF`;
//! `$400000-$F9FFFF` and `$FF0000-$FF7FFF` are nonexistent (bus error).

use crate::acia::{Acias, NullPeer, SerialPeer};
use crate::config::Config;
use crate::dma::{Dma, FDC_BYTE_CYCLES};
use crate::io::{self, Decode, Device};
use crate::iolog::IoAccess;
use crate::mfp_clock::MfpClock;
use crate::mmu::Mmu;
use crate::psg::Psg;
use crate::scheduler;
use crate::serial::FileSink;
use mc68000::{BusFault, FunctionCode, IackResponse, M68kBus, Size};
use mc68901::Mfp;
use st_shifter::{Monitor, Shifter};

/// Interrupt levels (TOS documentation, "The system vectors": `$68` level 2 HBL, `$70` level 4 VBL, `$78` level 6
/// MFP).
pub const HBL_LEVEL: u8 = 2;
pub const VBL_LEVEL: u8 = 4;
pub const MFP_LEVEL: u8 = 6;

pub const ROM_BASE: u32 = 0xFC_0000;
/// TOS 1.0x and the EmuTOS 192 KB images.
pub const ROM_SIZE: usize = 196_608;
const RAM_DECODE_END: u32 = 0x40_0000;
const CARTRIDGE: std::ops::Range<u32> = 0xFA_0000..0xFC_0000;
/// [H] *Internals* p.96: "Up to 128K in the address space $FA0000 to $FBFFFF can be addressed."
pub const CARTRIDGE_BYTES: usize = 128 * 1024;
const IO_BASE: u32 = 0xFF_8000;
const SUPERVISOR_ONLY_BELOW: u32 = 0x800;

/// Why the machine stopped at an I/O access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The device is known but not emulated yet.
    NotEmulated(Device),
    /// No hardware fact covers the access (see `io.rs`).
    NoFact,
}

/// The first I/O access the machine cannot answer: the machine stops there (spec section 16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoStop {
    pub reason: StopReason,
    pub addr: u32,
    pub size: Size,
    pub write: bool,
    pub value: u32,
    pub fc: FunctionCode,
    pub at_cycle: u64,
}

/// PSG port A bit 5: the Centronics strobe, active low (*Internals* p.59).
const PRINTER_STROBE: u8 = 0x20;

/// The device on the far end of the MFP's serial line.
struct Rs232(Box<dyn SerialPeer + Send>);

impl std::fmt::Debug for Rs232 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Rs232")
    }
}

#[derive(Debug)]
pub struct StBus {
    ram: Vec<u8>,
    mmu: Mmu,
    rom: Vec<u8>,
    stop: Option<IoStop>,
    /// Emulated time in CPU clocks. `fast`: the start of the running instruction; `accurate`: the cycle being run.
    pub(crate) now: u64,
    pub(crate) sched: scheduler::Scheduler,
    pub(crate) region: crate::config::Region,
    pub(crate) vbl_count: u64,
    pub(crate) last_vbl_at: u64,
    pub(crate) hbl_count: u64,
    /// This frame's shape (chosen at its start) and the lines done in it.
    pub(crate) geometry: scheduler::FrameGeometry,
    pub(crate) line: u32,
    /// The number of the one MFP timeout event that is current.
    pub(crate) mfp_event: u64,
    /// The picture drawn at the last VBL (640 x 400 RGB).
    pub(crate) frame: Vec<u8>,
    /// `--accuracy accurate`: time passes with every bus cycle.
    pub(crate) accurate: bool,
    /// `borders = on`: the picture is drawn with its borders (`st_shifter::OVERSCAN_WIDTH` x `OVERSCAN_HEIGHT`).
    pub(crate) borders: bool,
    /// The GLUE's interrupt latches, cleared by their acknowledge cycles.
    hbl_pending: bool,
    vbl_pending: bool,
    pub(crate) mfp: Mfp,
    pub(crate) shifter: Shifter,
    pub(crate) psg: Psg,
    pub(crate) acias: Acias,
    /// What the MFP's RS-232 line is connected to (nothing, unless the host attached something).
    rs232: Rs232,
    printer: Option<FileSink>,
    /// The ROM cartridge in the slot, at most [`CARTRIDGE_BYTES`] bytes.
    cartridge: Option<Vec<u8>>,
    pub(crate) dma: Dma,
    /// A DMA/FDC register was accessed: the controller's events may have moved.
    pub(crate) fdc_dirty: bool,
    /// The FDC's INTRQ as GPIP I5 last saw it.
    fdc_irq: bool,
    /// An ACIA register was accessed: its service events may have moved.
    pub(crate) acia_dirty: bool,
    pub(crate) monitor: Monitor,
    clock: MfpClock,
    /// MFP ticks the MFP has been advanced by.
    mfp_ticks: u64,
    /// An MFP register was written: its next timeout may have moved.
    pub(crate) mfp_dirty: bool,
    /// The address of the instruction running, for the I/O log.
    pub(crate) pc: u32,
    io_log: Vec<IoAccess>,
    io_log_limit: u32,
}

/// The bus and every device behind it as data: what a save state holds of the machine's hardware (not the TOS image,
/// the I/O log, the stop record or the configuration).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BusState {
    ram: Vec<u8>,
    mmu: Mmu,
    hbl_pending: bool,
    vbl_pending: bool,
    mfp: Mfp,
    shifter: Shifter,
    psg: crate::psg::PsgState,
    acias: [crate::acia::PortState; 2],
    dma: crate::dma::DmaState,
    fdc_dirty: bool,
    fdc_irq: bool,
    acia_dirty: bool,
    clock: MfpClock,
    mfp_ticks: u64,
    mfp_dirty: bool,
    pc: u32,
}

/// Where an address falls.
#[derive(Debug)]
pub(crate) enum Region {
    Ram(usize),
    UnfittedRam,
    Rom(usize),
    /// The offset into the 128 KB cartridge window.
    Cartridge(usize),
    Io,
    None,
}

/// The image must start the CPU inside itself: the reset PC (long 1 of the image, read through the `$0-$7` mirror)
/// even and within `$FC0000-$FEFFFF`. A TOS image starts with `BRA.S` (`$602E` in TOS 1.0x and EmuTOS 1.4); seen as
/// `$2E60`, the image is byte-swapped, a common result of reading the two ROM chips separately.
fn check_reset_pc(rom: &[u8]) -> Result<(), String> {
    if rom[0..2] == [0x2E, 0x60] {
        return Err("this TOS image looks byte-swapped (it starts 2E 60 instead of 60 2E): swap each pair of bytes".into());
    }
    let pc = u32::from_be_bytes([rom[4], rom[5], rom[6], rom[7]]) & 0xFF_FFFF;
    if pc & 1 != 0 {
        return Err(format!(
            "the image's reset PC ${pc:06X} is odd: not a TOS image for the ST"
        ));
    }
    if !(ROM_BASE..ROM_BASE + ROM_SIZE as u32).contains(&pc) {
        return Err(format!(
            "the image's reset PC ${pc:06X} is outside the ROM at $FC0000-$FEFFFF: not a 192 KB TOS image for the ST"
        ));
    }
    Ok(())
}

impl StBus {
    pub fn new(cfg: &Config) -> Result<Self, String> {
        if cfg.rom.len() != ROM_SIZE {
            return Err(format!(
                "the ST takes a {ROM_SIZE}-byte (192 KB) TOS image at $FC0000; this one has {} bytes{}",
                cfg.rom.len(),
                if cfg.rom.len() == 262_144 {
                    " (262144 bytes is a 256 KB image, for the STE, which maps it at $E00000)"
                } else {
                    ""
                }
            ));
        }
        check_reset_pc(&cfg.rom)?;
        let mmu = Mmu::new(cfg.ram);
        let cpu_hz = (scheduler::crystal_hz(cfg.region) / scheduler::CPU_DIVIDER) as u64;
        let mut acias = Acias::new();
        if let Some(k) = acias.ikbd() {
            k.set_cycles_per_second(cpu_hz);
        }
        let mut mfp = Mfp::new();
        for (line, level) in gpip_inputs(cfg.monitor) {
            mfp.set_input(line, level);
        }
        let shifter = Shifter::new(cfg.region == crate::config::Region::Pal);
        let geometry = scheduler::geometry(shifter.sync_50hz(), cfg.monitor)?;
        let mut bus = Self {
            ram: vec![0; mmu.ram_bytes()],
            mmu,
            rom: cfg.rom.clone(),
            stop: None,
            now: 0,
            sched: scheduler::Scheduler::new(),
            region: cfg.region,
            vbl_count: 0,
            last_vbl_at: 0,
            hbl_count: 0,
            geometry,
            line: 0,
            mfp_event: 0,
            frame: vec![0; (st_shifter::WIDTH * st_shifter::HEIGHT * 3) as usize],
            accurate: cfg.accuracy == crate::config::Accuracy::Accurate,
            borders: false,
            hbl_pending: false,
            vbl_pending: false,
            mfp,
            shifter,
            acias,
            rs232: Rs232(Box::new(NullPeer)),
            printer: None,
            cartridge: None,
            dma: Dma::new(2),
            fdc_dirty: false,
            fdc_irq: false,
            acia_dirty: false,
            psg: Psg::new(cpu_hz),
            monitor: cfg.monitor,
            clock: MfpClock::new(scheduler::crystal_hz(cfg.region)),
            mfp_ticks: 0,
            mfp_dirty: false,
            pc: 0,
            io_log: Vec::new(),
            io_log_limit: cfg.io_log_limit,
        };
        // The first line ends one line time after power-on.
        bus.sched.schedule(
            bus.geometry.clocks_per_line as u64,
            scheduler::CLASS_VIDEO,
            scheduler::Event::LineEnd,
        );
        Ok(bus)
    }

    /// The SHA-1 of the TOS image: what a save state records, so it is only loaded on the same firmware.
    pub fn firmware_sha1(&self) -> [u8; 20] {
        crate::state::sha1_of(&self.rom)
    }

    /// The hardware as data; `None` while the machine is stopped (a stopped machine has nothing to resume).
    pub(crate) fn snapshot(&self) -> Option<BusState> {
        if self.stop.is_some() {
            return None;
        }
        Some(BusState {
            ram: self.ram.clone(),
            mmu: self.mmu.clone(),
            hbl_pending: self.hbl_pending,
            vbl_pending: self.vbl_pending,
            mfp: self.mfp.clone(),
            shifter: self.shifter.clone(),
            psg: self.psg.snapshot(),
            acias: [
                self.acias.ports[0].snapshot(),
                self.acias.ports[1].snapshot(),
            ],
            dma: self.dma.snapshot(),
            fdc_dirty: self.fdc_dirty,
            fdc_irq: self.fdc_irq,
            acia_dirty: self.acia_dirty,
            clock: self.clock,
            mfp_ticks: self.mfp_ticks,
            mfp_dirty: self.mfp_dirty,
            pc: self.pc,
        })
    }

    /// Takes the state in, after checking it fits this machine (the same RAM, the same memory configuration shape).
    /// On an error nothing has changed.
    pub(crate) fn restore(&mut self, s: BusState) -> Result<(), String> {
        if s.ram.len() != self.ram.len() {
            return Err(format!(
                "the state was made with {} bytes of RAM, this machine has {}",
                s.ram.len(),
                self.ram.len()
            ));
        }
        if s.mmu.ram_bytes() != self.mmu.ram_bytes() {
            return Err("the state's memory configuration does not fit this machine's RAM".into());
        }
        // Everything that can fail goes first: the DMA state is the one with a check (and it merges with the disks
        // now in the drives, so it works on this bus's own DMA chip).
        self.dma.restore(s.dma)?;
        self.ram = s.ram;
        self.mmu = s.mmu;
        self.hbl_pending = s.hbl_pending;
        self.vbl_pending = s.vbl_pending;
        self.mfp = s.mfp;
        self.shifter = s.shifter;
        self.psg.restore(s.psg);
        let [kbd, midi] = s.acias;
        self.acias.ports[0].restore(kbd);
        self.acias.ports[1].restore(midi);
        self.fdc_dirty = s.fdc_dirty;
        self.fdc_irq = s.fdc_irq;
        self.acia_dirty = s.acia_dirty;
        self.clock = s.clock;
        self.mfp_ticks = s.mfp_ticks;
        self.mfp_dirty = s.mfp_dirty;
        self.pc = s.pc;
        self.stop = None;
        Ok(())
    }

    /// The interrupt level the GLUE and the MFP ask for: the highest one pending.
    pub fn ipl(&self) -> u8 {
        if self.mfp.irq() {
            MFP_LEVEL
        } else if self.vbl_pending {
            VBL_LEVEL
        } else if self.hbl_pending {
            HBL_LEVEL
        } else {
            0
        }
    }

    pub fn raise_hbl(&mut self) {
        self.hbl_pending = true;
    }

    pub fn raise_vbl(&mut self) {
        self.vbl_pending = true;
    }

    pub fn mfp_mut(&mut self) -> &mut Mfp {
        &mut self.mfp
    }

    pub fn mfp(&self) -> &Mfp {
        &self.mfp
    }

    /// The memory configuration register (`$FF8001`).
    pub fn mmu_config(&self) -> u8 {
        self.mmu.read_config()
    }

    pub fn shifter(&self) -> &Shifter {
        &self.shifter
    }

    pub fn psg(&self) -> &Psg {
        &self.psg
    }

    /// ACIA `n`: 0 keyboard, 1 MIDI.
    pub fn acia(&self, n: usize) -> &mc6850::Acia {
        &self.acias.ports[n].chip
    }

    /// The DMA chip's address counter.
    pub fn dma_address(&self) -> u32 {
        self.dma.address
    }

    pub fn dma(&self) -> &Dma {
        &self.dma
    }

    /// Advances the floppy controller to CPU cycle `target`, a byte time at a time while it works, moving each byte
    /// it offers through the DMA; then GPIP I5 follows its INTRQ.
    pub(crate) fn catch_up_fdc(&mut self, target: u64) {
        while self.dma.fdc_at < target {
            let gap = target - self.dma.fdc_at;
            let step = if self.dma.busy() {
                gap.min(FDC_BYTE_CYCLES)
            } else {
                gap.min(u32::MAX as u64)
            };
            self.dma.fdc.tick(step as u32);
            self.dma.fdc_at += step;
            if let Some(burst) = self.dma.service_drq() {
                self.dma_store(&burst, self.dma.fdc_at);
            }
            if self.dma.needs_burst() {
                let burst = self.dma_fetch(Dma::BURST_BYTES, self.dma.fdc_at);
                self.dma.load_burst(burst);
            }
            self.dma.service_write_drq();
        }
        if !self.dma.command_running() && self.dma.fdc.floppy.has_pending_track() {
            self.dma.fdc.floppy.commit_pending();
        }
        self.update_fdc_irq(target);
    }

    /// `n` bytes from memory at the DMA address for the FIFO (a write to disk): RAM, ROM and the cartridge are read, the
    /// I/O area or nonexistent memory has no fact and stops the machine.
    fn dma_fetch(&mut self, n: usize, at: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let addr = self.dma.address & 0xFF_FFFF;
            match self.region(addr) {
                Region::Io | Region::None => {
                    if self.stop.is_none() {
                        self.stop = Some(IoStop {
                            reason: StopReason::NoFact,
                            addr,
                            size: Size::Byte,
                            write: false,
                            value: 0,
                            fc: FunctionCode::SUPERVISOR_DATA,
                            at_cycle: at,
                        });
                    }
                    out.push(0);
                }
                _ => out.push(self.peek(addr)),
            }
            self.dma.address = (self.dma.address + 1) & 0xFF_FFFF;
        }
        out
    }

    /// A FIFO burst to memory at the DMA address: RAM takes it, ROM and the cartridge ignore it ([H?] as CPU writes),
    /// the I/O area or nonexistent memory has no fact and stops the machine.
    fn dma_store(&mut self, bytes: &[u8], at: u64) {
        for &b in bytes {
            let addr = self.dma.address & 0xFF_FFFF;
            match self.region(addr) {
                Region::Ram(i) if self.stop.is_none() => self.ram[i] = b,
                Region::Ram(_) | Region::Rom(_) | Region::Cartridge(_) | Region::UnfittedRam => {}
                Region::Io | Region::None => {
                    if self.stop.is_none() {
                        self.stop = Some(IoStop {
                            reason: StopReason::NoFact,
                            addr,
                            size: Size::Byte,
                            write: true,
                            value: b as u32,
                            fc: FunctionCode::SUPERVISOR_DATA,
                            at_cycle: at,
                        });
                    }
                }
            }
            self.dma.address = (self.dma.address + 1) & 0xFF_FFFF;
        }
    }

    /// GPIP I5 follows the FDC's INTRQ (active low), at CPU cycle `at`.
    /// Runs a complete ACSI command and moves its data through the DMA (the whole transfer at once: [H?] the
    /// document gives no timing). A transfer that would leave RAM is refused with the DMA error bit, and moves
    /// nothing.
    fn run_acsi(&mut self, target: u8, cdb: &[u8]) {
        use crate::acsi::Transfer;
        let outcome = self.dma.acsi.execute(target, cdb);
        let mut status = outcome.status;
        match outcome.transfer {
            Transfer::None => {}
            Transfer::ToHost(data) => {
                let n = data.len().min(self.dma.dma_bytes_left());
                match self.dma_ram_range(n) {
                    Some(places) => {
                        for (place, byte) in places.into_iter().zip(&data) {
                            self.ram[place] = *byte;
                        }
                        self.dma.acsi_moved(n);
                    }
                    None => self.dma.set_error(),
                }
            }
            Transfer::FromHost { target, lba, bytes } => {
                let n = bytes.min(self.dma.dma_bytes_left());
                match self.dma_ram_range(n) {
                    Some(places) if n == bytes => {
                        let data: Vec<u8> = places.into_iter().map(|p| self.ram[p]).collect();
                        self.dma.acsi_moved(n);
                        status = self.dma.acsi.finish_write(target, lba, &data);
                    }
                    Some(_) => {
                        // The sector count is short of the blocks the command asked for: nothing is written.
                        self.dma.set_error();
                    }
                    None => self.dma.set_error(),
                }
            }
        }
        self.dma.acsi.complete(status);
    }

    /// The RAM offsets of the `n` bytes from the DMA address, or `None` when any of them is outside installed RAM.
    fn dma_ram_range(&self, n: usize) -> Option<Vec<usize>> {
        let start = self.dma.address & 0xFF_FFFF;
        (0..n as u32)
            .map(|k| {
                let a = start + k;
                (a < RAM_DECODE_END)
                    .then(|| self.mmu.translate(a))
                    .flatten()
            })
            .collect()
    }

    fn update_fdc_irq(&mut self, at: u64) {
        let irq = self.dma.fdc.intrq() || self.dma.acsi.irq();
        if irq != self.fdc_irq {
            self.fdc_irq = irq;
            self.catch_up_mfp(at);
            self.mfp.set_input(5, !irq);
            self.mfp_dirty = true;
        }
    }

    /// The drive selects and side follow PSG port A.
    fn update_drive_select(&mut self) {
        let port_a = self.psg.port_a();
        self.dma.fdc.floppy.set_port_a(port_a);
    }

    /// GPIP I4 follows the ACIAs' shared IRQ line, at CPU cycle `at`.
    pub(crate) fn update_acia_irq(&mut self, at: u64) {
        self.catch_up_mfp(at);
        self.mfp.set_input(4, self.acias.gpip_level());
        self.mfp_dirty = true;
    }

    /// MFP ticks so far.
    pub fn mfp_ticks(&self) -> u64 {
        self.mfp_ticks
    }

    pub(crate) fn mfp_clock(&self) -> MfpClock {
        self.clock
    }

    /// Advances the MFP to CPU clock `cycle` (never backwards).
    pub(crate) fn catch_up_mfp(&mut self, cycle: u64) {
        let target = self.clock.ticks_at(cycle);
        if target > self.mfp_ticks {
            self.mfp.tick(target - self.mfp_ticks);
            self.mfp_ticks = target;
        }
        self.pump_rs232(cycle);
    }

    /// Moves the characters the USART has finished to the RS-232 peer and, when the receiver is free, the peer's
    /// next one into the receiver.
    fn pump_rs232(&mut self, at: u64) {
        while let Some(byte) = self.mfp.usart_take_tx() {
            self.rs232.0.receive(byte, at);
        }
        if self.mfp.usart_can_receive()
            && let Some((_, byte)) = self.rs232.0.next_byte(at)
        {
            self.mfp.usart_rx_begin(byte);
        }
    }

    /// The Centronics strobe (*Internals* p.59 and pp.88-89: PSG port A bit 5, a low pulse; the data lines are port
    /// B). [H?] the byte is taken when the strobe line falls between two driven states of port A: a port A that
    /// only now becomes an output does not strobe the printer.
    fn printer_strobe(&mut self, before: Option<u8>) {
        let (Some(was), Some(now)) = (before, self.psg.port_a_driven()) else {
            return;
        };
        if was & PRINTER_STROBE != 0
            && now & PRINTER_STROBE == 0
            && let Some(printer) = &mut self.printer
            && let Some(data) = self.psg.port_b_driven()
        {
            printer.write(data);
        }
    }

    /// Puts a ROM cartridge in the slot, or takes it out (`None`). The image is the user's file: any length up to
    /// 128 KB is accepted, and TOS decides whether it is a cartridge it runs (*Internals* ROM listing, p.273: the `$ABCDEF42`
    /// application header it scans for at `$FA0000`).
    pub fn set_cartridge(&mut self, image: Option<Vec<u8>>) -> Result<(), String> {
        if let Some(bytes) = &image {
            if bytes.is_empty() {
                return Err("the cartridge image is empty".to_string());
            }
            if bytes.len() > CARTRIDGE_BYTES {
                return Err(format!(
                    "a cartridge is at most {CARTRIDGE_BYTES} bytes (128 KB); this image is {}",
                    bytes.len()
                ));
            }
        }
        self.cartridge = image;
        Ok(())
    }

    pub fn cartridge(&self) -> Option<&[u8]> {
        self.cartridge.as_deref()
    }

    /// Connects a printer: its output file takes the bytes strobed out, and BUSY (GPIP I0) reads low. Without a
    /// printer BUSY stays high, as a printer that is off line.
    pub fn attach_printer(&mut self, sink: FileSink) {
        self.printer = Some(sink);
        self.mfp.set_input(0, false);
        self.mfp_dirty = true;
    }

    /// Connects `peer` to the MFP's serial line.
    pub fn attach_rs232_peer(&mut self, peer: Box<dyn SerialPeer + Send>) {
        self.rs232 = Rs232(peer);
    }

    /// The completed I/O accesses so far (at most `io_log_limit`).
    pub fn io_log(&self) -> &[IoAccess] {
        &self.io_log
    }

    /// The logged accesses, removed (the limit then applies to the next ones).
    pub fn take_io_log(&mut self) -> Vec<IoAccess> {
        std::mem::take(&mut self.io_log)
    }

    fn log_io(&mut self, addr: u32, size: Size, write: bool, value: u32) {
        if (self.io_log.len() as u32) < self.io_log_limit {
            self.io_log.push(IoAccess {
                pc: self.pc,
                addr,
                size,
                write,
                value,
            });
        }
    }

    /// The first unimplemented I/O access, if there was one.
    pub fn stop(&self) -> Option<&IoStop> {
        self.stop.as_ref()
    }

    pub(crate) fn region(&self, addr: u32) -> Region {
        let addr = addr & 0xFF_FFFF;
        if addr < 8 {
            Region::Rom(addr as usize)
        } else if addr < RAM_DECODE_END {
            match self.mmu.translate(addr) {
                Some(i) => Region::Ram(i),
                None => Region::UnfittedRam,
            }
        } else if CARTRIDGE.contains(&addr) {
            Region::Cartridge((addr - CARTRIDGE.start) as usize)
        } else if (ROM_BASE..ROM_BASE + ROM_SIZE as u32).contains(&addr) {
            Region::Rom((addr - ROM_BASE) as usize)
        } else if addr >= IO_BASE {
            Region::Io
        } else {
            Region::None
        }
    }

    /// A byte as the debugger sees it: no device side effects, I/O reads `$FF`.
    pub fn peek(&self, addr: u32) -> u8 {
        match self.region(addr) {
            Region::Ram(i) => self.ram[i],
            Region::Rom(i) => self.rom[i],
            Region::Cartridge(i) => self.cartridge_byte(i),
            _ => 0xFF,
        }
    }

    /// Stores a byte for the debugger and program loading: RAM only.
    pub fn poke(&mut self, addr: u32, value: u8) -> bool {
        match self.region(addr) {
            Region::Ram(i) => {
                self.ram[i] = value;
                true
            }
            _ => false,
        }
    }

    /// One I/O access through the decode table. `Err` is a bus error; a stop answers `$FF` per byte.
    fn io(
        &mut self,
        addr: u32,
        size: Size,
        write: bool,
        value: u32,
        fc: FunctionCode,
    ) -> Result<u32, BusFault> {
        let ff = match size {
            Size::Byte => 0xFF,
            Size::Word => 0xFFFF,
        };
        // After a stop the rest of the stopped instruction runs on invented values: it must reach no device and
        // no log line.
        if self.stop.is_some() {
            return Ok(ff);
        }
        let reason = match io::decode(addr, size) {
            Decode::BusError => return Err(BusFault),
            Decode::Device(Device::Mfp) => {
                self.catch_up_mfp(self.now);
                // Internals pp.60-61: register n at $FFFA01 + 2n.
                let reg = ((addr - 0xFF_FA01) / 2) as u8;
                let v = if write {
                    self.mfp.write(reg, value as u8);
                    self.mfp_dirty = true;
                    // The timer control registers (A, B, C and D): the prescaler starts at this clock.
                    if self.accurate && matches!(reg, 12..=14) {
                        self.clock.restart_at(self.now, self.mfp_ticks);
                    }
                    value
                } else {
                    self.mfp.read(reg) as u32
                };
                self.log_io(addr, size, write, v);
                return Ok(v);
            }
            Decode::Device(Device::Shifter) => {
                let word = size == Size::Word;
                let v = if write {
                    self.shifter.write_at(addr, word, value as u16, self.now);
                    value
                } else if matches!(addr, 0xFF_8205 | 0xFF_8207 | 0xFF_8209) {
                    let counter = self.video_counter();
                    (counter >> (8 * ((0xFF_8209 - addr) / 2))) & 0xFF
                } else {
                    self.shifter.read(addr, word).unwrap_or(0xFFFF) as u32
                };
                self.log_io(addr, size, write, v);
                return Ok(v);
            }
            Decode::Device(Device::Psg) => {
                self.psg.catch_up(self.now);
                let v = if write {
                    self.catch_up_fdc(self.now);
                    let before = self.psg.port_a_driven();
                    self.psg.write(addr, value as u8);
                    self.update_drive_select();
                    self.printer_strobe(before);
                    value
                } else {
                    self.psg.read(addr) as u32
                };
                self.log_io(addr, size, write, v);
                return Ok(v);
            }
            Decode::Device(Device::Dma)
                if !(addr == 0xFF_8604
                    && !write
                    && self.dma.hdc_selected()
                    && !self.dma.acsi.has_status()) =>
            {
                let now = self.now;
                self.catch_up_fdc(now);
                let v = match (addr, write) {
                    // With the HDC selected a read is the completion status of the last ACSI command.
                    (0xFF_8604, false) if self.dma.hdc_selected() => {
                        0xFF00 | self.dma.acsi.take_status().unwrap_or(0xFF) as u32
                    }
                    (0xFF_8604, false) => self.dma.read_data() as u32,
                    (0xFF_8604, true) if self.dma.acsi_command_mode() => {
                        let first = self.dma.acsi_first_byte();
                        if let Some((target, cdb)) = self.dma.acsi.command_byte(first, value as u8)
                        {
                            self.run_acsi(target, &cdb);
                        }
                        value
                    }
                    (0xFF_8604, true) => {
                        self.dma.write_data(value as u16);
                        value
                    }
                    (0xFF_8606, false) => self.dma.status() as u32,
                    (0xFF_8606, true) => {
                        self.dma.write_mode(value as u16);
                        value
                    }
                    // $FF8609, $FF860B, $FF860D: address high, mid, low.
                    (a, false) => self.dma.address_byte((a - 0xFF_8609) / 2) as u32,
                    (a, true) => {
                        self.dma.set_address_byte((a - 0xFF_8609) / 2, value as u8);
                        value
                    }
                };
                self.fdc_dirty = true;
                self.update_fdc_irq(now);
                self.log_io(addr, size, write, v);
                return Ok(v);
            }
            // [H?] an HDC (ACSI) read with nothing connected.
            Decode::Device(Device::Dma) => StopReason::NoFact,
            Decode::Device(Device::Acia) => {
                // Internals p.62: control/status at $FFFC00 and $FFFC04, data at $FFFC02 and $FFFC06.
                let port = ((addr - 0xFF_FC00) / 4) as usize;
                let rs = addr & 2 != 0;
                let now = self.now;
                let v = if write {
                    self.acias.ports[port].write(rs, value as u8, now);
                    value
                } else {
                    self.acias.ports[port].read(rs, now) as u32
                };
                self.acia_dirty = true;
                self.update_acia_irq(now);
                self.log_io(addr, size, write, v);
                return Ok(v);
            }
            Decode::Device(Device::Mmu) => {
                let v = if write {
                    self.mmu.write_config(value as u8);
                    value
                } else {
                    self.mmu.read_config() as u32
                };
                self.log_io(addr, size, write, v);
                return Ok(v);
            }
            Decode::Open => {
                self.log_io(addr, size, write, if write { value } else { ff });
                return Ok(ff);
            }
            Decode::NoFact => StopReason::NoFact,
        };
        if self.stop.is_none() {
            // The stopping access is logged only when it is a write: a read's value would be invented.
            if write {
                self.log_io(addr, size, write, value);
            }
            self.stop = Some(IoStop {
                reason,
                addr,
                size,
                write,
                value,
                fc,
                at_cycle: self.now,
            });
        }
        Ok(ff)
    }

    /// A byte of the cartridge window: the image, then [H?] `$FF` past its end or with no cartridge (an open bus).
    fn cartridge_byte(&self, i: usize) -> u8 {
        self.cartridge
            .as_ref()
            .and_then(|c| c.get(i))
            .copied()
            .unwrap_or(0xFF)
    }

    fn byte(&self, region: &Region) -> u8 {
        match *region {
            Region::Ram(i) => self.ram[i],
            Region::Rom(i) => self.rom[i],
            Region::Cartridge(i) => self.cartridge_byte(i),
            _ => 0xFF,
        }
    }

    fn user_refused(addr: u32, fc: FunctionCode) -> bool {
        let addr = addr & 0xFF_FFFF;
        !fc.is_supervisor() && !(SUPERVISOR_ONLY_BELOW..IO_BASE).contains(&addr)
    }
}

impl M68kBus for StBus {
    fn read(&mut self, addr: u32, size: Size, fc: FunctionCode) -> Result<u32, BusFault> {
        if !self.accurate {
            return self.read_cycle(addr, size, fc);
        }
        let wait = self.wait_clocks(addr, size);
        let start = self.begin_cycle(wait, false);
        let result = self.read_cycle(addr, size, fc);
        self.end_cycle(start, 0);
        result
    }

    fn write(
        &mut self,
        addr: u32,
        size: Size,
        value: u32,
        fc: FunctionCode,
    ) -> Result<(), BusFault> {
        if !self.accurate {
            return self.write_cycle(addr, size, value, fc);
        }
        let wait = self.wait_clocks(addr, size);
        let start = self.begin_cycle(wait, true);
        let result = self.write_cycle(addr, size, value, fc);
        self.end_cycle(start, 0);
        result
    }

    fn sync(&mut self, internal_clocks: u32) {
        if self.accurate {
            self.pass(internal_clocks);
        }
    }

    /// HBL and VBL are autovectored (VPA); the MFP puts its vector on the bus, and with nothing to give the cycle
    /// ends as spurious.
    fn iack(&mut self, level: u8) -> IackResponse {
        if self.accurate {
            let wait = self.iack_wait_clocks();
            self.pass(wait);
            self.run_events(self.now);
        }
        self.iack_cycle(level)
    }

    /// The RESET instruction: [D] the MFP's RESET input (MC68901 §2.2.3). Other devices keep their state until a
    /// source says the line reaches them.
    fn reset_devices(&mut self) {
        self.catch_up_mfp(self.now);
        self.mfp.reset();
        self.mfp_dirty = true;
    }
}

impl StBus {
    fn read_cycle(&mut self, addr: u32, size: Size, fc: FunctionCode) -> Result<u32, BusFault> {
        if Self::user_refused(addr, fc) {
            return Err(BusFault);
        }
        let region = self.region(addr);
        Ok(match region {
            Region::None => return Err(BusFault),
            Region::Io => self.io(addr & 0xFF_FFFF, size, false, 0, fc)?,
            _ => match size {
                Size::Byte => self.byte(&region) as u32,
                Size::Word => {
                    let lo = self.region(addr.wrapping_add(1));
                    u16::from_be_bytes([self.byte(&region), self.byte(&lo)]) as u32
                }
            },
        })
    }

    fn write_cycle(
        &mut self,
        addr: u32,
        size: Size,
        value: u32,
        fc: FunctionCode,
    ) -> Result<(), BusFault> {
        if Self::user_refused(addr, fc) {
            return Err(BusFault);
        }
        match self.region(addr) {
            Region::None => return Err(BusFault),
            Region::Io => {
                self.io(addr & 0xFF_FFFF, size, true, value, fc)?;
            }
            // Nor RAM (see `io`).
            Region::Ram(_) if self.stop.is_some() => {}
            Region::Ram(i) => match size {
                Size::Byte => self.ram[i] = value as u8,
                Size::Word => {
                    self.ram[i] = (value >> 8) as u8;
                    if let Some(b) = self.ram.get_mut(i + 1) {
                        *b = value as u8;
                    }
                }
            },
            // [H] *Internals* p.96: "a write access to these addresses creates a bus error. The GLUE takes care of
            // this."
            Region::Cartridge(_) => return Err(BusFault),
            Region::Rom(_) | Region::UnfittedRam => {}
        }
        Ok(())
    }

    fn iack_cycle(&mut self, level: u8) -> IackResponse {
        match level {
            MFP_LEVEL => {
                self.catch_up_mfp(self.now);
                match self.mfp.iack() {
                    Some(v) => IackResponse::Vector(v),
                    None => IackResponse::Spurious,
                }
            }
            VBL_LEVEL => {
                self.vbl_pending = false;
                IackResponse::Autovector
            }
            HBL_LEVEL => {
                self.hbl_pending = false;
                IackResponse::Autovector
            }
            _ => IackResponse::Spurious,
        }
    }
}

/// The GPIP lines' levels from the outside world (`docs/hardware-facts.md`, "Timing and interrupts"): I7 mono detect
/// ([H] Internals p.61, low = mono), I5 FDC/HDC idle high and I0 Centronics BUSY high ([O] the `$A1` Hatari reads
/// with a colour monitor); I1-I4 and I6 low ([O] same read).
fn gpip_inputs(monitor: Monitor) -> [(u8, bool); 8] {
    [
        (0, true),
        (1, false),
        (2, false),
        (3, false),
        (4, false),
        (5, true),
        (6, false),
        (7, monitor == Monitor::Colour),
    ]
}
