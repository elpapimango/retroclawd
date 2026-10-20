//! Atari ST, STF and STFM (one machine: the models differ in drives and the TV modulator, nothing software sees).
//! Design: `docs/superpowers/specs/2026-10-19-atari-st-design.md` in the retroclawd repository.
#![forbid(unsafe_code)]

mod bus;
mod config;

pub use bus::{IoStop, ROM_BASE, ROM_SIZE, StBus};
pub use config::{Accuracy, Config, RamSize, Region};
