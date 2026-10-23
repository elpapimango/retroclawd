//! Save states: the envelope (spec section 19) around a bincode v2 payload of the machine's hardware.
//!
//! Envelope, all integers big-endian:
//! - magic `RCST`, schema version (`u16`), machine id (`u8` length, bytes), plugin version tag (`u8` length, bytes),
//!   `mc68000` state version (`u16`), accuracy (`u8`), the TOS image's SHA-1 (20 bytes), for each of the two drives
//!   a present flag (`u8`), the media's SHA-1 (20 bytes) and a dirty flag (`u8`), the payload length (`u32`);
//! - the CRC-32 (IEEE) of everything above and the payload, then the payload.
//!
//! On load the state is refused when it names another machine, a newer schema, another CPU state version or accuracy
//! mode, another TOS image (both hashes are in the message), a payload over 64 MiB, a length that is not what follows,
//! or a bad CRC. A different disk than the one the state was made with loads, with a warning: the media is the user's.
//! Disks the guest has not written to are not in the payload (they stay the user's file); a modified disk's content
//! is, so a state taken after a write resumes with the written data.
//!
//! Every byte of a state is untrusted: lengths are checked against what is there before anything is allocated, and
//! the payload is decoded under a size limit.

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

pub const MAGIC: &[u8; 4] = b"RCST";
/// The envelope and payload layout. Older schemas would migrate by an explicit chain; there is only this one.
pub const SCHEMA: u16 = 2;
/// The `mc68000` crate's saved state layout (its `Cpu` as serialised); bump it with that type.
pub const CPU_STATE_VERSION: u16 = 1;
pub const MACHINE_ID: &str = "atari-st";
/// A payload larger than this is refused before anything is read.
pub const MAX_PAYLOAD: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateError {
    /// Not an `RCST` state, or cut short.
    Truncated,
    BadMagic,
    WrongMachine(String),
    NewerSchema(u16),
    OlderSchema(u16),
    CpuVersion {
        saved: u16,
        this: u16,
    },
    Accuracy {
        saved: u8,
        this: u8,
    },
    TooLarge(u64),
    /// The length field does not match the bytes that follow.
    Length {
        declared: u64,
        present: usize,
    },
    BadCrc,
    FirmwareMismatch {
        saved: String,
        loaded: String,
    },
    /// The payload does not decode, or describes a machine this one cannot become.
    Payload(String),
    /// The machine has stopped at an unemulated device: there is nothing to resume.
    Stopped,
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateError::Truncated => write!(f, "the save state is cut short"),
            StateError::BadMagic => write!(
                f,
                "not an Atari ST save state (it does not start with RCST)"
            ),
            StateError::WrongMachine(id) => {
                write!(
                    f,
                    "this save state is for the machine \"{id}\", not \"{MACHINE_ID}\""
                )
            }
            StateError::NewerSchema(v) => write!(
                f,
                "this save state has schema version {v}; this build reads up to {SCHEMA}"
            ),
            StateError::OlderSchema(v) => write!(
                f,
                "this save state has schema version {v}, which this build no longer reads (it needs {SCHEMA});                  save a new one"
            ),
            StateError::CpuVersion { saved, this } => write!(
                f,
                "this save state holds CPU state version {saved}; this build uses {this}"
            ),
            StateError::Accuracy { saved, this } => write!(
                f,
                "this save state was made in {} mode; the machine runs in {} mode",
                accuracy_name(*saved),
                accuracy_name(*this)
            ),
            StateError::TooLarge(n) => write!(
                f,
                "the save state's payload is {n} bytes; the limit is {MAX_PAYLOAD}"
            ),
            StateError::Length { declared, present } => write!(
                f,
                "the save state says its payload is {declared} bytes, but {present} follow"
            ),
            StateError::BadCrc => write!(f, "the save state is damaged (its CRC does not match)"),
            StateError::FirmwareMismatch { saved, loaded } => write!(
                f,
                "this save state was made with the TOS image SHA-1 {saved}; the loaded image is {loaded}"
            ),
            StateError::Payload(why) => {
                write!(f, "the save state does not fit this machine: {why}")
            }
            StateError::Stopped => write!(
                f,
                "the machine has stopped at a device that is not emulated: there is no state to save"
            ),
        }
    }
}

impl std::error::Error for StateError {}

fn accuracy_name(code: u8) -> &'static str {
    match code {
        0 => "fast",
        1 => "accurate",
        2 => "fast with borders",
        3 => "accurate with borders",
        4 => "fast, mono monitor",
        5 => "accurate, mono monitor",
        6 => "fast with borders, mono monitor",
        7 => "accurate with borders, mono monitor",
        _ => "unknown",
    }
}

/// What the envelope records about one drive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriveMeta {
    pub present: bool,
    pub sha1: [u8; 20],
    pub dirty: bool,
}

/// The envelope's fields (everything except the payload).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub version_tag: String,
    pub accuracy: u8,
    pub firmware_sha1: [u8; 20],
    pub drives: [DriveMeta; 2],
}

pub fn sha1_of(bytes: &[u8]) -> [u8; 20] {
    Sha1::digest(bytes).into()
}

pub fn hex(sha: &[u8; 20]) -> String {
    sha.iter().map(|b| format!("{b:02x}")).collect()
}

/// A payload as bincode v2 bytes.
pub fn encode_payload<T: Serialize>(payload: &T) -> Vec<u8> {
    bincode::serde::encode_to_vec(
        payload,
        bincode::config::standard().with_limit::<MAX_PAYLOAD>(),
    )
    .expect("a machine state encodes")
}

/// Bytes back to a payload, under the size limit; trailing bytes are an error.
pub fn decode_payload<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, StateError> {
    let (value, used): (T, usize) = bincode::serde::decode_from_slice(
        bytes,
        bincode::config::standard().with_limit::<MAX_PAYLOAD>(),
    )
    .map_err(|e| StateError::Payload(e.to_string()))?;
    if used != bytes.len() {
        return Err(StateError::Payload("bytes follow the payload".into()));
    }
    Ok(value)
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(255);
    out.push(n as u8);
    out.extend(&b[..n]);
}

/// The envelope around `payload`.
pub fn encode(header: &Header, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 128);
    out.extend(MAGIC);
    out.extend(SCHEMA.to_be_bytes());
    put_str(&mut out, MACHINE_ID);
    put_str(&mut out, &header.version_tag);
    out.extend(CPU_STATE_VERSION.to_be_bytes());
    out.push(header.accuracy);
    out.extend(header.firmware_sha1);
    for d in &header.drives {
        out.push(d.present as u8);
        out.extend(d.sha1);
        out.push(d.dirty as u8);
    }
    out.extend((payload.len() as u32).to_be_bytes());
    let mut crc = crc32fast::Hasher::new();
    crc.update(&out);
    crc.update(payload);
    out.extend(crc.finalize().to_be_bytes());
    out.extend(payload);
    out
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], StateError> {
        let (head, rest) = self.0.split_at_checked(n).ok_or(StateError::Truncated)?;
        self.0 = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, StateError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, StateError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, StateError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn string(&mut self) -> Result<String, StateError> {
        let n = self.u8()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }

    fn sha1(&mut self) -> Result<[u8; 20], StateError> {
        let mut a = [0u8; 20];
        a.copy_from_slice(self.take(20)?);
        Ok(a)
    }
}

/// Checks the envelope against what this machine is (`expect`: its accuracy and TOS image hash) and returns the
/// header and the payload bytes. The payload is not decoded here.
pub fn decode<'a>(
    bytes: &'a [u8],
    accuracy: u8,
    firmware_sha1: &[u8; 20],
) -> Result<(Header, &'a [u8]), StateError> {
    let mut r = Reader(bytes);
    if r.take(4)? != MAGIC {
        return Err(StateError::BadMagic);
    }
    let schema = r.u16()?;
    if schema > SCHEMA {
        return Err(StateError::NewerSchema(schema));
    }
    if schema < SCHEMA {
        return Err(StateError::OlderSchema(schema));
    }
    let id = r.string()?;
    if id != MACHINE_ID {
        return Err(StateError::WrongMachine(id));
    }
    let version_tag = r.string()?;
    let cpu = r.u16()?;
    let saved_accuracy = r.u8()?;
    let saved_firmware = r.sha1()?;
    let mut drives = [DriveMeta::default(); 2];
    for d in &mut drives {
        d.present = r.u8()? != 0;
        d.sha1 = r.sha1()?;
        d.dirty = r.u8()? != 0;
    }
    let declared = r.u32()? as u64;
    if declared > MAX_PAYLOAD as u64 {
        return Err(StateError::TooLarge(declared));
    }
    let header_len = bytes.len() - r.0.len();
    let crc = r.u32()?;
    let payload = r.0;
    if payload.len() as u64 != declared {
        return Err(StateError::Length {
            declared,
            present: payload.len(),
        });
    }
    let mut h = crc32fast::Hasher::new();
    h.update(&bytes[..header_len]);
    h.update(payload);
    if h.finalize() != crc {
        return Err(StateError::BadCrc);
    }
    if cpu != CPU_STATE_VERSION {
        return Err(StateError::CpuVersion {
            saved: cpu,
            this: CPU_STATE_VERSION,
        });
    }
    if saved_accuracy != accuracy {
        return Err(StateError::Accuracy {
            saved: saved_accuracy,
            this: accuracy,
        });
    }
    if &saved_firmware != firmware_sha1 {
        return Err(StateError::FirmwareMismatch {
            saved: hex(&saved_firmware),
            loaded: hex(firmware_sha1),
        });
    }
    Ok((
        Header {
            version_tag,
            accuracy: saved_accuracy,
            firmware_sha1: saved_firmware,
            drives,
        },
        payload,
    ))
}
