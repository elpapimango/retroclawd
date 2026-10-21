//! Atari ST, STF and STFM (one machine: the models differ in drives and the TV modulator, nothing software sees).
//! Design: `docs/superpowers/specs/2026-10-19-atari-st-design.md` in the retroclawd repository.
#![forbid(unsafe_code)]

mod bus;
mod config;
mod diag;
mod io;
mod machine;
mod scheduler;

pub use bus::{IoStop, ROM_BASE, ROM_SIZE, StBus, StopReason};
pub use config::{Accuracy, Config, RamSize, Region};
pub use io::{Decode, Device, decode};
pub use machine::AtariSt;
pub use mc68000;
pub use scheduler::{CLASS_VIDEO, Event, Scheduler};
