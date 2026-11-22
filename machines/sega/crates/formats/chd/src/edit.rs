//! Changing the metadata of a CHD file in place: adding, replacing and deleting entries of the chain, and keeping the header's overall
//! SHA-1 (which covers the checksummed metadata) right. The hunks and the map are not touched.
//!
//! The order of the writes keeps the file readable at every step: a new or longer entry is appended at the end of the file first,
//! then the one pointer that makes the chain reach it is changed, then the header's hash. A deleted entry is unlinked and its bytes
//! stay in the file as dead space, as chdman leaves them.

use crate::image::{Header, V5_HEADER_LEN};
use crate::meta::{self, FLAG_CHECKSUM, MetaEntry};
use crate::{ChdError, format_err};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

fn io(e: std::io::Error) -> ChdError {
    ChdError::Io(format!("editing the CHD file: {e}"))
}

struct Open {
    file: File,
    len: u64,
    header: Header,
    chain: Vec<MetaEntry>,
}

fn open(path: &Path) -> Result<Open, ChdError> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| ChdError::Io(format!("{}: {e}", path.display())))?;
    let len = file.metadata().map_err(io)?.len();
    let mut head = [0u8; V5_HEADER_LEN as usize];
    let have = file.read(&mut head).map_err(io)?;
    let header = Header::parse(&head[..have])?;
    let chain = meta::read_chain(&mut file, header.meta_offset, len)?;
    Ok(Open {
        file,
        len,
        header,
        chain,
    })
}

/// The header and the metadata of a file, without opening its map or any parent (so a delta file can be inspected alone).
pub fn read_header_and_metadata(path: &Path) -> Result<(Header, Vec<MetaEntry>), ChdError> {
    let o = open(path)?;
    Ok((o.header, o.chain))
}

/// The position in the chain of the `index`-th entry with `tag`.
fn find(chain: &[MetaEntry], tag: &[u8; 4], index: u32) -> Option<usize> {
    chain
        .iter()
        .enumerate()
        .filter(|(_, e)| &e.tag == tag)
        .nth(index as usize)
        .map(|(i, _)| i)
}

impl Open {
    /// Points the entry before position `at` (or the header, for position 0) at `target`.
    fn relink(&mut self, at: usize, target: u64) -> Result<(), ChdError> {
        if at == 0 {
            self.header.meta_offset = target;
            self.file.seek(SeekFrom::Start(48)).map_err(io)?;
        } else {
            let prev = self.chain[at - 1].offset;
            // the `next` field is the last eight bytes of the 16-byte entry header
            self.file.seek(SeekFrom::Start(prev + 8)).map_err(io)?;
        }
        self.file.write_all(&target.to_be_bytes()).map_err(io)
    }

    /// Writes the overall SHA-1 for the chain as it now is (a file without hashes keeps none).
    fn rehash(&mut self, chain: &[MetaEntry]) -> Result<(), ChdError> {
        if self.header.compressed() {
            self.header.sha1 = meta::overall_sha1(&self.header.raw_sha1, chain);
            self.file.seek(SeekFrom::Start(84)).map_err(io)?;
            self.file.write_all(&self.header.sha1).map_err(io)?;
        }
        self.file.sync_all().map_err(io)
    }
}

/// Replaces the two hashes in a compressed file's header (what `verify --fix` does after it found them wrong).
pub fn write_hashes(path: &Path, raw_sha1: &[u8; 20], sha1: &[u8; 20]) -> Result<(), ChdError> {
    let mut o = open(path)?;
    if !o.header.compressed() {
        return format_err("an uncompressed file holds no hashes");
    }
    o.file.seek(SeekFrom::Start(64)).map_err(io)?;
    o.file.write_all(raw_sha1).map_err(io)?;
    o.file.seek(SeekFrom::Start(84)).map_err(io)?;
    o.file.write_all(sha1).map_err(io)?;
    o.file.sync_all().map_err(io)
}

/// Adds an entry, or replaces the `index`-th entry with the same tag (in place when the new data fits in the old room).
pub fn set_metadata(
    path: &Path,
    tag: [u8; 4],
    index: u32,
    data: &[u8],
    checksum: bool,
) -> Result<(), ChdError> {
    if data.is_empty() || data.len() >= 1 << 24 {
        return format_err("metadata is at least one byte and under 16 MiB");
    }
    let flags = if checksum { FLAG_CHECKSUM } else { 0 };
    let mut o = open(path)?;
    let existing = find(&o.chain, &tag, index);
    if existing.is_none() && index > 0 {
        // asking for the Nth of something that has fewer is a mistake, not an append
        let have = o.chain.iter().filter(|e| e.tag == tag).count() as u32;
        if index > have {
            return format_err("the index is past the entries with that tag");
        }
    }
    let mut after = o.chain.clone();
    match existing {
        Some(i) if data.len() <= o.chain[i].data.len() => {
            // in place: the data and, when it changed, the length; the flags byte stays
            let e = &o.chain[i];
            o.file.seek(SeekFrom::Start(e.offset + 5)).map_err(io)?;
            o.file
                .write_all(&(data.len() as u32).to_be_bytes()[1..])
                .map_err(io)?;
            o.file.seek(SeekFrom::Start(e.offset + 16)).map_err(io)?;
            o.file.write_all(data).map_err(io)?;
            after[i].data = data.to_vec();
        }
        _ => {
            // appended at the end of the file, then linked in
            let new = MetaEntry {
                tag,
                flags,
                data: data.to_vec(),
                offset: o.len,
                next: existing.map_or(0, |i| o.chain[i].next),
            };
            let bytes = meta::entry_bytes(&new, new.next);
            o.file.seek(SeekFrom::Start(o.len)).map_err(io)?;
            o.file.write_all(&bytes).map_err(io)?;
            match existing {
                Some(i) => {
                    o.relink(i, new.offset)?;
                    after[i] = new;
                }
                None => {
                    // a new entry goes at the end of the chain
                    let at = o.chain.len();
                    o.relink(at, new.offset)?;
                    after.push(new);
                }
            }
        }
    }
    o.rehash(&after)
}

/// Unlinks the `index`-th entry with `tag`.
pub fn delete_metadata(path: &Path, tag: [u8; 4], index: u32) -> Result<(), ChdError> {
    let mut o = open(path)?;
    let Some(i) = find(&o.chain, &tag, index) else {
        return format_err("there is no such metadata entry");
    };
    let next = o.chain[i].next;
    o.relink(i, next)?;
    let mut after = o.chain.clone();
    after.remove(i);
    o.rehash(&after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Codec, CreateOptions, Image, Writer};

    fn make(name: &str, codecs: &[Codec]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rc-chd-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        let data: Vec<u8> = (0..20_000u32).map(|i| (i / 7) as u8).collect();
        let opts = CreateOptions {
            hunk_bytes: 4096,
            unit_bytes: 4,
            codecs: codecs.to_vec(),
            logical_bytes: data.len() as u64,
            metadata: vec![
                MetaEntry::text(*b"AAAA", "first"),
                MetaEntry::text(*b"BBBB", "second"),
            ],
            overwrite: true,
            threads: 1,
        };
        let mut w = Writer::create(&path, opts, None).unwrap();
        w.push(&data).unwrap();
        w.finish().unwrap();
        path
    }

    fn texts(path: &Path) -> Vec<(String, Vec<u8>)> {
        Image::open(path, None)
            .unwrap()
            .metadata()
            .iter()
            .map(|e| (String::from_utf8_lossy(&e.tag).into_owned(), e.data.clone()))
            .collect()
    }

    #[test]
    fn entries_are_added_replaced_and_deleted_and_the_file_still_verifies() {
        for codecs in [vec![Codec::Zlib], vec![]] {
            let path = make("e.chd", &codecs);
            let check = |path: &Path| {
                let mut image = Image::open(path, None).unwrap();
                assert!(image.verify(|_, _| {}).unwrap().ok());
            };
            // new entry goes last
            set_metadata(&path, *b"CCCC", 0, b"third\0", true).unwrap();
            let names: Vec<String> = texts(&path).into_iter().map(|t| t.0).collect();
            assert_eq!(names, ["AAAA", "BBBB", "CCCC"]);
            check(&path);
            // shorter: in place; longer: appended and relinked in the same position
            set_metadata(&path, *b"BBBB", 0, b"2nd\0", true).unwrap();
            set_metadata(&path, *b"AAAA", 0, b"a much longer first entry\0", true).unwrap();
            let t = texts(&path);
            assert_eq!(
                t[0],
                ("AAAA".to_string(), b"a much longer first entry\0".to_vec())
            );
            assert_eq!(t[1], ("BBBB".to_string(), b"2nd\0".to_vec()));
            check(&path);
            // a second entry of the same tag, then deleting the first of the two
            set_metadata(&path, *b"CCCC", 1, b"again\0", true).unwrap();
            delete_metadata(&path, *b"CCCC", 0).unwrap();
            let t = texts(&path);
            assert_eq!(t.iter().filter(|e| e.0 == "CCCC").count(), 1);
            assert_eq!(t.last().unwrap().1, b"again\0");
            delete_metadata(&path, *b"AAAA", 0).unwrap();
            assert_eq!(texts(&path)[0].0, "BBBB");
            check(&path);
            // mistakes
            assert!(delete_metadata(&path, *b"ZZZZ", 0).is_err());
            assert!(set_metadata(&path, *b"BBBB", 5, b"x", true).is_err());
            assert!(set_metadata(&path, *b"BBBB", 0, b"", true).is_err());
        }
    }
}
