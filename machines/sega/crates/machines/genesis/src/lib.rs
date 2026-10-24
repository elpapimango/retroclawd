//! Sega Mega Drive / Genesis. Design: `docs/superpowers/specs/2026-10-24-sega-genesis-design.md` in the
//! retroclawd repository; every hardware fact used is listed in `docs/hardware-facts.md`.
#![forbid(unsafe_code)]

mod bus;
mod cart;
mod config;
mod io;
mod machine;
mod sound;
mod state;
mod tmss;
mod z80bus;
mod z80sys;

pub use bus::{Bus, Stop};
pub use cart::Cart;
pub use config::{Config, LINE_CLOCKS, M68K_DIVIDER, Region, Tmss, Z80_DIVIDER, frame_clocks};
pub use io::{BUTTONS, Io, PadKind};
pub use machine::Genesis;
pub use mc68000;
pub use sound::{SAMPLE_MASTER_CLOCKS, Sound};
pub use state::{MAX_STATE_BYTES, StateError};
pub use ym2612::Chip as FmChip;
pub use z80;
pub use z80sys::Z80Sys;
