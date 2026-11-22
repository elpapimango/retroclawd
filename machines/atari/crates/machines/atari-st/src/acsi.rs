//! ACSI: hard disks behind the DMA chip (`docs/hardware-facts.md`, "ACSI").
//!
//! The command protocol is Atari's *Application Notes on the Atari Computer System Interface* (ACSI), 27 September
//! 1985 (<https://mirrorservice.org/sites/www.bitsavers.org/pdf/atari/ST/Atari_ST_GEM_Programming_1986/GEM_0942.pdf>,
//! SHA-256 `a027f606fe71b0df5c33f2e86f38f20216a891bb1b4e613bdf3e8bf00e74f9fb`), sections 2 and 3:
//! - a command is a 6-byte descriptor block written to `$FF8604` with the mode register selecting the controller's
//!   command register (bit 3): the first byte with A0 = 0, the rest with A0 = 1; byte 0 holds the controller number
//!   (bits 7-5) and the operation code (bits 4-0), bytes 1-3 the block address (21 bits, bits 7-5 of byte 1 being the
//!   device number), byte 4 the block count, byte 5 the control byte;
//! - the operation codes of the document are `$00` TEST UNIT READY, `$08` READ, `$0A` WRITE, `$0B` SEEK and `$1A` MODE
//!   SENSE; transfers go through the DMA chip in 16-byte units with the sector count register as the trigger;
//! - the controller ends a command with a completion status byte: `$00` no error, `$04` device not ready, `$20`
//!   invalid command, `$21` invalid address, `$23` volume overflow, `$24` invalid argument, `$25` invalid device
//!   number; it also says the controller "has complete control of the bus until the status byte is returned".
//!
//! What the document does not give is labelled `[H?]` where it is used: the completion time (the command is finished
//! when the last command byte arrives, and the interrupt line GPIP I5 falls at once), what INQUIRY, REQUEST SENSE,
//! MODE SENSE and READ CAPACITY return (SCSI-2 layouts, as the controllers of the time answered), the extended 11-byte
//! command (first byte opcode `$1F`, then a 10-byte descriptor), and the meaning of a block count of 0 (256).

use serde::{Deserialize, Serialize};

pub const BLOCK_BYTES: usize = 512;
/// The largest disk image: checked when the disk is attached, so the block arithmetic cannot overflow later.
pub const MAX_DISK_BYTES: u64 = 512 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockError {
    OutOfRange,
}

/// The medium behind a target: whole 512-byte blocks.
pub trait BlockDevice: Send {
    /// Size in bytes (a multiple of 512).
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn read(&mut self, lba: u64, out: &mut [u8]) -> Result<(), BlockError>;
    fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError>;
    /// The whole medium, when it is held in memory (to save it).
    fn image(&self) -> Option<&[u8]> {
        None
    }
    /// Whether the guest has written since the image was loaded or last marked saved.
    fn is_modified(&self) -> bool {
        false
    }
    fn mark_saved(&mut self) {}
}

/// A disk held in memory. A guest write changes this copy only; the file changes only when the user asks for the
/// image to be saved.
#[derive(Debug, Clone)]
pub struct MemoryDisk {
    data: Vec<u8>,
    modified: bool,
}

impl MemoryDisk {
    /// A raw image: not empty, a multiple of 512 bytes and at most [`MAX_DISK_BYTES`].
    pub fn from_image(bytes: Vec<u8>) -> Result<Self, String> {
        let n = bytes.len() as u64;
        if n == 0 {
            return Err("the hard disk image is empty".to_string());
        }
        if n > MAX_DISK_BYTES {
            return Err(format!(
                "the hard disk image is {n} bytes; the limit is {MAX_DISK_BYTES} (512 MiB)"
            ));
        }
        if !n.is_multiple_of(BLOCK_BYTES as u64) {
            return Err(format!(
                "the hard disk image is {n} bytes: not a whole number of 512-byte blocks"
            ));
        }
        Ok(Self {
            data: bytes,
            modified: false,
        })
    }

    pub fn image(&self) -> &[u8] {
        &self.data
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    pub fn mark_saved(&mut self) {
        self.modified = false;
    }
}

impl BlockDevice for MemoryDisk {
    fn len(&self) -> u64 {
        self.data.len() as u64
    }

    fn read(&mut self, lba: u64, out: &mut [u8]) -> Result<(), BlockError> {
        let at = lba
            .checked_mul(BLOCK_BYTES as u64)
            .ok_or(BlockError::OutOfRange)?;
        let end = at
            .checked_add(out.len() as u64)
            .ok_or(BlockError::OutOfRange)?;
        if end > self.len() {
            return Err(BlockError::OutOfRange);
        }
        out.copy_from_slice(&self.data[at as usize..end as usize]);
        Ok(())
    }

    fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        let at = lba
            .checked_mul(BLOCK_BYTES as u64)
            .ok_or(BlockError::OutOfRange)?;
        let end = at
            .checked_add(data.len() as u64)
            .ok_or(BlockError::OutOfRange)?;
        if end > self.len() {
            return Err(BlockError::OutOfRange);
        }
        self.data[at as usize..end as usize].copy_from_slice(data);
        self.modified = true;
        Ok(())
    }

    fn image(&self) -> Option<&[u8]> {
        Some(&self.data)
    }

    fn is_modified(&self) -> bool {
        self.modified
    }

    fn mark_saved(&mut self) {
        self.modified = false;
    }
}

/// Completion status bytes (section 2.2 and "Command Errors").
pub const STATUS_OK: u8 = 0x00;
pub const STATUS_INVALID_COMMAND: u8 = 0x20;
pub const STATUS_INVALID_ADDRESS: u8 = 0x21;
pub const STATUS_INVALID_DEVICE: u8 = 0x25;

/// What the controller needs the DMA to do for a command.
#[derive(Debug, PartialEq, Eq)]
pub enum Transfer {
    None,
    /// Data for the host: the DMA writes it to memory.
    ToHost(Vec<u8>),
    /// The host sends `bytes` bytes (the DMA reads them from memory) to be written at block `lba` of `target`.
    FromHost {
        target: u8,
        lba: u64,
        bytes: usize,
    },
    /// The host sends `bytes` bytes that the device takes and does not keep (the parameter list of MODE SELECT).
    Discard(usize),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub status: u8,
    pub transfer: Transfer,
}

/// The controller's own state (the disks are the user's media and are not part of it).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AcsiState {
    cmd: Vec<u8>,
    /// The completion status waiting to be read, and the target that gave it.
    status: Option<u8>,
    /// GPIP I5 is held low until the status is read.
    irq: bool,
    /// The sense key of the last failed command per target ([H?] REQUEST SENSE).
    sense: [u8; 8],
}

pub struct Acsi {
    disks: [Option<Box<dyn BlockDevice>>; 8],
    pub(crate) state: AcsiState,
}

impl std::fmt::Debug for Acsi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Acsi")
            .field(
                "targets",
                &self.disks.iter().map(|d| d.is_some()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Default for Acsi {
    fn default() -> Self {
        Self::new()
    }
}

impl Acsi {
    pub fn new() -> Self {
        Self {
            disks: Default::default(),
            state: AcsiState::default(),
        }
    }

    pub fn attach(&mut self, id: u8, disk: Box<dyn BlockDevice>) -> Result<(), String> {
        let slot = self
            .disks
            .get_mut(id as usize)
            .ok_or_else(|| format!("ACSI target {id} does not exist (0-7)"))?;
        *slot = Some(disk);
        Ok(())
    }

    pub fn detach(&mut self, id: u8) {
        if let Some(slot) = self.disks.get_mut(id as usize) {
            *slot = None;
        }
    }

    pub fn present(&self, id: u8) -> bool {
        self.disks.get(id as usize).is_some_and(Option::is_some)
    }

    pub fn any_present(&self) -> bool {
        self.disks.iter().any(Option::is_some)
    }

    pub fn disk(&self, id: u8) -> Option<&dyn BlockDevice> {
        self.disks.get(id as usize)?.as_deref()
    }

    pub fn disk_mut(&mut self, id: u8) -> Option<&mut (dyn BlockDevice + 'static)> {
        self.disks.get_mut(id as usize)?.as_deref_mut()
    }

    /// Whether the interrupt line (GPIP I5, active low) is held by a finished command.
    pub fn irq(&self) -> bool {
        self.state.irq
    }

    /// Whether there is a completion status to read.
    pub fn has_status(&self) -> bool {
        self.state.status.is_some()
    }

    /// The completion status, read at `$FF8604`: it also releases the interrupt line. [H?] the line is released by
    /// reading the status.
    pub fn take_status(&mut self) -> Option<u8> {
        self.state.irq = false;
        self.state.status.take()
    }

    pub fn state(&self) -> AcsiState {
        self.state.clone()
    }

    /// Checks a state read from a file (every field is a plain value, so only the command length is bounded).
    pub fn restore(&mut self, s: AcsiState) -> Result<(), String> {
        if s.cmd.len() > 11 {
            return Err("the ACSI state holds a command longer than any command".to_string());
        }
        self.state = s;
        Ok(())
    }

    /// How long the command that starts with `first` is: 6 bytes, or 11 with the extended-command escape `$1F`.
    fn command_len(first: u8) -> usize {
        if first & 0x1F == 0x1F { 11 } else { 6 }
    }

    /// A command byte written to `$FF8604` with the mode register selecting the controller. `first` is the byte with
    /// A0 = 0 (the controller select), the others come with A0 = 1. Returns the command's outcome when this byte
    /// completes it; `None` otherwise, and for a command for a target that is not there (it is ignored as before).
    pub fn command_byte(&mut self, first: bool, byte: u8) -> Option<(u8, Vec<u8>)> {
        // Each byte written takes the line the controller raised for the one before it (or for the last command).
        self.state.irq = false;
        if first {
            self.state.cmd.clear();
            self.state.status = None;
        }
        if self.state.cmd.is_empty() && !first {
            // A stray byte with no select before it.
            return None;
        }
        self.state.cmd.push(byte);
        let want = Self::command_len(self.state.cmd[0]);
        if self.state.cmd.len() < want {
            // [H?] the controller acknowledges every command byte but the last with its interrupt line, and the driver
            // waits for it before it sends the next (the document's "Command Phase" shows the line beside each byte;
            // EmuTOS waits on GPIP I5 after the select byte).
            if self.present(self.state.cmd[0] >> 5) {
                self.state.irq = true;
            }
            return None;
        }
        let cdb = std::mem::take(&mut self.state.cmd);
        let target = cdb[0] >> 5;
        self.present(target).then_some((target, cdb))
    }

    /// Runs a complete command.
    pub fn execute(&mut self, target: u8, cdb: &[u8]) -> Outcome {
        let ext = cdb[0] & 0x1F == 0x1F;
        let (op, args) = if ext {
            (cdb[1], &cdb[2..])
        } else {
            (cdb[0] & 0x1F, &cdb[1..])
        };
        let lun = if ext { 0 } else { args[0] >> 5 };
        let ok = |transfer| Outcome {
            status: STATUS_OK,
            transfer,
        };
        let fail = |this: &mut Self, status: u8, key: u8| {
            this.state.sense[target as usize & 7] = key;
            Outcome {
                status,
                transfer: Transfer::None,
            }
        };
        let blocks_on_disk = self
            .disk(target)
            .map_or(0, |d| d.len() / BLOCK_BYTES as u64);
        if lun != 0 {
            return fail(self, STATUS_INVALID_DEVICE, 5);
        }
        match (ext, op) {
            (false, 0x00) => ok(Transfer::None),
            // Seek: the address must exist.
            (false, 0x0B) => {
                let lba = lba6(args);
                if lba >= blocks_on_disk {
                    return fail(self, STATUS_INVALID_ADDRESS, 5);
                }
                ok(Transfer::None)
            }
            (false, 0x08 | 0x0A) => {
                let lba = lba6(args);
                // [H?] a count of 0 is 256 blocks.
                let count = if args[3] == 0 { 256 } else { args[3] as u64 };
                if lba
                    .checked_add(count)
                    .is_none_or(|end| end > blocks_on_disk)
                {
                    return fail(self, STATUS_INVALID_ADDRESS, 5);
                }
                self.state.sense[target as usize & 7] = 0;
                if op == 0x08 {
                    let mut data = vec![0u8; count as usize * BLOCK_BYTES];
                    match self.disk_mut(target).map(|d| d.read(lba, &mut data)) {
                        Some(Ok(())) => ok(Transfer::ToHost(data)),
                        _ => fail(self, STATUS_INVALID_ADDRESS, 3),
                    }
                } else {
                    ok(Transfer::FromHost {
                        target,
                        lba,
                        bytes: count as usize * BLOCK_BYTES,
                    })
                }
            }
            // The four commands of the 1986 table that this device has nothing to do for ([D] *ST spec* pp.19-20 names them;
            // [H?] what they do is a choice: a disk image has no medium to verify, format or correct, so a range that
            // exists is accepted and nothing changes, a FORMAT TRACK in particular leaves the data as it is).
            (false, 0x05 | 0x06) => {
                let lba = lba6(args);
                let count = if args[3] == 0 { 256 } else { args[3] as u64 };
                if lba
                    .checked_add(count)
                    .is_none_or(|end| end > blocks_on_disk)
                {
                    return fail(self, STATUS_INVALID_ADDRESS, 5);
                }
                self.state.sense[target as usize & 7] = 0;
                ok(Transfer::None)
            }
            (false, 0x0D) => {
                self.state.sense[target as usize & 7] = 0;
                ok(Transfer::None)
            }
            // MODE SELECT(6): the parameter list (its length is byte 4, 0 = none) is taken and ignored ([H?]).
            (false, 0x15) => {
                self.state.sense[target as usize & 7] = 0;
                match args[3] {
                    0 => ok(Transfer::None),
                    n => ok(Transfer::Discard(n as usize)),
                }
            }
            // [H?] SCSI-2 INQUIRY: a direct-access device, fixed vendor and product, 36 bytes, cut to the allocation.
            (false, 0x12) => {
                let mut d = vec![0u8; 36];
                d[4] = 31;
                d[8..16].copy_from_slice(b"retroclw");
                d[16..32].copy_from_slice(b"ACSI hard disk  ");
                d[32..36].copy_from_slice(b"1.0 ");
                d.truncate(if args[3] == 0 { 4 } else { args[3] as usize });
                ok(Transfer::ToHost(d))
            }
            // [H?] SCSI-2 REQUEST SENSE: fixed format, the key of the last failed command.
            (false, 0x03) => {
                let mut d = vec![0u8; 18];
                d[0] = 0x70;
                d[2] = std::mem::take(&mut self.state.sense[target as usize & 7]);
                d[7] = 10;
                d.truncate(if args[3] == 0 { 4 } else { args[3] as usize });
                ok(Transfer::ToHost(d))
            }
            // [H?] MODE SENSE(6): a header and one block descriptor with the block count and size.
            (false, 0x1A) => {
                let mut d = vec![0u8; 12];
                d[0] = 11;
                d[3] = 8;
                d[5..8].copy_from_slice(&(blocks_on_disk as u32).to_be_bytes()[1..]);
                d[9..12].copy_from_slice(&(BLOCK_BYTES as u32).to_be_bytes()[1..]);
                let n = if args[3] == 0 { 4 } else { args[3] as usize };
                d.truncate(n);
                ok(Transfer::ToHost(d))
            }
            // [H?] the extended command: SCSI-2 READ CAPACITY(10), the last block and the block size.
            (true, 0x25) => {
                let mut d = Vec::with_capacity(8);
                d.extend((blocks_on_disk.saturating_sub(1) as u32).to_be_bytes());
                d.extend((BLOCK_BYTES as u32).to_be_bytes());
                ok(Transfer::ToHost(d))
            }
            _ => fail(self, STATUS_INVALID_COMMAND, 5),
        }
    }

    /// The data of a WRITE has come from memory: the blocks go to the disk.
    pub fn finish_write(&mut self, target: u8, lba: u64, data: &[u8]) -> u8 {
        match self.disk_mut(target).map(|d| d.write(lba, data)) {
            Some(Ok(())) => STATUS_OK,
            _ => STATUS_INVALID_ADDRESS,
        }
    }

    /// The command is finished: its status waits to be read and the interrupt line falls.
    pub fn complete(&mut self, status: u8) {
        self.state.status = Some(status);
        self.state.irq = true;
    }
}

/// The 21-bit block address of a 6-byte descriptor: byte 1 bits 4-0, then bytes 2 and 3 (`args` start at byte 1).
fn lba6(args: &[u8]) -> u64 {
    (u64::from(args[0] & 0x1F) << 16) | (u64::from(args[1]) << 8) | u64::from(args[2])
}
