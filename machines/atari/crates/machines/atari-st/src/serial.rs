//! What the ST's serial lines (the MFP's RS-232, the MIDI port) can be connected to: files and an echo, made from the
//! plugin's options. The guest never chooses a path: the user gives each file, the emulator opens it, and nothing the
//! guest does reaches another file.
//!
//! - An output file is created and never replaced unless the user asked (`overwrite`); it is written as bytes arrive.
//! - An input file is read whole, at most [`MAX_INPUT`] bytes, then sent one character at a time.
//! - Loopback sends every character back, as a cable from the transmit to the receive pin would.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;

pub use crate::acia::MIDI_FRAME_CYCLES;
use crate::acia::SerialPeer;

/// The most an input file may hold: a hostile or mistaken path (a device, a huge file) must not fill memory.
pub const MAX_INPUT: u64 = 16 << 20;

/// A file the guest's characters are written to.
#[derive(Debug)]
pub struct FileSink {
    out: BufWriter<File>,
}

impl FileSink {
    /// Creates `path`. An existing file is an error unless `overwrite` is set (the user's say-so).
    pub fn create(path: &Path, overwrite: bool) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true);
        if overwrite {
            options.create(true).truncate(true);
        } else {
            options.create_new(true);
        }
        let file = options.open(path).map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                io::Error::new(
                    e.kind(),
                    format!(
                        "{} exists; the output is not overwritten unless asked to",
                        path.display()
                    ),
                )
            } else {
                e
            }
        })?;
        Ok(Self {
            out: BufWriter::new(file),
        })
    }

    pub fn write(&mut self, byte: u8) {
        // A full disk loses output; it must not stop the machine.
        let _ = self.out.write_all(&[byte]);
        let _ = self.out.flush();
    }
}

/// A file read in as the characters a device on the line sends.
pub struct FileSource {
    bytes: VecDeque<u8>,
}

impl FileSource {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(io::Error::other(format!(
                "{} is not a regular file",
                path.display()
            )));
        }
        let mut bytes = Vec::new();
        file.take(MAX_INPUT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_INPUT {
            return Err(io::Error::other(format!(
                "{} is larger than {MAX_INPUT} bytes",
                path.display()
            )));
        }
        Ok(Self::from_bytes(bytes))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }
}

/// The device on the other end of a serial line: an optional output file, an optional input file, an optional echo.
/// `frame_cycles` is the CPU cycles one character takes on a line the peer is timed by (the MIDI port's ACIA).
pub struct LinePeer {
    sink: Option<FileSink>,
    source: Option<FileSource>,
    loopback: bool,
    echo: VecDeque<(u64, u8)>,
    frame_cycles: u64,
}

impl LinePeer {
    pub fn new(
        sink: Option<FileSink>,
        source: Option<FileSource>,
        loopback: bool,
        frame_cycles: u64,
    ) -> Self {
        Self {
            sink,
            source,
            loopback,
            echo: VecDeque::new(),
            frame_cycles,
        }
    }
}

impl SerialPeer for LinePeer {
    fn receive(&mut self, byte: u8, at: u64) {
        if let Some(sink) = &mut self.sink {
            sink.write(byte);
        }
        if self.loopback {
            // A cable from the transmit to the receive pin: bounded, so a guest that never reads cannot grow it.
            if self.echo.len() < 4096 {
                self.echo.push_back((at, byte));
            }
        }
    }

    fn next_byte(&mut self, now: u64) -> Option<(u64, u8)> {
        if let Some(&(at, byte)) = self.echo.front()
            && at <= now
        {
            self.echo.pop_front();
            return Some((at, byte));
        }
        self.source
            .as_mut()
            .and_then(|s| s.bytes.pop_front())
            .map(|b| (now, b))
    }

    fn frame_cycles(&self) -> u64 {
        self.frame_cycles
    }
}
