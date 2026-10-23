# Benchmarks (Phase 3, Task 47)

`cargo run --release --example bench -p atari-st -- <EmuTOS image>` (`RC_BENCH_RUNS` runs each, default 5; set
`RUSTC_VERSION` at build time to record the compiler). A frame is 1/50 s of machine time, so the real-time multiple is
the frames a second over 50. The NOP/ALU mix is an endless loop of six register instructions (`ADD.L`, `SUB.L`,
`EOR.L`, `ADD.W`, `NOP`, `MOVE.L`) and a `BRA.S`.

Measured 2026-10-23: AMD Ryzen 9 5900HX (8 cores, 3.3 GHz), Windows 11, `rustc 1.98.1 (48a229cea 2026-09-01)`,
release profile with `opt-level = 3` for the Atari crates (the family workspace had `"s"` for them, which gave 7 times
real time on the desktop), median of 5 runs, the range in brackets.

| Mode | NOP/ALU mix | Idle EmuTOS desktop (frames 900 to 1200) | Budget |
|---|---|---|---|
| `fast` | 17.9 MIPS (17.7 to 18.0) | 690 frames/s (685 to 695) = 13.8x real time | at least 10x: met |
| `accurate` | 14.9 MIPS (14.8 to 15.0) | 642 frames/s (624 to 647) = 12.8x real time | at least 2x: met |

`tests/speed_smoke.rs` asserts the budget with `RC_BENCH=1` (frames 600 to 800 of the same desktop, which also holds the
boot's floppy and desktop drawing: 11.3x and 10.1x).

Before the optimisation of the Atari crates' build (`opt-level = 3`) and the floppy fix of Task 46 the same measure
read 7.1x (`fast`) and 6.5x (`accurate`). No code path of the machine was changed for speed: the budget needed none.
