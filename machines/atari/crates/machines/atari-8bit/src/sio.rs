//! Two sector-image SIO peripherals, not emulated 810 CPUs/FDCs.
//! Altirra Hardware Reference 2026-01-02, sections 9.1 and 10.2.
//! Commands travel on actual PIA COMMAND and POKEY serial data pins.
//! Sector-image timing is deterministic, not rotational/FDC emulation.
//! PUT ($50) and WRITE ($57) share reliable image-sector writes: verification
//! cannot fail for an in-memory image. FORMAT and high-speed commands NAK.
use atari_media::Atr;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SioState {
    image: Option<Vec<u8>>,
    bit_cycles: u16,
    command: bool,
    frame: Vec<u8>,
    receive_bits: u8,
    receive_shift: u8,
    receive_delay: u16,
    transmit: u16,
    transmit_bits: u8,
    transmit_delay: u32,
    output: bool,
    queue: VecDeque<(u8, u32)>,
    pub reads: u64,
    pub sector: u16,
    write_error: bool,
}

pub(crate) struct Sio {
    disk: Option<Atr>,
    disk_b: Option<Atr>,
    state: SioState,
    media: SioMediaState,
}

/// Separate Machine v5 extension. Never append fields to the legacy SioState.
/// Restore onto a newly validated Sio::restore result before committing the Machine.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct SioMediaState {
    image_b: Option<Vec<u8>>,
    dirty: [bool; 2],
    // 0 = ATR, 1 = logical XFD, 2 = physical XFD (zero boot padding).
    formats: [u8; 2],
    write_error_b: bool,
    pending: Option<WriteFrame>,
}

#[derive(Clone, Serialize, Deserialize)]
struct WriteFrame {
    drive: u8,
    sector: u16,
    expected: u16,
    bytes: Vec<u8>,
    framing_error: bool,
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, &byte| {
        let value = u16::from(sum) + u16::from(byte);
        (value as u8).wrapping_add((value >> 8) as u8)
    })
}

impl Sio {
    pub fn new(clock: u64) -> Self {
        Self {
            disk: None,
            disk_b: None,
            media: SioMediaState::default(),
            state: SioState {
                image: None,
                bit_cycles: ((clock + 9520) / 19040) as u16,
                command: true,
                frame: Vec::new(),
                receive_bits: 0,
                receive_shift: 0,
                receive_delay: 0,
                transmit: 0,
                transmit_bits: 0,
                transmit_delay: 0,
                output: true,
                queue: VecDeque::new(),
                reads: 0,
                sector: 0,
                write_error: false,
            },
        }
    }
    pub fn attach(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.attach_in(0, bytes)
    }
    pub fn attach_in(&mut self, drive: u8, bytes: &[u8]) -> Result<(), String> {
        self.attach_format(drive, "atr", bytes)
    }
    pub fn attach_format(
        &mut self,
        drive: u8,
        extension: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        Self::valid_drive(drive)?;
        let (disk, format) = if extension.eq_ignore_ascii_case("atr") {
            (Atr::parse(bytes)?, 0)
        } else if extension.eq_ignore_ascii_case("xfd") {
            let disk = Atr::parse_xfd(bytes)?;
            let format = if disk.sector_size() == 256 && bytes.len().is_multiple_of(256) {
                2
            } else {
                1
            };
            (disk, format)
        } else {
            return Err(format!(
                "unsupported Atari disk format {extension}; expected atr or xfd"
            ));
        };
        if drive == 0 {
            self.disk = Some(disk)
        } else {
            self.disk_b = Some(disk)
        }
        self.media.formats[usize::from(drive)] = format;
        self.media.dirty[usize::from(drive)] = false;
        self.abort();
        self.state.reads = 0;
        self.state.sector = 0;
        if drive == 0 {
            self.state.write_error = false;
        } else {
            self.media.write_error_b = false;
        }
        Ok(())
    }
    pub fn detach(&mut self) {
        self.detach_in(0).expect("drive zero");
    }
    fn valid_drive(drive: u8) -> Result<(), String> {
        if drive < 2 {
            Ok(())
        } else {
            Err("Atari SIO supports only drives 0 and 1 (D1:/D2:)".into())
        }
    }
    fn disk_in(&self, drive: u8) -> Option<&Atr> {
        match drive {
            0 => self.disk.as_ref(),
            1 => self.disk_b.as_ref(),
            _ => None,
        }
    }
    pub fn detach_in(&mut self, drive: u8) -> Result<(), String> {
        Self::valid_drive(drive)?;
        if drive == 0 {
            self.disk = None
        } else {
            self.disk_b = None
        }
        self.media.formats[usize::from(drive)] = 0;
        self.media.dirty[usize::from(drive)] = false;
        self.abort();
        Ok(())
    }
    pub fn attached_in(&self, drive: u8) -> bool {
        self.disk_in(drive).is_some()
    }
    pub fn dirty_in(&self, drive: u8) -> bool {
        self.media
            .dirty
            .get(usize::from(drive))
            .copied()
            .unwrap_or(false)
    }
    pub fn mark_saved_in(&mut self, drive: u8) -> Result<(), String> {
        Self::valid_drive(drive)?;
        self.media.dirty[usize::from(drive)] = false;
        Ok(())
    }
    pub fn export_in(&self, drive: u8) -> Option<Vec<u8>> {
        let disk = self.disk_in(drive)?;
        Some(match self.media.formats[usize::from(drive)] {
            1 => disk.to_xfd(false),
            2 => disk.to_xfd(true),
            _ => disk.to_bytes(),
        })
    }
    pub fn extension_in(&self, drive: u8) -> Option<&'static str> {
        self.disk_in(drive)?;
        Some(if self.media.formats[usize::from(drive)] == 0 {
            "atr"
        } else {
            "xfd"
        })
    }
    pub fn attached(&self) -> bool {
        self.disk.is_some()
    }
    pub fn export(&self) -> Option<Vec<u8>> {
        self.export_in(0)
    }
    pub fn reads(&self) -> u64 {
        self.state.reads
    }
    pub fn sector(&self) -> u16 {
        self.state.sector
    }
    pub fn busy(&self) -> bool {
        !self.state.command
            || self.state.transmit_bits != 0
            || !self.state.queue.is_empty()
            || self.media.pending.is_some()
    }
    pub fn save(&self) -> SioState {
        let mut state = self.state.clone();
        // Legacy SioState always stores canonical ATR, even for an XFD source.
        state.image = self.disk.as_ref().map(Atr::to_bytes);
        state
    }
    pub fn save_media(&self) -> SioMediaState {
        let mut media = self.media.clone();
        media.image_b = self.disk_b.as_ref().map(Atr::to_bytes);
        media
    }
    pub fn restore_media(&mut self, mut media: SioMediaState) -> Result<(), String> {
        let disk_b = media.image_b.as_deref().map(Atr::parse).transpose()?;
        for (drive, disk) in [self.disk.as_ref(), disk_b.as_ref()]
            .into_iter()
            .enumerate()
        {
            let format = media.formats[drive];
            if format > 2 || (disk.is_none() && (format != 0 || media.dirty[drive])) {
                return Err("invalid Atari SIO media format/dirty state".into());
            }
            if let Some(disk) = disk.filter(|_| format != 0) {
                let raw = disk.to_xfd(format == 2);
                if Atr::parse_xfd(&raw)? != *disk {
                    return Err("invalid Atari SIO normalized XFD state".into());
                }
            }
        }
        if let Some(pending) = &media.pending {
            let disk = match pending.drive {
                0 => self.disk.as_ref(),
                1 => disk_b.as_ref(),
                _ => None,
            }
            .ok_or("invalid Atari SIO pending write drive")?;
            let expected = disk.sector(pending.sector)?.len();
            let frame = &self.state.frame;
            if usize::from(pending.expected) != expected
                || pending.bytes.len() > expected
                || !self.state.command
                || frame.len() != 5
                || frame[0] != 0x31 + pending.drive
                || !matches!(frame[1], 0x50 | 0x57)
                || u16::from_le_bytes([frame[2], frame[3]]) != pending.sector
                || checksum(&frame[..4]) != frame[4]
            {
                return Err("invalid Atari SIO pending write state".into());
            }
        }
        media.image_b = None;
        self.disk_b = disk_b;
        self.media = media;
        Ok(())
    }
    pub fn restore(state: SioState, clock: u64) -> Result<Self, String> {
        if state.bit_cycles != ((clock + 9520) / 19040) as u16
            || state.frame.len() > 6
            || state.receive_bits > 10
            || state.receive_delay > state.bit_cycles
            || state.transmit > 0x3ff
            || state.transmit_bits > 10
            || state.transmit_delay > 20000
            || state.queue.len() > 260
            || state.queue.iter().any(|&(_, delay)| delay > 20000)
        {
            return Err("invalid Atari SIO transfer state".to_string());
        }
        let disk = state.image.as_deref().map(Atr::parse).transpose()?;
        let mut state = state;
        state.image = None;
        Ok(Self {
            disk,
            disk_b: None,
            state,
            media: SioMediaState::default(),
        })
    }
    fn abort(&mut self) {
        self.media.pending = None;
        self.state.frame.clear();
        self.state.queue.clear();
        self.state.receive_bits = 0;
        self.state.receive_delay = 0;
        self.state.transmit_bits = 0;
        self.state.transmit_delay = 0;
        self.state.output = true;
    }
    fn execute(&mut self) {
        let s = &mut self.state;
        if s.frame.len() != 5
            || !(0x31..=0x32).contains(&s.frame[0])
            || checksum(&s.frame[..4]) != s.frame[4]
        {
            return;
        }
        let drive = s.frame[0] - 0x31;
        let Some(disk) = (if drive == 0 { &self.disk } else { &self.disk_b }) else {
            return;
        };
        let sector = u16::from_le_bytes([s.frame[2], s.frame[3]]);
        // AHR 9.1: write commands ACK the command, receive data with COMMAND
        // high, ACK/NAK its checksum, then report C/E. No ROM/SIO trap.
        if matches!(s.frame[1], 0x50 | 0x57) {
            match disk.sector(sector) {
                Ok(bytes) => {
                    self.media.pending = Some(WriteFrame {
                        drive,
                        sector,
                        expected: bytes.len() as u16,
                        bytes: Vec::new(),
                        framing_error: false,
                    });
                    s.queue.push_back((b'A', 1800));
                }
                Err(_) => s.queue.push_back((b'N', 1800)),
            }
            return;
        }
        let write_error = if drive == 0 {
            s.write_error
        } else {
            self.media.write_error_b
        };
        let data = match s.frame[1] {
            0x52 => match disk.sector(sector) {
                Ok(bytes) => {
                    s.sector = sector;
                    s.reads = s.reads.saturating_add(1);
                    Some(bytes.to_vec())
                }
                Err(_) => None,
            },
            0x53 => Some(vec![
                0x10 | if disk.sector_size() == 256 { 0x20 } else { 0 }
                    | if disk.sector_count() == 1040 { 0x80 } else { 0 }
                    | if disk.write_protected() && write_error {
                        8
                    } else {
                        0
                    }
                    | if write_error { 4 } else { 0 },
                if disk.write_protected() && write_error {
                    0xbf
                } else {
                    0xff
                },
                0xe0,
                0,
            ]),
            0x21 | 0x22 => {
                if drive == 0 {
                    s.write_error = true
                } else {
                    self.media.write_error_b = true
                }
                None
            }
            _ => None,
        };
        s.queue
            .push_back((if data.is_some() { b'A' } else { b'N' }, 1800));
        if let Some(data) = data {
            s.queue.push_back((b'C', 1000));
            let sum = checksum(&data);
            s.queue.extend(data.into_iter().map(|byte| (byte, 0)));
            s.queue.push_back((sum, 0));
        }
    }
    fn receive(&mut self, input: bool) -> Option<(u8, bool)> {
        let s = &mut self.state;
        if s.receive_bits == 0 {
            if input {
                return None;
            }
            s.receive_bits = 1;
            s.receive_delay = s.bit_cycles / 2;
        }
        s.receive_delay = s.receive_delay.saturating_sub(1);
        if s.receive_delay != 0 {
            return None;
        }
        match s.receive_bits {
            1 => {
                if input {
                    s.receive_bits = 0;
                    return None;
                }
                s.receive_shift = 0;
            }
            2..=9 => s.receive_shift |= u8::from(input) << (s.receive_bits - 2),
            10 => {
                s.receive_bits = 0;
                return Some((s.receive_shift, input));
            }
            _ => unreachable!(),
        }
        s.receive_bits += 1;
        s.receive_delay = s.bit_cycles;
        None
    }
    fn write_byte(&mut self, byte: u8, framed: bool) {
        let pending = self.media.pending.as_mut().expect("pending write");
        pending.framing_error |= !framed;
        if pending.bytes.len() < usize::from(pending.expected) {
            pending.bytes.push(byte);
            return;
        }
        let pending = self.media.pending.take().expect("complete write");
        if pending.framing_error || checksum(&pending.bytes) != byte {
            self.state.queue.push_back((b'N', 1800));
            return;
        }
        self.state.queue.push_back((b'A', 1800));
        let disk = if pending.drive == 0 {
            self.disk.as_mut()
        } else {
            self.disk_b.as_mut()
        };
        let result = disk
            .ok_or_else(|| "disk detached".to_string())
            .and_then(|disk| disk.sector_mut(pending.sector));
        let error = match result {
            Ok(sector) => {
                sector.copy_from_slice(&pending.bytes);
                self.media.dirty[usize::from(pending.drive)] = true;
                self.state.sector = pending.sector;
                false
            }
            Err(_) => true,
        };
        if pending.drive == 0 {
            self.state.write_error = error
        } else {
            self.media.write_error_b = error
        }
        self.state
            .queue
            .push_back((if error { b'E' } else { b'C' }, 1000));
    }
    fn transmit(&mut self) {
        let s = &mut self.state;
        if s.transmit_delay != 0 {
            s.transmit_delay -= 1;
            if s.transmit_delay != 0 {
                return;
            }
        }
        if s.transmit_bits != 0 {
            s.transmit >>= 1;
            s.transmit_bits -= 1;
        }
        if s.transmit_bits == 0 {
            if let Some(&(_, delay)) = s.queue.front() {
                if delay != 0 {
                    s.queue.front_mut().expect("queued byte").1 = 0;
                    s.transmit_delay = delay;
                    s.output = true;
                    return;
                }
                let (byte, _) = s.queue.pop_front().expect("queued byte");
                s.transmit = (u16::from(byte) << 1) | 0x200;
                s.transmit_bits = 10;
            } else {
                s.output = true;
                return;
            }
        }
        s.output = s.transmit & 1 != 0;
        s.transmit_delay = u32::from(s.bit_cycles);
    }
    pub fn tick(&mut self, command: bool, data: bool) -> bool {
        if command != self.state.command {
            if !command {
                self.abort();
            } else {
                self.execute();
            }
            self.state.command = command;
        }
        if !command {
            if let Some((byte, framed)) = self.receive(data) {
                if framed && self.state.frame.len() < 6 {
                    self.state.frame.push(byte);
                } else {
                    self.state.frame.resize(6, 0);
                }
            }
        } else {
            if self.media.pending.is_some()
                && let Some((byte, framed)) = self.receive(data)
            {
                self.write_byte(byte, framed);
            }
            self.transmit();
        }
        self.state.output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command(sio: &mut Sio, id: u8, code: u8, sector: u16, valid: bool) {
        let mut frame = vec![id, code, sector as u8, (sector >> 8) as u8];
        frame.push(checksum(&frame) ^ u8::from(!valid));
        sio.tick(false, true);
        for byte in frame {
            for bit in std::iter::once(false)
                .chain((0..8).map(|bit| byte & (1 << bit) != 0))
                .chain([true])
            {
                for _ in 0..94 {
                    sio.tick(false, bit);
                }
            }
        }
        sio.tick(true, true);
    }
    fn response(sio: &mut Sio) -> Vec<u8> {
        let mut pokey = pokey::Pokey::new();
        pokey.write(4, 40);
        pokey.write(6, 0);
        pokey.write(8, 0x28);
        pokey.write(15, 0x13);
        pokey.write(14, 0x20);
        let mut bytes = Vec::new();
        for _ in 0..260000 {
            pokey.set_serial_input(sio.tick(true, true));
            pokey.tick();
            if pokey.irq() {
                bytes.push(pokey.read(13));
                pokey.write(14, 0);
                pokey.write(14, 0x20);
            }
        }
        bytes
    }
    #[test]
    fn real_serial_frames_return_sector_checksum_and_reject_bad_commands() {
        let mut disk = Atr::blank(720, 128).unwrap();
        for (i, byte) in disk.sector_mut(1).unwrap().iter_mut().enumerate() {
            *byte = i as u8;
        }
        let mut sio = Sio::new(1_789_772);
        sio.attach(&disk.to_bytes()).unwrap();
        command(&mut sio, 0x31, 0x52, 1, true);
        let mut expected = vec![b'A', b'C'];
        expected.extend(0..128);
        expected.push(checksum(&expected[2..]));
        assert_eq!(response(&mut sio), expected);
        for (id, sector, valid, expected) in [
            (0x31, 721, true, vec![b'N']),
            (0x32, 1, true, vec![]),
            (0x31, 1, false, vec![]),
        ] {
            command(&mut sio, id, 0x52, sector, valid);
            assert_eq!(response(&mut sio), expected);
        }
        command(&mut sio, 0x31, 0x21, 1, true);
        assert_eq!(response(&mut sio), vec![b'N']);
        assert_eq!(sio.export(), Some(disk.to_bytes()));
    }

    fn send_data(sio: &mut Sio, bytes: &[u8]) {
        for &byte in bytes {
            for bit in std::iter::once(false)
                .chain((0..8).map(|bit| byte & (1 << bit) != 0))
                .chain([true])
            {
                for _ in 0..94 {
                    sio.tick(true, bit);
                }
            }
        }
    }

    #[test]
    fn writes_use_command_ack_data_ack_checksum_and_completion_on_both_drives() {
        for (drive, code, sector) in [(0, 0x50, 1), (1, 0x57, 4)] {
            let disk = Atr::blank(720, 256).unwrap();
            let original = disk.to_bytes();
            let mut sio = Sio::new(1_789_772);
            sio.attach_in(0, &original).unwrap();
            sio.attach_in(1, &original).unwrap();
            command(&mut sio, 0x31 + drive, code, sector, true);
            assert_eq!(response(&mut sio), b"A");
            assert!(sio.busy());
            let mut bytes = vec![0x9b; if sector <= 3 { 128 } else { 256 }];
            bytes.push(checksum(&bytes));
            send_data(&mut sio, &bytes);
            assert_eq!(response(&mut sio), b"AC");
            assert!(sio.dirty_in(drive));
            assert!(!sio.dirty_in(1 - drive));
            assert_eq!(sio.export_in(1 - drive).unwrap(), original);
            let updated = Atr::parse(&sio.export_in(drive).unwrap()).unwrap();
            assert!(updated.sector(sector).unwrap().iter().all(|&n| n == 0x9b));
            assert!(!sio.busy());
            sio.mark_saved_in(drive).unwrap();
            assert!(!sio.dirty_in(drive));
            assert!(sio.attached_in(drive));
            assert_eq!(sio.extension_in(drive), Some("atr"));
            sio.detach_in(drive).unwrap();
            assert!(!sio.attached_in(drive));
        }
    }

    #[test]
    fn corrupt_write_naks_without_completion_and_protection_reports_error_without_mutation() {
        for protected in [false, true] {
            let mut image = Atr::blank(720, 128).unwrap().to_bytes();
            image[15] = u8::from(protected);
            let mut sio = Sio::new(1_789_772);
            sio.attach(&image).unwrap();
            command(&mut sio, 0x31, 0x57, 1, true);
            assert_eq!(response(&mut sio), b"A");
            let mut bytes = vec![0x42; 128];
            bytes.push(checksum(&bytes) ^ u8::from(!protected));
            send_data(&mut sio, &bytes);
            assert_eq!(
                response(&mut sio),
                if protected { &b"AE"[..] } else { &b"N"[..] }
            );
            assert_eq!(sio.export().unwrap(), image);
            assert!(!sio.dirty_in(0));
            if protected {
                command(&mut sio, 0x31, 0x53, 0, true);
                let status = response(&mut sio);
                assert_eq!(&status[..4], &[b'A', b'C', 0x1c, 0xbf]);
            }
        }
        let mut sio = Sio::new(1_789_772);
        sio.attach(&Atr::blank(720, 128).unwrap().to_bytes())
            .unwrap();
        command(&mut sio, 0x31, 0x50, 721, true);
        assert_eq!(response(&mut sio), b"N");
        assert!(!sio.busy());
        assert!(sio.attach_in(2, &[]).is_err());
        assert!(sio.detach_in(2).is_err());
        assert!(sio.mark_saved_in(2).is_err());
    }

    #[test]
    fn a_bad_stop_bit_naks_the_whole_data_frame_without_writing() {
        let image = Atr::blank(720, 128).unwrap().to_bytes();
        let mut sio = Sio::new(1_789_772);
        sio.attach(&image).unwrap();
        command(&mut sio, 0x31, 0x50, 1, true);
        assert_eq!(response(&mut sio), b"A");
        for _ in 0..940 {
            sio.tick(true, false);
        }
        for _ in 0..188 {
            sio.tick(true, true);
        }
        send_data(&mut sio, &[0; 128]); // remaining 127 zero bytes plus checksum
        assert_eq!(response(&mut sio), b"N");
        assert_eq!(sio.export().unwrap(), image);
        assert!(!sio.dirty_in(0));
    }

    #[test]
    fn write_extension_resumes_inside_data_bit_and_preserves_xfd_format_and_dirty_state() {
        let raw = Atr::blank(720, 256).unwrap().to_xfd(true);
        let mut a = Sio::new(1_789_772);
        a.attach_format(1, "XFD", &raw).unwrap();
        assert_eq!(a.export_in(1).unwrap(), raw);
        command(&mut a, 0x32, 0x57, 4, true);
        assert_eq!(response(&mut a), b"A");
        let mut bytes = vec![0x5a; 256];
        bytes.push(checksum(&bytes));
        send_data(&mut a, &bytes[..37]);
        for _ in 0..30 {
            a.tick(true, false);
        }
        let old_state = a.save();
        let extension = a.save_media();
        let config = bincode::config::standard();
        let original_wire = bincode::serde::encode_to_vec(&old_state, config).unwrap();
        let extension_wire = bincode::serde::encode_to_vec(&extension, config).unwrap();
        let (extension, used) = bincode::serde::decode_from_slice(&extension_wire, config).unwrap();
        assert_eq!(used, extension_wire.len());
        let mut b = Sio::restore(old_state, 1_789_772).unwrap();
        assert_eq!(
            bincode::serde::encode_to_vec(b.save(), config).unwrap(),
            original_wire
        );
        b.restore_media(extension).unwrap();
        for bit in std::iter::repeat_n(false, 64)
            .chain((0..8).flat_map(|bit| std::iter::repeat_n(0x5a & (1 << bit) != 0, 94)))
            .chain(std::iter::repeat_n(true, 94))
        {
            assert_eq!(a.tick(true, bit), b.tick(true, bit));
        }
        send_data(&mut a, &bytes[38..]);
        send_data(&mut b, &bytes[38..]);
        assert_eq!(response(&mut a), b"AC");
        assert_eq!(response(&mut b), b"AC");
        assert_eq!(a.export_in(1), b.export_in(1));
        assert!(a.dirty_in(1));
        assert_eq!(a.extension_in(1), Some("xfd"));
        let mut c = Sio::restore(a.save(), 1_789_772).unwrap();
        c.restore_media(a.save_media()).unwrap();
        assert_eq!(c.export_in(1), a.export_in(1));
        assert!(c.dirty_in(1));
    }

    #[test]
    fn extension_validation_is_atomic_and_new_command_aborts_partial_write() {
        let image = Atr::blank(720, 128).unwrap().to_bytes();
        let mut sio = Sio::new(1_789_772);
        sio.attach(&image).unwrap();
        command(&mut sio, 0x31, 0x57, 1, true);
        response(&mut sio);
        send_data(&mut sio, &[1, 2, 3]);
        for bad in [0, 1, 2] {
            let mut state = sio.save_media();
            match bad {
                0 => state.formats[0] = 3,
                1 => state.pending.as_mut().unwrap().drive = 2,
                _ => state.pending.as_mut().unwrap().bytes = vec![0; 129],
            }
            assert!(sio.restore_media(state).is_err());
            assert_eq!(sio.media.pending.as_ref().unwrap().bytes, [1, 2, 3]);
            assert_eq!(sio.export().unwrap(), image);
        }
        command(&mut sio, 0x31, 0x52, 1, true);
        assert_eq!(&response(&mut sio)[..2], b"AC");
        assert!(!sio.dirty_in(0));
        assert_eq!(sio.export().unwrap(), image);
    }
}
