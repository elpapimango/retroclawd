//! The disc drive behind the CDD port: answers the BIOS's commands from the disc's table of contents and hands the CD
//! controller one sector per 1/75 s while it reads (facts E4-E6, E14 in `docs/hardware-facts.md`).
//!
//! Playing an audio track hands 588 stereo frames of the sector under the head to the mixer each period (fact E17).

use disc::{Disc, RAW_SECTOR, Track};
use mcd_gate_array::cdc::Cdc;
use mcd_gate_array::cdd::{Drive, NoDisc, command, status};

/// Sectors the lead-in takes before the first sector of the program area: absolute time is LBA + 150 (fact D1).
const LEAD_IN: u32 = 150;

/// What the drive is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// A disc is in, the motor is off.
    Stopped,
    /// Moving to `lba`; `then` says what follows.
    Seeking {
        periods_left: u8,
        then: After,
    },
    /// Reading data sectors, one a period.
    Reading,
    /// Playing audio: the sector under the head goes to the mixer.
    Playing,
    Paused,
    Scanning,
    TrayOpen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum After {
    Read,
    Pause,
    Play,
}

pub struct DiscDrive {
    disc: Disc,
    tracks: Vec<Track>,
    lead_out: u32,
    mode: Mode,
    /// The sector the head is at.
    lba: i32,
    /// What the status packets report besides the status (the TC3 of the last report request, fact E4).
    report: u8,
    /// Periods of "playing" status the drive shows after a read starts before the first sector arrives.
    start_delay: u8,
    /// The last status values RS0-RS8 sent; RS1-RS8 stay as they were while the drive is stopped (fact E15).
    last: [u8; 9],
    /// Audio played and not yet taken.
    audio: Vec<i16>,
}

/// The layout of a drive's saved state: bump when a field is added or changes meaning.
const STATE_VERSION: u8 = 1;
/// The most audio a drive can hold unread (a second of stereo samples, well above what a frame produces).
const MAX_AUDIO_SAMPLES: usize = 44_100 * 2 * 4;

impl DiscDrive {
    /// A number that identifies the disc's layout, so a state of another disc is refused (a CRC of the track table).
    fn layout_id(&self) -> u32 {
        let mut hash = crc32fast::Hasher::new();
        for t in &self.tracks {
            for v in [u32::from(t.number), t.start_lba, t.pregap_lba, t.sectors] {
                hash.update(&v.to_le_bytes());
            }
        }
        hash.update(&self.lead_out.to_le_bytes());
        hash.finalize()
    }

    pub fn new(disc: Disc) -> Self {
        // a track with no sectors in the image (a truncated copy) is not on the disc
        let tracks: Vec<Track> = disc
            .tracks()
            .iter()
            .copied()
            .filter(|t| t.sectors > 0)
            .collect();
        let lead_out = disc.lead_out();
        Self {
            disc,
            tracks,
            lead_out,
            mode: Mode::Stopped,
            lba: 0,
            report: 0,
            start_delay: 0,
            last: [0; 9],
            audio: Vec::new(),
        }
    }

    fn status_nibble(&self) -> u8 {
        match self.mode {
            Mode::Stopped => status::TOC_READ,
            Mode::Seeking { .. } => status::SEEK,
            Mode::Reading | Mode::Playing => status::PLAY,
            Mode::Paused => status::PAUSE,
            Mode::Scanning => status::SCAN,
            Mode::TrayOpen => status::DOOR_OPEN,
        }
    }

    /// The track under the head; a head in the lead-in is on the first track.
    /// What the drive does when it plays from the head: an audio track is played, a data track is read (fact E17).
    fn playing_mode(&self) -> Mode {
        if self.on_audio_track() {
            Mode::Playing
        } else {
            Mode::Reading
        }
    }

    fn on_audio_track(&self) -> bool {
        self.track_of(self.lba)
            .is_some_and(|t| t.kind == disc::cue::TrackKind::Audio)
    }

    fn track_of(&self, lba: i32) -> Option<&Track> {
        self.tracks
            .iter()
            .rev()
            .find(|t| lba >= t.start_lba as i32)
            .or_else(|| self.tracks.first())
    }

    /// Minutes, seconds, frames of the absolute time of `lba` (which may lie in the lead-in, before sector 0).
    fn msf_of(lba: i32) -> (u8, u8, u8) {
        let abs = (lba + LEAD_IN as i32).max(0) as u32;
        (
            (abs / 4500).min(99) as u8,
            (abs / 75 % 60) as u8,
            (abs % 75) as u8,
        )
    }

    fn digits(value: u32) -> (u8, u8) {
        let v = value.min(99);
        ((v / 10) as u8, (v % 10) as u8)
    }

    /// The minutes, seconds and frames as six values (fact E4).
    fn msf_digits(m: u8, s: u8, f: u8) -> [u8; 6] {
        let (m1, m2) = Self::digits(u32::from(m));
        let (s1, s2) = Self::digits(u32::from(s));
        let (f1, f2) = Self::digits(u32::from(f));
        [m1, m2, s1, s2, f1, f2]
    }

    /// The BCD time of a command (TC2-TC7) as a sector number of the program area.
    fn lba_of(tc: &[u8; 10]) -> i32 {
        let m = u32::from(tc[2]) * 10 + u32::from(tc[3]);
        let s = u32::from(tc[4]) * 10 + u32::from(tc[5]);
        let f = u32::from(tc[6]) * 10 + u32::from(tc[7]);
        ((m * 60 + s) * 75 + f) as i32 - LEAD_IN as i32
    }

    fn reply(&self, tc: &[u8; 10]) -> [u8; 9] {
        let mut rs = [0u8; 9];
        rs[0] = self.status_nibble();
        rs[1] = self.report;
        let head = self.lba;
        // RS8 carries the flags of the block under the head: bit 2 is set on a data track (fact E15)
        let flags = if self
            .track_of(head)
            .is_some_and(|t| t.kind != disc::cue::TrackKind::Audio)
        {
            4
        } else {
            0
        };
        match self.report {
            // absolute time of the head
            0 => {
                let (m, s, f) = Self::msf_of(head);
                rs[2..8].copy_from_slice(&Self::msf_digits(m, s, f));
                rs[8] = flags;
            }
            // time from the start of the current track
            1 => {
                let start = self.track_of(head).map_or(0, |t| t.start_lba as i32);
                let rel = (head - start).unsigned_abs();
                let (m, s, f) = ((rel / 4500) as u8, (rel / 75 % 60) as u8, (rel % 75) as u8);
                rs[2..8].copy_from_slice(&Self::msf_digits(m, s, f));
                rs[8] = flags;
            }
            // the current track
            2 => {
                let n = self.track_of(head).map_or(1, |t| u32::from(t.number));
                let (a, b) = Self::digits(n);
                rs[2] = a;
                rs[3] = b;
            }
            // the total length of the disc
            3 => {
                let (m, s, f) = Disc::msf(self.lead_out);
                rs[2..8].copy_from_slice(&Self::msf_digits(m, s, f));
            }
            // the first and the last track
            4 => {
                let first = self.tracks.first().map_or(1, |t| u32::from(t.number));
                let last = self.tracks.last().map_or(1, |t| u32::from(t.number));
                let ((a, b), (c, d)) = (Self::digits(first), Self::digits(last));
                rs[2..6].copy_from_slice(&[a, b, c, d]);
            }
            // the start of track N (TC4, TC5), flagged as a data track in RS6 bit 3
            5 => {
                let n = u32::from(tc[4]) * 10 + u32::from(tc[5]);
                if let Some(t) = self.tracks.iter().find(|t| u32::from(t.number) == n) {
                    let (m, s, f) = Disc::msf(t.start_lba);
                    rs[2..8].copy_from_slice(&Self::msf_digits(m, s, f));
                    if t.kind != disc::cue::TrackKind::Audio {
                        rs[6] |= 8;
                    }
                    rs[8] = (n & 0xF) as u8;
                }
            }
            _ => {}
        }
        rs
    }

    fn seek(&mut self, lba: i32, then: After) {
        self.lba = lba.clamp(-(LEAD_IN as i32), self.lead_out as i32 - 1);
        self.mode = Mode::Seeking {
            periods_left: 12,
            then,
        };
    }
}

impl Drive for DiscDrive {
    fn command(&mut self, tc: &[u8; 10]) -> [u8; 9] {
        match tc[0] {
            command::NOP => {}
            command::STOP => {
                self.mode = Mode::Stopped;
                // the first answer to a stop shows the motor-off status zero and "track information not valid" (fact E15)
                self.last = [status::STOP, 0xF, 0, 0, 0, 0, 0, 0, 0];
                return self.last;
            }
            command::REPORT_REQUEST => {
                self.report = tc[3];
                if (3..=5).contains(&tc[3]) {
                    // table of contents questions are answered whatever the drive is doing
                    let mut rs = self.reply(tc);
                    rs[0] = self.status_nibble();
                    return rs;
                }
            }
            command::READ => {
                self.report = 0;
                let lba = Self::lba_of(tc);
                self.seek(lba, After::Read);
                self.start_delay = 2;
                self.last = [status::SEEK, 0xF, 0, 0, 0, 0, 0, 0, 0];
                return self.last;
            }
            command::SEEK => {
                self.report = 0;
                let lba = Self::lba_of(tc);
                self.seek(lba, After::Pause);
                self.last = [status::SEEK, 0xF, 0, 0, 0, 0, 0, 0, 0];
                return self.last;
            }
            command::PAUSE => self.mode = Mode::Paused,
            command::PLAY => {
                let lba = Self::lba_of(tc);
                if tc[2..8].iter().any(|n| *n != 0) {
                    self.seek(lba, After::Play);
                } else {
                    self.mode = self.playing_mode();
                }
            }
            command::FORWARD | command::REVERSE => self.mode = Mode::Scanning,
            command::TRACK_SKIP | command::TRACK_CUE => {
                let n = u32::from(tc[4]) * 10 + u32::from(tc[5]);
                if let Some(t) = self.tracks.iter().find(|t| u32::from(t.number) == n) {
                    let lba = t.start_lba as i32;
                    self.seek(lba, After::Pause);
                }
            }
            command::DOOR_OPEN => self.mode = Mode::TrayOpen,
            command::DOOR_CLOSE => self.mode = Mode::Stopped,
            _ => {
                let mut rs = [0; 9];
                rs[0] = status::COMMAND_ERROR;
                return rs;
            }
        }
        // the status shows at once; the rest follows the report type while the drive is doing something with the disc
        let nibble = self.status_nibble();
        let mut rs = self.last;
        rs[0] = nibble;
        if matches!(
            nibble,
            status::PLAY | status::SEEK | status::SCAN | status::PAUSE
        ) {
            let fresh = self.reply(tc);
            rs[1..].copy_from_slice(&fresh[1..]);
            if matches!(self.mode, Mode::Seeking { .. }) {
                rs[1] = 0xF; // track information is not valid while seeking
            }
        }
        self.last = rs;
        rs
    }

    fn period(&mut self, cdc: &mut Cdc) {
        match self.mode {
            Mode::Seeking { periods_left, then } => {
                if periods_left > 1 {
                    self.mode = Mode::Seeking {
                        periods_left: periods_left - 1,
                        then,
                    };
                } else {
                    self.mode = match then {
                        After::Read | After::Play => self.playing_mode(),
                        After::Pause => Mode::Paused,
                    };
                }
            }
            Mode::Reading if self.start_delay > 0 => self.start_delay -= 1,
            Mode::Reading => {
                let mut raw = [0u8; RAW_SECTOR];
                // the sectors before the program area (the lead-in) are not on the image: the drive still passes them under the
                // head, so the controller sees their headers (software seeks a few sectors early and waits for the time it wants)
                if self.lba < 0 || self.disc.read_raw(self.lba as u32, &mut raw).is_err() {
                    raw = [0; RAW_SECTOR];
                    raw[1..11].fill(0xFF);
                    raw[15] = 1;
                }
                // the header carries the sector's own address (the drive reports it, whatever the image holds)
                let (m, sec, f) = Self::msf_of(self.lba);
                let bcd = |v: u8| ((v / 10) << 4) | (v % 10);
                raw[12] = bcd(m);
                raw[13] = bcd(sec);
                raw[14] = bcd(f);
                cdc.deliver_sector(&raw);
                self.lba += 1;
                if self.lba >= self.lead_out as i32 {
                    self.mode = Mode::Paused;
                }
            }
            Mode::Playing | Mode::Scanning => {
                if self.mode == Mode::Playing && self.lba >= 0 && self.on_audio_track() {
                    let mut raw = [0u8; RAW_SECTOR];
                    if self.disc.read_raw(self.lba as u32, &mut raw).is_ok() {
                        self.audio.extend(
                            raw.as_chunks::<2>()
                                .0
                                .iter()
                                .map(|b| i16::from_le_bytes(*b)),
                        );
                    }
                }
                self.lba = (self.lba + 1).min(self.lead_out as i32 - 1);
            }
            _ => {}
        }
    }

    fn music_playing(&self) -> bool {
        matches!(self.mode, Mode::Playing | Mode::Scanning) && self.on_audio_track()
    }

    fn take_audio(&mut self, out: &mut Vec<i16>) {
        out.append(&mut self.audio);
    }

    fn state(&self) -> Vec<u8> {
        let mut out = vec![STATE_VERSION];
        out.extend_from_slice(&self.layout_id().to_le_bytes());
        match self.mode {
            Mode::Stopped => out.extend_from_slice(&[0, 0, 0]),
            Mode::Seeking { periods_left, then } => out.extend_from_slice(&[
                1,
                periods_left,
                match then {
                    After::Read => 0,
                    After::Pause => 1,
                    After::Play => 2,
                },
            ]),
            Mode::Reading => out.extend_from_slice(&[2, 0, 0]),
            Mode::Playing => out.extend_from_slice(&[3, 0, 0]),
            Mode::Paused => out.extend_from_slice(&[4, 0, 0]),
            Mode::Scanning => out.extend_from_slice(&[5, 0, 0]),
            Mode::TrayOpen => out.extend_from_slice(&[6, 0, 0]),
        }
        out.extend_from_slice(&self.lba.to_le_bytes());
        out.push(self.report);
        out.push(self.start_delay);
        out.extend_from_slice(&self.last);
        out.extend_from_slice(&(self.audio.len() as u32).to_le_bytes());
        for sample in &self.audio {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }

    fn restore(&mut self, state: &[u8]) -> Result<(), String> {
        let bad = || "the state is not this disc drive's".to_string();
        let (&version, rest) = state.split_first().ok_or_else(bad)?;
        if version != STATE_VERSION {
            return Err(bad());
        }
        let (id, rest) = rest.split_at_checked(4).ok_or_else(bad)?;
        if u32::from_le_bytes(id.try_into().unwrap()) != self.layout_id() {
            return Err("the state was saved with another disc".to_string());
        }
        let (head, rest) = rest.split_at_checked(3 + 4 + 2 + 9 + 4).ok_or_else(bad)?;
        let mode = match (head[0], head[1], head[2]) {
            (0, 0, 0) => Mode::Stopped,
            (1, periods_left, then) if then <= 2 => Mode::Seeking {
                periods_left,
                then: [After::Read, After::Pause, After::Play][usize::from(then)],
            },
            (2, 0, 0) => Mode::Reading,
            (3, 0, 0) => Mode::Playing,
            (4, 0, 0) => Mode::Paused,
            (5, 0, 0) => Mode::Scanning,
            (6, 0, 0) => Mode::TrayOpen,
            _ => return Err(bad()),
        };
        let lba = i32::from_le_bytes(head[3..7].try_into().unwrap());
        let span = i64::from(self.lead_out) + i64::from(LEAD_IN) + 1;
        if i64::from(lba) < -i64::from(LEAD_IN) - 1 || i64::from(lba) > span {
            return Err("the state has the head off the disc".to_string());
        }
        let (report, start_delay) = (head[7], head[8]);
        let last: [u8; 9] = head[9..18].try_into().unwrap();
        let count = u32::from_le_bytes(head[18..22].try_into().unwrap()) as usize;
        if count > MAX_AUDIO_SAMPLES || rest.len() != count * 2 {
            return Err(bad());
        }
        self.mode = mode;
        self.lba = lba;
        self.report = report;
        self.start_delay = start_delay;
        self.last = last;
        self.audio = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        Ok(())
    }

    fn box_clone(&self) -> Box<dyn Drive> {
        // a disc cannot be duplicated (it holds open files): a copy of the machine's state has an empty drive
        Box::new(NoDisc::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use disc::builder::{self, BuildTrack, Fill};
    use disc::cue::TrackKind;

    fn drive() -> (DiscDrive, std::path::PathBuf) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("segacd-drive-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cue = builder::write_disc(
            &dir,
            "d",
            &[
                BuildTrack {
                    kind: TrackKind::Mode1Raw,
                    sectors: 300,
                    fill: Fill::Pattern,
                },
                BuildTrack {
                    kind: TrackKind::Audio,
                    sectors: 150,
                    fill: Fill::Byte(1),
                },
            ],
        )
        .unwrap();
        (DiscDrive::new(Disc::open(&cue).unwrap()), dir)
    }

    /// A command packet: the command value TC0 and the values at the given positions.
    fn cmd(c: u8, set: &[(usize, u8)]) -> [u8; 10] {
        let mut tc = [0; 10];
        tc[0] = c;
        for (i, v) in set {
            tc[*i] = *v;
        }
        tc
    }

    #[test]
    fn a_drives_state_carries_its_position_and_only_fits_the_same_disc() {
        let (mut d, _) = drive();
        d.command(&cmd(command::NOP, &[]));
        // seek to 00:04:00 then play from there: the head moves and the mode changes
        d.command(&cmd(
            command::READ,
            &[(2, 0), (3, 0), (4, 0), (5, 4), (6, 0), (7, 0)],
        ));
        let mut cdc = Cdc::new();
        for _ in 0..20 {
            d.period(&mut cdc);
        }
        let state = d.state();
        let (mut fresh, _) = drive();
        fresh.restore(&state).unwrap();
        assert_eq!(
            fresh.state(),
            state,
            "the restored drive saves what it was given"
        );
        for _ in 0..30 {
            d.period(&mut cdc);
            fresh.period(&mut cdc);
        }
        assert_eq!(fresh.state(), d.state(), "and goes on the same way");

        // another disc (a different track table) is refused, and nothing changes
        let dir = std::env::temp_dir().join(format!("segacd-drive-other-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let other = builder::write_disc(
            &dir,
            "o",
            &[BuildTrack {
                kind: TrackKind::Mode1Raw,
                sectors: 100,
                fill: Fill::Pattern,
            }],
        )
        .unwrap();
        let mut other = DiscDrive::new(Disc::open(&other).unwrap());
        let before = other.state();
        assert!(other.restore(&state).unwrap_err().contains("another disc"));
        assert_eq!(other.state(), before);
    }

    #[test]
    fn a_damaged_drive_state_is_refused_without_a_change() {
        let (d, _) = drive();
        let good = d.state();
        let (mut target, _) = drive();
        let before = target.state();
        for len in 0..good.len() {
            assert!(target.restore(&good[..len]).is_err(), "cut at {len}");
        }
        let mut junk = good.clone();
        junk[5] = 99; // no such mode
        assert!(target.restore(&junk).is_err());
        let mut far = good.clone();
        far[9..13].copy_from_slice(&i32::MAX.to_le_bytes()); // the head far off the disc
        assert!(target.restore(&far).is_err());
        assert_eq!(target.state(), before);
    }

    #[test]
    fn a_new_disc_shows_the_table_of_contents_status_until_asked_to_do_something() {
        let (mut d, _) = drive();
        let rs = d.command(&cmd(command::NOP, &[]));
        assert_eq!(rs[0], status::TOC_READ);
        // the first answer to a stop is status zero with the track information marked invalid, then stopped again
        let rs = d.command(&cmd(command::STOP, &[]));
        assert_eq!((rs[0], rs[1]), (status::STOP, 0xF));
        let rs = d.command(&cmd(command::NOP, &[]));
        assert_eq!(
            (rs[0], rs[1]),
            (status::TOC_READ, 0xF),
            "the rest of the packet stays"
        );
    }

    #[test]
    fn the_report_requests_describe_the_disc() {
        let (mut d, _) = drive();
        // TC3 = 4: first and last track
        let rs = d.command(&cmd(command::REPORT_REQUEST, &[(3, 4)]));
        assert_eq!(rs[1], 4);
        assert_eq!(rs[2..6], [0, 1, 0, 2]);
        // TC3 = 3: the end of the disc, 450 sectors + 150 = 600 frames = 00:08:00
        let rs = d.command(&cmd(command::REPORT_REQUEST, &[(3, 3)]));
        assert_eq!(rs[2..8], [0, 0, 0, 8, 0, 0]);
        // TC3 = 5: where track 1 (a data track) and track 2 (audio) start
        let rs = d.command(&cmd(command::REPORT_REQUEST, &[(3, 5), (5, 1)]));
        assert_eq!(
            (rs[2..8].to_vec(), rs[8]),
            (vec![0, 0, 0, 2, 8, 0], 1),
            "00:02:00, flagged as data"
        );
        let rs = d.command(&cmd(command::REPORT_REQUEST, &[(3, 5), (5, 2)]));
        assert_eq!(
            (rs[2..8].to_vec(), rs[8]),
            (vec![0, 0, 0, 6, 0, 0], 2),
            "300 + 150 frames = 00:06:00, audio"
        );
    }

    #[test]
    fn a_seek_takes_a_few_periods_and_ends_paused_at_the_asked_time() {
        let (mut d, _) = drive();
        let mut cdc = Cdc::new();
        // seek to 00:02:10: ten sectors into the disc
        let rs = d.command(&cmd(command::SEEK, &[(5, 2), (6, 1)]));
        assert_eq!((rs[0], rs[1]), (status::SEEK, 0xF));
        for _ in 0..12 {
            d.period(&mut cdc);
        }
        let rs = d.command(&cmd(command::REPORT_REQUEST, &[(3, 0)]));
        assert_eq!(rs[0], status::PAUSE);
        assert_eq!(rs[2..8], [0, 0, 0, 2, 1, 0], "absolute time 00:02:10");
        assert_eq!(rs[8], 4, "a data track");
    }

    #[test]
    fn a_read_starts_after_a_short_delay_and_hands_one_sector_a_period_to_the_controller() {
        let (mut d, _) = drive();
        let mut cdc = Cdc::new();
        cdc.write_address(1);
        cdc.write(0x20); // DECIEN
        cdc.write_address(10);
        cdc.write(0x84); // DECEN, WRRQ
        // read from 00:02:05, five sectors in
        let rs = d.command(&cmd(command::READ, &[(5, 2), (7, 5)]));
        assert_eq!(rs[0], status::SEEK);
        let mut delivered = 0;
        for _ in 0..24 {
            d.period(&mut cdc);
            if cdc.int_edge() {
                delivered += 1;
                cdc.write_address(15);
                cdc.read();
                cdc.int_edge(); // the gate array watches the line after every access
                if delivered == 1 {
                    // the header of the first sector: 00:02:05 in mode 1
                    cdc.write_address(4);
                    assert_eq!(
                        [cdc.read(), cdc.read(), cdc.read(), cdc.read()],
                        [Some(0), Some(2), Some(5), Some(1)]
                    );
                }
            }
        }
        assert!((5..=12).contains(&delivered), "{delivered}");
        let rs = d.command(&cmd(command::NOP, &[]));
        assert_eq!(rs[0], status::PLAY);
    }

    #[test]
    fn playing_an_audio_track_hands_588_stereo_frames_a_period_to_the_mixer() {
        let (mut d, _) = drive();
        let mut cdc = Cdc::new();
        // track 2 starts at sector 300 of the program area: absolute time 00:06:00
        d.command(&cmd(command::PLAY, &[(5, 6)]));
        let mut audio = Vec::new();
        for _ in 0..40 {
            d.period(&mut cdc);
            d.take_audio(&mut audio);
        }
        assert!(d.music_playing());
        // twelve periods of seeking, then every period one sector of 588 frames; the track is filled with 0x01 bytes
        assert!(audio.len() >= 2 * 588 * 20, "{}", audio.len());
        assert_eq!(audio.len() % (2 * 588), 0, "whole sectors only");
        assert!(audio.iter().all(|s| *s == 0x0101));
        d.command(&cmd(command::PAUSE, &[]));
        let before = audio.len();
        d.period(&mut cdc);
        d.take_audio(&mut audio);
        assert_eq!(audio.len(), before, "a paused drive plays nothing");
    }

    #[test]
    fn the_read_command_plays_an_audio_track_and_reads_a_data_track() {
        let (mut d, _) = drive();
        let mut cdc = Cdc::new();
        let mut audio = Vec::new();
        d.command(&cmd(command::READ, &[(5, 6)])); // 00:06:00: the audio track
        for _ in 0..30 {
            d.period(&mut cdc);
            d.take_audio(&mut audio);
        }
        assert!(d.music_playing());
        assert!(!audio.is_empty());
        let rs = d.command(&cmd(command::NOP, &[]));
        assert_eq!(rs[0], status::PLAY);
    }

    #[test]
    fn playing_a_data_track_makes_no_audio() {
        let (mut d, _) = drive();
        let mut cdc = Cdc::new();
        d.command(&cmd(command::PLAY, &[(5, 2)])); // 00:02:00: the data track
        let mut audio = Vec::new();
        for _ in 0..30 {
            d.period(&mut cdc);
            d.take_audio(&mut audio);
        }
        assert!(audio.is_empty());
        assert!(!d.music_playing());
    }
}
