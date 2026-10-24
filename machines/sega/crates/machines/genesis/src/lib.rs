//! Sega Mega Drive / Genesis. Design: `docs/superpowers/specs/2026-10-24-sega-genesis-design.md` in the
//! retroclawd repository. Skeleton: the real machine arrives task by task (plan `2026-10-24-sega-genesis.md`).
#![forbid(unsafe_code)]

mod machine;

pub use machine::Genesis;
