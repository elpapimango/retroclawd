//! Creating a CHD version 5 file: data goes in as bytes, comes out as hunks each stored with the best of the chosen codecs (or as it is),
//! repeated hunks become references, hunks the parent holds become parent references, and the map, the metadata and the header
//! with its SHA-1s are written when the data ends.
//!
//! A file with no codecs is written the plain way: the header, the map of one 32-bit position per hunk, the hunks each aligned to the
//! hunk size, an all-zero hunk (or, with a parent, one equal to the parent's) left out.

use crate::codec::{self, Codec};
use crate::image::{Header, Image, MAX_HUNK_BYTES, V5_HEADER_LEN};
use crate::map::{Entry, Kind, MAX_HUNKS};
use crate::meta::{self, MetaEntry};
use crate::{ChdError, cd, crc16, mapenc};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// How many hunks are compressed together (in parallel) before they are written.
const BATCH_PER_THREAD: usize = 16;

#[derive(Clone, Debug)]
pub struct CreateOptions {
    pub hunk_bytes: u32,
    pub unit_bytes: u32,
    /// Up to four codecs in slot order; empty writes an uncompressed file.
    pub codecs: Vec<Codec>,
    /// The size of the image in bytes.
    pub logical_bytes: u64,
    pub metadata: Vec<MetaEntry>,
    /// Replace the output if it exists.
    pub overwrite: bool,
    /// Compression threads (0: as many as the machine has).
    pub threads: usize,
}

/// What was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub hunks: u32,
    pub file_bytes: u64,
    /// Hunks stored with each codec slot, as they are, as references to another hunk, and as parent references.
    pub per_codec: [u32; 4],
    pub plain: u32,
    pub self_refs: u32,
    pub parent_refs: u32,
    pub raw_sha1: [u8; 20],
    pub sha1: [u8; 20],
}

/// A hunk after the parallel stage.
struct Prepared {
    bytes: Vec<u8>,
    sha1: [u8; 20],
    crc: u16,
    /// The best codec's slot and output, if any beat the hunk's size.
    packed: Option<(u8, Vec<u8>)>,
}

pub struct Writer {
    out: BufWriter<File>,
    path: PathBuf,
    opts: CreateOptions,
    parent: Option<ParentIndex>,
    /// Bytes of the next hunk, collecting.
    pending: Vec<u8>,
    batch: Vec<Vec<u8>>,
    batch_size: usize,
    threads: usize,
    pos: u64,
    pushed: u64,
    hunks_done: u32,
    hunk_total: u32,
    entries: Vec<Entry>,
    /// Uncompressed files: the stored position of each hunk divided by the hunk size (0: not stored).
    plain_map: Vec<u32>,
    seen: HashMap<[u8; 20], u32>,
    /// An uncompressed file: where its metadata chain starts and the entries (written when the file was started).
    plain_meta: Option<(u64, Vec<MetaEntry>)>,
    raw: Sha1,
    summary: Summary,
    finished: bool,
}

/// The parent's hunks by hash, and its overall SHA-1 for the header.
struct ParentIndex {
    sha1: [u8; 20],
    by_hash: HashMap<[u8; 20], u32>,
    by_position: Vec<[u8; 20]>,
    unit_bytes: u32,
}

fn io(e: std::io::Error) -> ChdError {
    ChdError::Io(format!("writing the CHD file: {e}"))
}

fn config<T>(why: impl Into<String>) -> Result<T, ChdError> {
    Err(ChdError::Format(why.into()))
}

fn sha1_of(data: &[u8]) -> [u8; 20] {
    Sha1::digest(data).into()
}

/// Places the entries of a chain one after another from file offset `at`, each pointing at the next; returns the bytes of the chain and
/// the offset of its first entry (0 for an empty chain).
fn lay_out_chain(entries: &mut [MetaEntry], at: u64) -> (Vec<u8>, u64) {
    let mut here = at;
    let offsets: Vec<u64> = entries
        .iter()
        .map(|e| {
            let o = here;
            here += 16 + e.data.len() as u64;
            o
        })
        .collect();
    let mut bytes = Vec::new();
    for (i, e) in entries.iter_mut().enumerate() {
        e.offset = offsets[i];
        e.next = offsets.get(i + 1).copied().unwrap_or(0);
        bytes.extend(meta::entry_bytes(e, e.next));
    }
    (bytes, offsets.first().copied().unwrap_or(0))
}

impl ParentIndex {
    fn build(mut parent: Image, hunk_bytes: u32) -> Result<ParentIndex, ChdError> {
        let h = parent.header().clone();
        // the child's hunks are compared with the parent's data cut at the child's hunk size
        let total = h.logical_bytes.div_ceil(u64::from(hunk_bytes));
        let mut by_hash = HashMap::new();
        let mut by_position = Vec::new();
        for k in 0..total {
            let bytes = parent.read_bytes(k * u64::from(hunk_bytes), hunk_bytes as usize)?;
            let digest = sha1_of(&bytes);
            by_hash.entry(digest).or_insert(k as u32);
            by_position.push(digest);
        }
        Ok(ParentIndex {
            sha1: h.sha1,
            by_hash,
            by_position,
            unit_bytes: h.unit_bytes,
        })
    }
}

impl Writer {
    /// Starts a file. `parent`, if any, is the file this one is a delta of (the data pushed is the whole image; the hunks the parent
    /// holds are not stored again).
    pub fn create(
        path: &Path,
        opts: CreateOptions,
        parent: Option<Image>,
    ) -> Result<Writer, ChdError> {
        if opts.hunk_bytes == 0 || opts.hunk_bytes > MAX_HUNK_BYTES {
            return config("the hunk size is out of range");
        }
        if opts.unit_bytes == 0 || !opts.hunk_bytes.is_multiple_of(opts.unit_bytes) {
            return config("the hunk size must be a whole number of units");
        }
        if opts.codecs.len() > 4 {
            return config("a CHD file names at most four codecs");
        }
        for (i, c) in opts.codecs.iter().enumerate() {
            if opts.codecs[..i].contains(c) {
                return config(format!(
                    "the codec {} is named twice",
                    String::from_utf8_lossy(&c.tag())
                ));
            }
            if c.is_cd() && !(opts.hunk_bytes as usize).is_multiple_of(cd::FRAME_SIZE) {
                return config(format!(
                    "the codec {} needs a hunk size that is a multiple of {} bytes",
                    String::from_utf8_lossy(&c.tag()),
                    cd::FRAME_SIZE
                ));
            }
        }
        let hunk_total = opts.logical_bytes.div_ceil(u64::from(opts.hunk_bytes));
        if hunk_total == 0 || hunk_total > u64::from(MAX_HUNKS) {
            return config("the image is empty or has too many hunks");
        }
        let parent = match parent {
            Some(p) => {
                if p.header().unit_bytes != opts.unit_bytes {
                    return config("the parent's unit size differs");
                }
                Some(ParentIndex::build(p, opts.hunk_bytes)?)
            }
            None => None,
        };
        let mut open = OpenOptions::new();
        open.write(true);
        if opts.overwrite {
            open.create(true).truncate(true);
        } else {
            open.create_new(true);
        }
        let file = open.open(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                ChdError::Io(format!(
                    "{}: the file exists (overwrite is off)",
                    path.display()
                ))
            } else {
                ChdError::Io(format!("{}: {e}", path.display()))
            }
        })?;
        let threads = if opts.threads == 0 {
            std::thread::available_parallelism().map_or(1, |n| n.get())
        } else {
            opts.threads
        };
        let mut w = Writer {
            out: BufWriter::with_capacity(1 << 20, file),
            path: path.to_path_buf(),
            parent,
            pending: Vec::new(),
            batch: Vec::new(),
            batch_size: BATCH_PER_THREAD * threads,
            threads,
            pos: 0,
            pushed: 0,
            hunks_done: 0,
            hunk_total: hunk_total as u32,
            entries: Vec::new(),
            plain_map: Vec::new(),
            seen: HashMap::new(),
            plain_meta: None,
            raw: Sha1::new(),
            summary: Summary {
                hunks: hunk_total as u32,
                file_bytes: 0,
                per_codec: [0; 4],
                plain: 0,
                self_refs: 0,
                parent_refs: 0,
                raw_sha1: [0; 20],
                sha1: [0; 20],
            },
            finished: false,
            opts,
        };
        w.out
            .write_all(&[0u8; V5_HEADER_LEN as usize])
            .map_err(io)?;
        w.pos = u64::from(V5_HEADER_LEN);
        if !w.compressed() {
            // the map's room is reserved and the metadata goes right after it; the hunks start on the next multiple of the hunk size
            let map_bytes = u64::from(w.hunk_total) * 4;
            let mut entries = std::mem::take(&mut w.opts.metadata);
            let (chain, first) = lay_out_chain(&mut entries, w.pos + map_bytes);
            let end = w.pos + map_bytes + chain.len() as u64;
            let zeros = vec![0u8; 1 << 16];
            let write_zeros = |out: &mut BufWriter<File>, mut left: u64| -> Result<(), ChdError> {
                while left > 0 {
                    let n = left.min(zeros.len() as u64) as usize;
                    out.write_all(&zeros[..n]).map_err(io)?;
                    left -= n as u64;
                }
                Ok(())
            };
            write_zeros(&mut w.out, map_bytes)?;
            w.out.write_all(&chain).map_err(io)?;
            // (the padding up to the first stored hunk is written with that hunk, so a file with none ends after its metadata)
            w.pos = end;
            w.plain_meta = Some((first, entries));
        }
        Ok(w)
    }

    fn compressed(&self) -> bool {
        !self.opts.codecs.is_empty()
    }

    /// Adds image data (any amount; the sizes of the pieces do not matter).
    pub fn push(&mut self, mut data: &[u8]) -> Result<(), ChdError> {
        if self.pushed + data.len() as u64 > self.opts.logical_bytes {
            return config("more data than the image size says");
        }
        self.pushed += data.len() as u64;
        let hb = self.opts.hunk_bytes as usize;
        while !data.is_empty() {
            let take = (hb - self.pending.len()).min(data.len());
            self.pending.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.pending.len() == hb {
                self.batch.push(std::mem::take(&mut self.pending));
                if self.batch.len() >= self.batch_size {
                    self.flush_batch()?;
                }
            }
        }
        Ok(())
    }

    fn prepare(opts: &CreateOptions, bytes: Vec<u8>) -> Prepared {
        let mut best: Option<(u8, Vec<u8>)> = None;
        let mut best_len = bytes.len();
        for (slot, codec) in opts.codecs.iter().enumerate() {
            if let Some(packed) = codec::compress(*codec, &bytes)
                && packed.len() < best_len
            {
                best_len = packed.len();
                best = Some((slot as u8, packed));
            }
        }
        Prepared {
            sha1: sha1_of(&bytes),
            crc: crc16(&bytes),
            packed: best,
            bytes,
        }
    }

    fn flush_batch(&mut self) -> Result<(), ChdError> {
        let hunks = std::mem::take(&mut self.batch);
        if hunks.is_empty() {
            return Ok(());
        }
        let opts = &self.opts;
        let prepared: Vec<Prepared> = if self.threads <= 1 || hunks.len() < 2 {
            hunks.into_iter().map(|h| Self::prepare(opts, h)).collect()
        } else {
            let per = hunks.len().div_ceil(self.threads);
            let mut chunks: Vec<Vec<Vec<u8>>> = Vec::new();
            let mut it = hunks.into_iter();
            loop {
                let chunk: Vec<Vec<u8>> = it.by_ref().take(per).collect();
                if chunk.is_empty() {
                    break;
                }
                chunks.push(chunk);
            }
            std::thread::scope(|s| {
                let handles: Vec<_> = chunks
                    .into_iter()
                    .map(|chunk| {
                        s.spawn(move || {
                            chunk
                                .into_iter()
                                .map(|h| Self::prepare(opts, h))
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|h| h.join().expect("a compression thread panicked"))
                    .collect()
            })
        };
        for p in prepared {
            self.store(p)?;
        }
        Ok(())
    }

    fn store(&mut self, p: Prepared) -> Result<(), ChdError> {
        let n = self.hunks_done;
        let hb = u64::from(self.opts.hunk_bytes);
        let logical = (self.opts.logical_bytes - u64::from(n) * hb).min(hb) as usize;
        self.raw.update(&p.bytes[..logical]);
        let per_hunk = u64::from(self.opts.hunk_bytes / self.opts.unit_bytes);

        // the parent has the same bytes
        let in_parent = self.parent.as_ref().and_then(|par| {
            if par.by_position.get(n as usize) == Some(&p.sha1) {
                Some(u64::from(n) * per_hunk)
            } else {
                par.by_hash.get(&p.sha1).map(|&k| u64::from(k) * per_hunk)
            }
        });
        if self.compressed() {
            let entry = if let Some(offset) = in_parent {
                self.summary.parent_refs += 1;
                Entry {
                    kind: Kind::Parent,
                    length: 0,
                    offset,
                    crc: 0,
                }
            } else if let Some(&first) = self.seen.get(&p.sha1) {
                self.summary.self_refs += 1;
                Entry {
                    kind: Kind::SelfRef,
                    length: 0,
                    offset: u64::from(first),
                    crc: 0,
                }
            } else {
                self.seen.insert(p.sha1, n);
                let offset = self.pos;
                match &p.packed {
                    Some((slot, packed)) => {
                        self.out.write_all(packed).map_err(io)?;
                        self.pos += packed.len() as u64;
                        self.summary.per_codec[usize::from(*slot)] += 1;
                        Entry {
                            kind: Kind::Codec(*slot),
                            length: packed.len() as u32,
                            offset,
                            crc: p.crc,
                        }
                    }
                    None => {
                        self.out.write_all(&p.bytes).map_err(io)?;
                        self.pos += p.bytes.len() as u64;
                        self.summary.plain += 1;
                        Entry {
                            kind: Kind::Plain,
                            length: self.opts.hunk_bytes,
                            offset,
                            crc: p.crc,
                        }
                    }
                }
            };
            self.entries.push(entry);
        } else {
            // plain file: a hunk equal to the parent's (or all zeros without a parent) stays unstored
            let unstored = match &self.parent {
                Some(par) => par.by_position.get(n as usize) == Some(&p.sha1),
                None => p.bytes.iter().all(|&b| b == 0),
            };
            if unstored {
                self.plain_map.push(0);
                if self.parent.is_some() {
                    self.summary.parent_refs += 1;
                }
            } else {
                let aligned = self.pos.div_ceil(hb) * hb;
                if aligned > self.pos {
                    let zeros = vec![0u8; (aligned - self.pos) as usize];
                    self.out.write_all(&zeros).map_err(io)?;
                    self.pos = aligned;
                }
                let at = self.pos / hb;
                if at > u64::from(u32::MAX) {
                    return config("a plain file cannot be larger than 2^32 hunks");
                }
                self.out.write_all(&p.bytes).map_err(io)?;
                self.pos += hb;
                self.plain_map.push(at as u32);
                self.summary.plain += 1;
            }
        }
        self.hunks_done += 1;
        Ok(())
    }

    /// Ends the image: pads the last hunk with zeros, writes the map, the metadata and the header.
    pub fn finish(mut self) -> Result<Summary, ChdError> {
        if self.pushed != self.opts.logical_bytes {
            return config(format!(
                "the image should have {} bytes but {} were given",
                self.opts.logical_bytes, self.pushed
            ));
        }
        if !self.pending.is_empty() {
            let mut last = std::mem::take(&mut self.pending);
            last.resize(self.opts.hunk_bytes as usize, 0);
            self.batch.push(last);
        }
        self.flush_batch()?;
        debug_assert_eq!(self.hunks_done, self.hunk_total);

        let hb = self.opts.hunk_bytes;
        let (map_offset, raw_sha1, sha1);
        // metadata first (at the end of the data), then the map for a compressed file
        let compressed = self.compressed();
        let map_bytes = if compressed {
            Some(mapenc::encode(&self.entries, hb, self.opts.unit_bytes))
        } else {
            None
        };
        if let Some(map) = &map_bytes {
            map_offset = self.pos;
            self.out.write_all(map).map_err(io)?;
            self.pos += map.len() as u64;
        } else {
            map_offset = u64::from(V5_HEADER_LEN);
        }
        let (meta_offset, entries) = if let Some((first, entries)) = self.plain_meta.take() {
            (first, entries)
        } else {
            let mut entries = std::mem::take(&mut self.opts.metadata);
            let (chain, first) = lay_out_chain(&mut entries, self.pos);
            self.out.write_all(&chain).map_err(io)?;
            self.pos += chain.len() as u64;
            (first, entries)
        };

        let raw: [u8; 20] = self.raw.clone().finalize().into();
        if compressed {
            raw_sha1 = raw;
            sha1 = meta::overall_sha1(&raw, &entries);
        } else {
            // an uncompressed file does not carry the hashes
            raw_sha1 = [0; 20];
            sha1 = [0; 20];
        }
        let header = Header {
            compressors: std::array::from_fn(|i| {
                self.opts.codecs.get(i).map_or([0; 4], |c| c.tag())
            }),
            logical_bytes: self.opts.logical_bytes,
            map_offset,
            meta_offset,
            hunk_bytes: hb,
            unit_bytes: self.opts.unit_bytes,
            raw_sha1,
            sha1,
            parent_sha1: self.parent.as_ref().map_or([0; 20], |p| p.sha1),
        };
        if !compressed {
            self.out.seek(SeekFrom::Start(map_offset)).map_err(io)?;
            let mut bytes = Vec::with_capacity(self.plain_map.len() * 4);
            for v in &self.plain_map {
                bytes.extend_from_slice(&v.to_be_bytes());
            }
            self.out.write_all(&bytes).map_err(io)?;
        }
        self.out.seek(SeekFrom::Start(0)).map_err(io)?;
        self.out.write_all(&header.to_bytes()).map_err(io)?;
        self.out.flush().map_err(io)?;
        self.out.get_ref().sync_all().map_err(io)?;
        self.finished = true;
        let _ = self.parent.as_ref().map(|p| p.unit_bytes);
        let mut summary = self.summary.clone();
        summary.file_bytes = self.pos;
        summary.raw_sha1 = raw_sha1;
        summary.sha1 = sha1;
        Ok(summary)
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        if !self.finished {
            // a file that was not finished is not a CHD; do not leave it behind
            let _ = self.out.flush();
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp(name: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("rc-chd-writer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{}-{name}", N.fetch_add(1, Ordering::Relaxed)))
    }

    /// Data of several kinds, with repeated hunks and an all-zero stretch.
    fn data(len: usize) -> Vec<u8> {
        let mut seed = 1u32;
        (0..len)
            .map(|i| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                match (i / 4096) % 6 {
                    0 => b"retroclawd writes chd files "[i % 28],
                    1 => 0,
                    2 => (i % 251) as u8,
                    3 => (seed >> 24) as u8,
                    4 => b"retroclawd writes chd files "[i % 28],
                    _ => ((i as f64 / 7.0).sin() * 100.0) as i8 as u8,
                }
            })
            .collect()
    }

    fn options(codecs: &[Codec], logical: usize, hunk: u32) -> CreateOptions {
        CreateOptions {
            hunk_bytes: hunk,
            unit_bytes: 4,
            codecs: codecs.to_vec(),
            logical_bytes: logical as u64,
            metadata: vec![MetaEntry::text(*b"TEST", "hello")],
            overwrite: false,
            threads: 2,
        }
    }

    fn write(path: &Path, opts: CreateOptions, bytes: &[u8], parent: Option<Image>) -> Summary {
        let mut w = Writer::create(path, opts, parent).unwrap();
        // pushed in odd pieces: the piece sizes must not matter
        for piece in bytes.chunks(1000) {
            w.push(piece).unwrap();
        }
        w.finish().unwrap()
    }

    #[test]
    fn every_codec_set_writes_a_file_that_reads_and_verifies() {
        let bytes = data(4096 * 40 + 100);
        let sets: Vec<Vec<Codec>> = vec![
            vec![],
            vec![Codec::Zlib],
            vec![Codec::Zstd],
            vec![Codec::Lzma],
            vec![Codec::Huff],
            vec![Codec::Flac],
            vec![Codec::Lzma, Codec::Zlib, Codec::Huff, Codec::Flac],
        ];
        for codecs in sets {
            let path = temp("codecs.chd");
            let summary = write(&path, options(&codecs, bytes.len(), 4096), &bytes, None);
            assert_eq!(summary.hunks, 41);
            let mut image = Image::open(&path, None).unwrap();
            assert_eq!(
                image.read_bytes(0, bytes.len()).unwrap(),
                bytes,
                "{codecs:?}"
            );
            let report = image.verify(|_, _| {}).unwrap();
            assert!(report.ok(), "{codecs:?}: {report:?}");
            assert_eq!(image.metadata().len(), 1);
            assert_eq!(image.metadata()[0].data, b"hello\0");
            if !codecs.is_empty() {
                assert_eq!(report.raw_sha1, summary.raw_sha1);
                assert_eq!(image.header().sha1, summary.sha1);
                assert!(summary.self_refs > 0, "repeated hunks become references");
            }
        }
    }

    #[test]
    fn a_delta_file_stores_only_what_the_parent_lacks() {
        let base = data(4096 * 30);
        let parent_path = temp("parent.chd");
        write(
            &parent_path,
            options(&[Codec::Zlib], base.len(), 4096),
            &base,
            None,
        );
        let mut changed = base.clone();
        changed[4096 * 3 + 5] ^= 0xFF;
        changed[4096 * 20] ^= 0x55;
        for codecs in [vec![Codec::Zlib], vec![]] {
            let child = temp("child.chd");
            let parent = Image::open(&parent_path, None).unwrap();
            let summary = write(
                &child,
                options(&codecs, changed.len(), 4096),
                &changed,
                Some(parent),
            );
            assert!(summary.parent_refs >= 26, "{summary:?}");
            assert!(
                Image::open(&child, None).is_err(),
                "a delta file needs its parent"
            );
            let mut image = Image::open(&child, Some(&parent_path)).unwrap();
            assert_eq!(image.read_bytes(0, changed.len()).unwrap(), changed);
            assert!(image.verify(|_, _| {}).unwrap().ok());
            // the wrong parent is refused
            let other = temp("other.chd");
            write(
                &other,
                options(&[Codec::Zlib], 8192, 4096),
                &data(8192),
                None,
            );
            assert!(Image::open(&child, Some(&other)).is_err());
        }
    }

    #[test]
    fn mistakes_are_errors_and_leave_no_file() {
        let path = temp("bad.chd");
        let bytes = data(10_000);
        // too little data
        let mut w = Writer::create(&path, options(&[Codec::Zlib], 20_000, 4096), None).unwrap();
        w.push(&bytes).unwrap();
        assert!(w.finish().is_err());
        assert!(!path.exists());
        // too much data
        let mut w = Writer::create(&path, options(&[Codec::Zlib], 100, 4096), None).unwrap();
        assert!(w.push(&bytes).is_err());
        drop(w);
        assert!(!path.exists());
        // an existing file is not replaced unasked
        std::fs::write(&path, b"mine").unwrap();
        assert!(Writer::create(&path, options(&[Codec::Zlib], 100, 4096), None).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"mine");
        // a CD codec needs whole frames
        assert!(
            Writer::create(&temp("cd.chd"), options(&[Codec::CdZlib], 100, 4096), None).is_err()
        );
        assert!(
            Writer::create(
                &temp("five.chd"),
                options(&[Codec::Zlib; 5], 100, 4096),
                None
            )
            .is_err()
        );
    }
}
