//! Source-level debugging: the `.rcdbg` file the assemblers write (`rcasm --debug-info`, `rcb build`) says which source
//! line made which bytes. The debuggers use it to show the line at the PC, to set a breakpoint by `file:line` and to
//! step by line. Format (one record per line, `#` starts a comment):
//!
//! ```text
//! rcdbg 1
//! file 0 src/main.asm
//! line 0 12 0xE000 3        # file index, line, address, length in bytes
//! sym reset 0xE000
//! ```
//!
//! A program the system loads where it likes (an Atari ST `.PRG`, an Amiga or X68000 executable) has offsets from the
//! start of its text instead of addresses, and says how to find that start:
//!
//! ```text
//! base text gemdos          # `gemdos`: the GEMDOS basepage 256 bytes before the text names it (p_tbase); `scan`: no check
//! anchor 41F9????????4E75   # the text's first bytes, `??` where the loader patched an address
//! ```
//!
//! The debugger then looks for the anchor in memory when it is paused (at most twice a second while not found), checks a
//! place it found again later (a program run a second time may be elsewhere), and `--source-base ADDR` names the
//! address instead.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How a relocated program's text is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    /// The basepage of the GEMDOS process, 256 bytes before the text, holds the text's address at offset 8.
    Gemdos,
    /// The first place the anchor matches.
    Scan,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceInfo {
    files: Vec<String>,
    /// (file index, line, address, length)
    lines: Vec<(u32, u32, u32, u32)>,
    /// Offsets from the text of a program loaded by the system, and how to find it.
    relocated: Option<Method>,
    /// The text's first bytes; `None` for the bytes the loader changes.
    anchor: Vec<Option<u8>>,
}

static INFO: OnceLock<SourceInfo> = OnceLock::new();

/// Where the relocated program's text is, once found or given.
struct Placement {
    base: Option<u32>,
    /// `--source-base`: never searched for.
    fixed: bool,
    last_search: Option<Instant>,
}

static PLACE: Mutex<Placement> = Mutex::new(Placement {
    base: None,
    fixed: false,
    last_search: None,
});

/// How far a search for the program goes: the RAM of the 68000 machines here is at most 4 MB.
const SEARCH_LIMIT: u32 = 0x40_0000;
const SEARCH_INTERVAL: Duration = Duration::from_millis(500);

fn number(word: &str) -> Option<u32> {
    match word.strip_prefix("0x").or_else(|| word.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => word.parse().ok(),
    }
}

impl SourceInfo {
    pub(crate) fn parse(text: &str) -> Result<SourceInfo, String> {
        let mut info = SourceInfo::default();
        let mut header = false;
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let bad = |what: &str| format!("line {}: {what}", n + 1);
            let mut words = line.split_whitespace();
            match words.next() {
                Some("rcdbg") => {
                    if words.next() != Some("1") {
                        return Err(bad("only `rcdbg 1` is known"));
                    }
                    header = true;
                }
                Some("file") => {
                    let index: usize = words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(|| bad("file needs an index"))?;
                    let name = line
                        .splitn(3, char::is_whitespace)
                        .nth(2)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| bad("file needs a name"))?;
                    if index != info.files.len() {
                        return Err(bad("file indexes count up from 0"));
                    }
                    info.files.push(name.to_string());
                }
                Some("line") => {
                    let mut next = || words.next().and_then(number);
                    let (f, l, a, len) = match (next(), next(), next(), next()) {
                        (Some(f), Some(l), Some(a), Some(len)) => (f, l, a, len),
                        _ => return Err(bad("line needs file, line, address and length")),
                    };
                    if f as usize >= info.files.len() {
                        return Err(bad("line names a file that is not listed"));
                    }
                    info.lines.push((f, l, a, len));
                }
                Some("base") => {
                    info.relocated = match (words.next(), words.next()) {
                        (Some("text"), Some("gemdos")) => Some(Method::Gemdos),
                        (Some("text"), Some("scan")) => Some(Method::Scan),
                        _ => return Err(bad("base is `text gemdos` or `text scan`")),
                    };
                }
                Some("anchor") => {
                    let hex = words.next().unwrap_or("");
                    if hex.is_empty() || hex.len() % 2 != 0 || !hex.is_ascii() {
                        return Err(bad("anchor is pairs of hex digits or ??"));
                    }
                    info.anchor = hex
                        .as_bytes()
                        .chunks(2)
                        .map(|p| match p {
                            b"??" => Ok(None),
                            p => std::str::from_utf8(p)
                                .ok()
                                .and_then(|s| u8::from_str_radix(s, 16).ok())
                                .map(Some)
                                .ok_or_else(|| bad("anchor is pairs of hex digits or ??")),
                        })
                        .collect::<Result<_, _>>()?;
                }
                // symbols are `--symbols`' business
                Some("sym") => {}
                Some(other) => return Err(bad(&format!("unknown record \"{other}\""))),
                None => {}
            }
        }
        if !header {
            return Err("not an rcdbg file (no `rcdbg 1` line)".to_string());
        }
        if info.relocated.is_some() && !info.anchor.iter().any(Option::is_some) {
            return Err("a relocated program needs an anchor with known bytes".to_string());
        }
        Ok(info)
    }

    /// The source line that made the byte at `addr` (an offset from the text for a relocated program).
    pub(crate) fn line_at(&self, addr: u64) -> Option<(&str, u32)> {
        let addr = u32::try_from(addr).ok()?;
        self.lines
            .iter()
            .find(|(_, _, a, n)| addr >= *a && addr < a.saturating_add((*n).max(1)))
            .map(|(f, l, _, _)| (self.files[*f as usize].as_str(), *l))
    }

    /// The address of the first instruction `file:line` made. `file` matches a recorded name or the end of one
    /// (`main.asm` finds `src/main.asm`).
    pub(crate) fn address_of(&self, file: &str, line: u32) -> Result<u32, String> {
        let norm = |s: &str| s.replace('\\', "/");
        let want = norm(file);
        let candidates: Vec<usize> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                let f = norm(f);
                f == want || f.ends_with(&format!("/{want}"))
            })
            .map(|(i, _)| i)
            .collect();
        if candidates.is_empty() {
            return Err(format!(
                "no source file \"{file}\" in the debug information"
            ));
        }
        self.lines
            .iter()
            .filter(|(f, l, _, n)| candidates.contains(&(*f as usize)) && *l == line && *n > 0)
            .map(|(_, _, a, _)| *a)
            .min()
            .ok_or_else(|| format!("{file}:{line} made no code"))
    }

    /// The anchor's known bytes are at `addr` (and, for GEMDOS, the basepage before it names `addr`).
    fn placed_at(&self, read: &dyn Fn(u32) -> u8, addr: u32) -> bool {
        let bytes_match = self.anchor.iter().enumerate().all(|(i, b)| match b {
            Some(b) => read(addr.wrapping_add(i as u32)) == *b,
            None => true,
        });
        if !bytes_match {
            return false;
        }
        match self.relocated {
            Some(Method::Gemdos) => {
                let Some(bp) = addr.checked_sub(0x100) else {
                    return false;
                };
                let tbase = (0..4).fold(0u32, |v, k| v << 8 | u32::from(read(bp + 8 + k)));
                tbase == addr
            }
            _ => true,
        }
    }

    /// Searches `0..limit` (even addresses: 68000 code) for the program's text.
    fn find(&self, read: &dyn Fn(u32) -> u8, limit: u32) -> Option<u32> {
        let (k, first) = self
            .anchor
            .iter()
            .enumerate()
            .find_map(|(i, b)| b.map(|b| (i, b)))?;
        let span = self.anchor.len() as u32;
        (0..limit.saturating_sub(span))
            .step_by(2)
            .find(|&a| read(a + k as u32) == first && self.placed_at(read, a))
    }
}

/// Reads the `--source-info` file (a path, URL or `archive.zip#entry`) and keeps it for the rest of the run.
pub(crate) fn load(source: &str) -> Result<(), String> {
    let loaded = crate::inputs::load(source)?;
    let info = SourceInfo::parse(&String::from_utf8_lossy(&loaded.bytes))
        .map_err(|e| format!("{source}: {e}"))?;
    let _ = INFO.set(info);
    Ok(())
}

/// `--source-base ADDR`: where a relocated program's text is, instead of searching for it.
pub(crate) fn set_base(addr: u32) {
    let mut p = PLACE.lock().unwrap();
    p.base = Some(addr);
    p.fixed = true;
}

/// For a relocated program: finds where its text is in memory (`read` reads the machine's memory), or checks that the
/// place found before still holds it. Cheap when nothing is to be done; a search runs at most every half second.
pub(crate) fn locate(read: &dyn Fn(u32) -> u8) {
    let Some(info) = INFO.get() else { return };
    if info.relocated.is_none() {
        return;
    }
    let mut p = PLACE.lock().unwrap();
    if p.fixed {
        return;
    }
    if let Some(base) = p.base {
        if info.placed_at(read, base) {
            return;
        }
        p.base = None;
    }
    if p.last_search.is_some_and(|t| t.elapsed() < SEARCH_INTERVAL) {
        return;
    }
    p.last_search = Some(Instant::now());
    p.base = info.find(read, SEARCH_LIMIT);
}

/// The text's address of a relocated program: `Ok(0)` for one that is not relocated.
fn offset_base(info: &SourceInfo) -> Result<u32, String> {
    if info.relocated.is_none() {
        return Ok(0);
    }
    PLACE.lock().unwrap().base.ok_or_else(|| {
        "the program has not been found in memory yet (it is looked for while the machine is paused, once it is \
         loaded); --source-base ADDR names its text address"
            .to_string()
    })
}

/// The source line at `addr`, as `(file, line)`.
pub(crate) fn line_at(addr: u64) -> Option<(String, u32)> {
    let info = INFO.get()?;
    let base = offset_base(info).ok()?;
    let at = addr.checked_sub(u64::from(base))?;
    info.line_at(at).map(|(f, l)| (f.to_string(), l))
}

/// The address of `file:line`; an error says why not (no debug information, an unknown file, a line without code, a
/// relocated program not found yet).
pub(crate) fn address_of(file: &str, line: u32) -> Result<u32, String> {
    let info = INFO
        .get()
        .ok_or_else(|| "no debug information: start with --source-info FILE.rcdbg".to_string())?;
    let offset = info.address_of(file, line)?;
    Ok(offset_base(info)?.wrapping_add(offset))
}

/// Splits `file:line` (the line after the last colon).
pub(crate) fn split_spec(spec: &str) -> Option<(&str, u32)> {
    let (file, line) = spec.rsplit_once(':')?;
    if file.is_empty() {
        return None;
    }
    Some((file, line.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "rcdbg 1\nfile 0 src/main.asm\nfile 1 src/lib.asm\nline 0 3 0xE000 2\nline 0 4 0xE002 3 # a comment\nline 1 9 0xF000 1\nsym reset 0xE000\n";

    #[test]
    fn lines_map_both_ways() {
        let info = SourceInfo::parse(TEXT).unwrap();
        assert_eq!(info.line_at(0xE000), Some(("src/main.asm", 3)));
        assert_eq!(info.line_at(0xE001), Some(("src/main.asm", 3)));
        assert_eq!(info.line_at(0xE004), Some(("src/main.asm", 4)));
        assert_eq!(info.line_at(0xE005), None);
        assert_eq!(info.line_at(0xF000), Some(("src/lib.asm", 9)));
        assert_eq!(info.address_of("src/main.asm", 4), Ok(0xE002));
        assert_eq!(
            info.address_of("main.asm", 4),
            Ok(0xE002),
            "a file name is enough"
        );
        assert_eq!(info.address_of("lib.asm", 9), Ok(0xF000));
        assert!(
            info.address_of("main.asm", 99)
                .unwrap_err()
                .contains("no code")
        );
        assert!(
            info.address_of("other.asm", 1)
                .unwrap_err()
                .contains("no source file")
        );
    }

    #[test]
    fn bad_files_are_errors_not_panics() {
        assert!(SourceInfo::parse("").is_err());
        assert!(SourceInfo::parse("rcdbg 2\n").is_err());
        assert!(
            SourceInfo::parse("rcdbg 1\nline 0 1 0 1\n")
                .unwrap_err()
                .contains("not listed")
        );
        assert!(SourceInfo::parse("rcdbg 1\nfile 3 x\n").is_err());
        assert!(SourceInfo::parse("rcdbg 1\nbogus\n").is_err());
        assert!(SourceInfo::parse("rcdbg 1\nfile 0 a\nline 0 x\n").is_err());
        assert!(SourceInfo::parse("rcdbg 1\nbase text elsewhere\nanchor 4E75\n").is_err());
        assert!(SourceInfo::parse("rcdbg 1\nbase text scan\nanchor 4E7\n").is_err());
        assert!(SourceInfo::parse("rcdbg 1\nbase text scan\nanchor zz\n").is_err());
        assert!(
            SourceInfo::parse("rcdbg 1\nbase text scan\nanchor ????\n").is_err(),
            "an anchor of masked bytes only finds anything"
        );
        assert!(SourceInfo::parse("rcdbg 1\nbase text scan\nanchor \u{e9}\u{e9}\n").is_err());
    }

    /// A relocated program: found by its anchor (masked bytes ignored), and with GEMDOS only where the basepage names it.
    #[test]
    fn a_relocated_program_is_found_by_its_anchor() {
        let text = "rcdbg 1\nbase text gemdos\nanchor 41F9????????4E75\nfile 0 main.s\nline 0 2 0x0 6\nline 0 3 0x6 2\n";
        let info = SourceInfo::parse(text).unwrap();
        let mut ram = vec![0u8; 0x10000];
        let code = [0x41, 0xF9, 0, 0, 0x20, 0x08, 0x4E, 0x75];
        // a copy of the program file in a disk buffer, with no basepage before it
        ram[0x1000..0x1008].copy_from_slice(&[0x41, 0xF9, 0, 0, 0, 0x08, 0x4E, 0x75]);
        // the loaded program at $2000 (the loader patched the long), its basepage at $1F00 naming it
        ram[0x2000..0x2008].copy_from_slice(&code);
        ram[0x1F08..0x1F0C].copy_from_slice(&0x2000u32.to_be_bytes());
        let read = |a: u32| ram.get(a as usize).copied().unwrap_or(0xFF);
        assert_eq!(info.find(&read, 0x10000), Some(0x2000));
        assert!(info.placed_at(&read, 0x2000));
        assert!(!info.placed_at(&read, 0x1000));
        // `scan` takes the first match
        let scan = SourceInfo::parse(&text.replace("gemdos", "scan")).unwrap();
        assert_eq!(scan.find(&read, 0x10000), Some(0x1000));
        assert_eq!(info.line_at(0x6), Some(("main.s", 3)));
        assert_eq!(info.address_of("main.s", 3), Ok(0x6));
        // nothing to find
        let empty = vec![0u8; 0x1000];
        assert_eq!(info.find(&|a| empty[a as usize], 0x1000), None);
    }

    #[test]
    fn file_colon_line_is_split_on_the_last_colon() {
        assert_eq!(split_spec("src/main.asm:12"), Some(("src/main.asm", 12)));
        assert_eq!(split_spec("C:\\src\\a.asm:7"), Some(("C:\\src\\a.asm", 7)));
        assert_eq!(split_spec("main.asm"), None);
        assert_eq!(split_spec(":3"), None);
        assert_eq!(split_spec("a.asm:x"), None);
    }
}
