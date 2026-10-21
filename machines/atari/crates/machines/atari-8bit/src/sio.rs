//! Read-only D1: SIO peripheral, not an emulated 810 CPU/FDC.
//! Altirra Hardware Reference 2026-01-02, sections 9.1 and 10.2.
//! Commands travel on actual PIA COMMAND and POKEY serial data pins.
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
    state: SioState,
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
        let disk = Atr::parse(bytes)?;
        self.disk = Some(disk);
        self.abort();
        self.state.reads = 0;
        self.state.sector = 0;
        self.state.write_error = false;
        Ok(())
    }
    pub fn detach(&mut self) {
        self.disk = None;
        self.abort();
    }
    pub fn attached(&self) -> bool {
        self.disk.is_some()
    }
    pub fn export(&self) -> Option<Vec<u8>> {
        self.disk.as_ref().map(Atr::to_bytes)
    }
    pub fn reads(&self) -> u64 {
        self.state.reads
    }
    pub fn sector(&self) -> u16 {
        self.state.sector
    }
    pub fn busy(&self) -> bool {
        !self.state.command || self.state.transmit_bits != 0 || !self.state.queue.is_empty()
    }
    pub fn save(&self) -> SioState {
        let mut state = self.state.clone();
        state.image = self.export();
        state
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
        Ok(Self { disk, state })
    }
    fn abort(&mut self) {
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
        if s.frame.len() != 5 || s.frame[0] != 0x31 || checksum(&s.frame[..4]) != s.frame[4] {
            return;
        }
        let Some(disk) = &self.disk else { return };
        let sector = u16::from_le_bytes([s.frame[2], s.frame[3]]);
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
                    | if s.write_error { 8 } else { 0 },
                0xff,
                0xe0,
                0,
            ]),
            0x50 | 0x57 | 0x21 | 0x22 => {
                s.write_error = true;
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
    fn receive(&mut self, input: bool) {
        let s = &mut self.state;
        if s.receive_bits == 0 {
            if input {
                return;
            }
            s.receive_bits = 1;
            s.receive_delay = s.bit_cycles / 2;
        }
        s.receive_delay = s.receive_delay.saturating_sub(1);
        if s.receive_delay != 0 {
            return;
        }
        match s.receive_bits {
            1 => {
                if input {
                    s.receive_bits = 0;
                    return;
                }
                s.receive_shift = 0;
            }
            2..=9 => s.receive_shift |= u8::from(input) << (s.receive_bits - 2),
            10 => {
                if input && s.frame.len() < 6 {
                    s.frame.push(s.receive_shift);
                } else {
                    s.frame.resize(6, 0);
                }
                s.receive_bits = 0;
                return;
            }
            _ => unreachable!(),
        }
        s.receive_bits += 1;
        s.receive_delay = s.bit_cycles;
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
            self.receive(data);
        } else {
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
        command(&mut sio, 0x31, 0x57, 1, true);
        assert_eq!(response(&mut sio), vec![b'N']);
        assert_eq!(sio.export(), Some(disk.to_bytes()));
    }
}
