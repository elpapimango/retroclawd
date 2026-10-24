//! Sega Mega Drive / Genesis. Design: `docs/superpowers/specs/2026-10-24-sega-genesis-design.md` in the
//! retroclawd repository; every hardware fact used is listed in `docs/hardware-facts.md`.
#![forbid(unsafe_code)]

mod bus;
mod config;
mod io;
mod machine;

pub use bus::{Bus, Stop};
pub use config::{Config, LINE_CLOCKS, M68K_DIVIDER, Region, Z80_DIVIDER, frame_clocks};
pub use io::{BUTTONS, Io, PadKind};
pub use machine::Genesis;
pub use mc68000;
