//! Cross-checks with chdman, the reference tool, used as a black box. Ignored unless `RC_CHDMAN` names its executable:
//!
//!     RC_CHDMAN=C:/path/chdman.exe cargo test -p chd --test chdman -- --ignored --nocapture
//!
//! Files made by chdman must read here, and files made here must pass `chdman verify`.

use chd::{Codec, Image};
use std::path::{Path, PathBuf};
use std::process::Command;

fn chdman() -> Option<PathBuf> {
    std::env::var("RC_CHDMAN").ok().map(PathBuf::from)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rc-chd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn run(tool: &Path, args: &[&str]) -> String {
    let out = Command::new(tool).args(args).output().expect("chdman runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "chdman {args:?} failed:\n{text}");
    text
}

/// 100 KB of data with structure (text, runs, ramps, stereo-ish audio) and a tail that does not fill a hunk.
fn sample(len: usize) -> Vec<u8> {
    let mut seed = 7u32;
    (0..len)
        .map(|i| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            match (i / 512) % 5 {
                0 => b"the quick brown fox jumps over the lazy dog "[i % 44],
                1 => 0,
                2 => (i % 256) as u8,
                3 => ((i as f64 / 9.0).sin() * 90.0) as i8 as u8,
                _ => (seed >> 24) as u8 & 0x0F,
            }
        })
        .collect()
}

#[test]
#[ignore]
fn chdman_files_of_every_codec_read_and_verify_here() {
    let Some(tool) = chdman() else { return };
    let data = sample(100_000 + 124);
    let input = scratch("in.bin");
    std::fs::write(&input, &data).unwrap();
    for codec in [
        "none",
        "zlib",
        "zstd",
        "lzma",
        "huff",
        "flac",
        "zlib,huff,flac,lzma",
    ] {
        let out = scratch(&format!("m-{}.chd", codec.replace(',', "-")));
        let _ = std::fs::remove_file(&out);
        run(
            &tool,
            &[
                "createraw",
                "-i",
                input.to_str().unwrap(),
                "-o",
                out.to_str().unwrap(),
                "-hs",
                "4096",
                "-us",
                "4",
                "-c",
                codec,
            ],
        );
        let mut image = Image::open(&out, None).unwrap_or_else(|e| panic!("{codec}: {e}"));
        assert_eq!(image.header().logical_bytes, data.len() as u64);
        let bytes = image
            .read_bytes(0, data.len())
            .unwrap_or_else(|e| panic!("{codec}: {e}"));
        assert!(bytes == data, "{codec}: the bytes differ");
        let report = image
            .verify(|_, _| {})
            .unwrap_or_else(|e| panic!("{codec}: {e}"));
        assert!(report.ok(), "{codec}: {report:?}");
        println!(
            "{codec}: ok, {} hunks, {} bytes, compressors {:?}",
            image.hunk_count(),
            image.file_len(),
            image
                .header()
                .compressors
                .iter()
                .filter(|t| **t != [0; 4])
                .map(|t| String::from_utf8_lossy(t).to_string())
                .collect::<Vec<_>>()
        );
        let _ = Codec::ALL;
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
#[ignore]
fn files_written_here_pass_chdman_verify_and_agree_on_the_hashes() {
    use chd::{CreateOptions, MetaEntry, Writer};
    let Some(tool) = chdman() else { return };
    let data = sample(100_000 + 124);
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
        let out = scratch("w.chd");
        let _ = std::fs::remove_file(&out);
        let opts = CreateOptions {
            hunk_bytes: 4096,
            unit_bytes: 4,
            codecs: codecs.clone(),
            logical_bytes: data.len() as u64,
            metadata: vec![MetaEntry::text(*b"TEST", "hello")],
            overwrite: false,
            threads: 0,
        };
        let mut w = Writer::create(&out, opts, None).unwrap();
        w.push(&data).unwrap();
        let summary = w.finish().unwrap();
        if !codecs.is_empty() {
            // chdman has nothing to verify in an uncompressed file (it holds no hashes)
            let text = run(&tool, &["verify", "-i", out.to_str().unwrap()]);
            assert!(
                text.contains("verification successful"),
                "{codecs:?}: {text}"
            );
        }
        let info = run(&tool, &["info", "-v", "-i", out.to_str().unwrap()]);
        if !codecs.is_empty() {
            assert!(
                info.contains(&hex(&summary.raw_sha1)),
                "{codecs:?}: raw sha1 differs:\n{info}"
            );
            assert!(
                info.contains(&hex(&summary.sha1)),
                "{codecs:?}: overall sha1 differs:\n{info}"
            );
        }
        println!("{codecs:?}: chdman verify ok");
    }
}
