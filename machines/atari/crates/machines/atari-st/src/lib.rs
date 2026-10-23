//! Atari ST, STF and STFM (one machine: the models differ in drives and the TV modulator, nothing software sees).
//! Design: `docs/superpowers/specs/2026-10-19-atari-st-design.md` in the retroclawd repository.
#![forbid(unsafe_code)]

mod acia;
pub mod acsi;
mod bus;
mod config;
mod diag;
mod dma;
mod drive;
mod events;
mod io;
mod iolog;
mod machine;
mod mfp_clock;
mod mmu;
mod psg;
mod scheduler;
pub mod serial;
pub mod state;
mod waits;

pub use acia::{NullPeer, SerialPeer};
pub use bus::{
    CARTRIDGE_BYTES, HBL_LEVEL, IoStop, MFP_LEVEL, ROM_BASE, ROM_SIZE, StBus, StopReason, VBL_LEVEL,
};
pub use config::{Accuracy, Config, RamSize, Region};
pub use dma::Dma;
pub use drive::{Drives, MEDIA_CHANGE_BYTES, StDrive};
pub use ikbd;
pub use io::{Decode, Device, decode};
pub use iolog::IoAccess;
pub use machine::{AtariSt, StatePayload};
pub use mc68000;
pub use mc68901;
pub use mfp_clock::{MFP_HZ, MfpClock};
pub use mmu::{BankSize, installed_banks};
pub use psg::Psg;
pub use scheduler::{
    CLASS_ACIA, CLASS_FDC, CLASS_MFP, CLASS_VIDEO, Event, FrameGeometry, Scheduler, geometry,
};
pub use st_floppy;
pub use st_shifter::{self, Monitor, Resolution};
pub use state::StateError;
