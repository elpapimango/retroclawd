//! Memory watchpoints: address ranges that stop the machine when the CPU reads or writes them, set by the plugin option
//! `watch` (`$1000-$10FF:w,$400:rw`). The stop is the machine's ordinary one: it keeps the access, the value, the PC (in the
//! diagnostic report) and the cycle.

/// The most ranges a list holds.
pub const MAX_WATCHES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watch {
    pub lo: u32,
    pub hi: u32,
    pub read: bool,
    pub write: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WatchList {
    ranges: Vec<Watch>,
}

fn number(text: &str) -> Result<u32, String> {
    let t = text.trim();
    let hex = t
        .strip_prefix('$')
        .or_else(|| t.strip_prefix("0x"))
        .or_else(|| t.strip_prefix("0X"));
    let v = match hex {
        Some(h) => u32::from_str_radix(h, 16),
        None => t.parse::<u32>(),
    }
    .map_err(|_| format!("\"{t}\" is not an address (decimal, $hex or 0xhex)"))?;
    if v > 0xFF_FFFF {
        return Err(format!("address ${v:X} is beyond the 24-bit address space"));
    }
    Ok(v)
}

impl WatchList {
    /// Parses `RANGE[:MODE]` entries separated by commas: RANGE is `ADDR` or `ADDR-ADDR` (inclusive), MODE `r`, `w` or `rw`
    /// (the default). An empty text is an empty list.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut ranges = Vec::new();
        for entry in text.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let (range, mode) = entry.split_once(':').unwrap_or((entry, "rw"));
            let (read, write) = match mode.trim().to_ascii_lowercase().as_str() {
                "r" => (true, false),
                "w" => (false, true),
                "rw" | "wr" => (true, true),
                other => {
                    return Err(format!(
                        "watch \"{entry}\": mode \"{other}\" is not r, w or rw"
                    ));
                }
            };
            let (lo, hi) = match range.split_once('-') {
                Some((a, b)) => (number(a)?, number(b)?),
                None => {
                    let a = number(range)?;
                    (a, a)
                }
            };
            if lo > hi {
                return Err(format!(
                    "watch \"{entry}\": the range ends before it starts"
                ));
            }
            ranges.push(Watch {
                lo,
                hi,
                read,
                write,
            });
            if ranges.len() > MAX_WATCHES {
                return Err(format!("at most {MAX_WATCHES} watches"));
            }
        }
        Ok(Self { ranges })
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn ranges(&self) -> &[Watch] {
        &self.ranges
    }

    /// The first watch an access of `size` bytes at `addr` touches.
    pub fn hit(&self, addr: u32, size: u32, write: bool) -> Option<Watch> {
        let (a, b) = (addr & 0xFF_FFFF, (addr & 0xFF_FFFF) + size.max(1) - 1);
        self.ranges
            .iter()
            .copied()
            .find(|w| (if write { w.write } else { w.read }) && a <= w.hi && b >= w.lo)
    }
}
