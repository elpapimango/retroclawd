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

use std::sync::OnceLock;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceInfo {
    files: Vec<String>,
    /// (file index, line, address, length)
    lines: Vec<(u32, u32, u32, u32)>,
}

static INFO: OnceLock<SourceInfo> = OnceLock::new();

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
                // symbols are `--symbols`' business
                Some("sym") => {}
                Some(other) => return Err(bad(&format!("unknown record \"{other}\""))),
                None => {}
            }
        }
        if !header {
            return Err("not an rcdbg file (no `rcdbg 1` line)".to_string());
        }
        Ok(info)
    }

    /// The source line that made the byte at `addr`.
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
            return Err(format!("no source file \"{file}\" in the debug information"));
        }
        self.lines
            .iter()
            .filter(|(f, l, _, n)| candidates.contains(&(*f as usize)) && *l == line && *n > 0)
            .map(|(_, _, a, _)| *a)
            .min()
            .ok_or_else(|| format!("{file}:{line} made no code"))
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

/// The source line at `addr`, as `(file, line)`.
pub(crate) fn line_at(addr: u64) -> Option<(String, u32)> {
    INFO.get()?.line_at(addr).map(|(f, l)| (f.to_string(), l))
}

/// The address of `file:line`; an error says why not (no debug information, an unknown file, a line without code).
pub(crate) fn address_of(file: &str, line: u32) -> Result<u32, String> {
    INFO.get()
        .ok_or_else(|| "no debug information: start with --source-info FILE.rcdbg".to_string())?
        .address_of(file, line)
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
        assert_eq!(info.address_of("main.asm", 4), Ok(0xE002), "a file name is enough");
        assert_eq!(info.address_of("lib.asm", 9), Ok(0xF000));
        assert!(info.address_of("main.asm", 99).unwrap_err().contains("no code"));
        assert!(info.address_of("other.asm", 1).unwrap_err().contains("no source file"));
    }

    #[test]
    fn bad_files_are_errors_not_panics() {
        assert!(SourceInfo::parse("").is_err());
        assert!(SourceInfo::parse("rcdbg 2\n").is_err());
        assert!(SourceInfo::parse("rcdbg 1\nline 0 1 0 1\n").unwrap_err().contains("not listed"));
        assert!(SourceInfo::parse("rcdbg 1\nfile 3 x\n").is_err());
        assert!(SourceInfo::parse("rcdbg 1\nbogus\n").is_err());
        assert!(SourceInfo::parse("rcdbg 1\nfile 0 a\nline 0 x\n").is_err());
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
