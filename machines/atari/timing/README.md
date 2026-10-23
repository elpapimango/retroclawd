# Timing probes

68000 programs that measure the ST's timing from the inside; `../docs/timing-oracle.md` says how they are built, run in
Hatari and compared. `build.sh` builds `software/scenarios/timing/pNN.st` (not committed; `p22` also needs the blank disk
`blank-hd.img` it makes) with the tools of `retroclawd-tools` (`rcasm`, `rcdisk`). The numbers follow the order the probes
were written in; the Phase 3 plan's table is indicative only (its P03, P05, P08 and P11 to P13 slots were folded into
others or renumbered, see the ledger in the plan's workspace and `docs/hardware-facts.md`, "Timing (Phase 3)").

| Probe | Measures | Result (bytes), VBLs | Test in `tests/timing_probes.rs` | Manifest |
|---|---|---|---|---|
| P01 `p01_smoke` | the harness: constant bytes in RAM | 16, 600 | `p01_*` | P01 |
| P02 `gen_p02.py` | instruction timing and interrupt placement: 48 loop variants, one per frame, the counts of a loop between HBLs | 4 + 48 x 128 | `p02_*` (the mean) | P02 (gap) |
| P04 `p04_video_counter` | the video address counter along a line | | `p04_*` | P04 (gap) |
| P06 `p06_mfp_timer` | MFP timer data register reads after a control write | | `p06_*` | P06 (gap) |
| P07 `p07_palette_strip` | a palette write at every clock of a line | | S12 (picture) | S12 |
| P09, P10, P11 `gen_p09.py` | left border (resolution), right border (sync), control: two writes swept along a line, the video counter after | 4 + 128 x 4 | `p09_and_p10_*` | P09, P10 (gaps) |
| P12, P13 `gen_p12.py` | top and bottom border: a 60 Hz line swept down the frame, and its control | 4 + 256 x 7 | `p12_*` | P12 (gap) |
| P14 `p14_mono_frame` | the mono frame: loops with the HBL masked, HBLs per frame | 66, 700 | `p14_*` | P14 (gap) |
| P15 `p15_mono_counter` | the video counter at every HBL of a mono frame | 4 + 501 x 3, 700 | `p15_*` | |
| P16 `p16_dma_stall` | a counted loop while nine sectors are read by DMA: the stall per burst | 4 + 30 x 8, 700 | `p16_*` | P16 (gap) |
| P17 `p17_floppy_rotation` | the index pulse and the header pitch | 196, 800 | `p17_*` | P17 (gap) |
| P18 `gen_p18.py` | what an access to each of 12 registers (and 4 ACIA alignments) costs | 196, 800 | `p18_*` | P18 (gap) |
| P19 `p19_floppy_step` | 20-track seeks at the four step rates | 24, 800 | `p19_*` | P19 (gap) |
| P20 `p20_ikbd_latency` | the keyboard controller's answer latencies and byte spacing for 12 commands | 484, 3000 | `p20_*` | P20 (gap) |
| P21 `p21_mfp_serial_tx` | the MFP transmitter's byte time at 9600 and 1200 baud | 68, 1500 | `p21_*` | P21 (gap) |
| P22 `p22_acsi_timing` | ACSI commands: completion at once, the DMA address after (blank disk on target 0) | 36, 1500 | `p22_*` | P22 (gap) |
| P23 `p23_boot_time` | TOS's 200 Hz counter when the floppy boot reaches the body | 52, 600 | `p23_*` | |
| P24 `p24_floppy_turnaround` | eight single-sector reads in a row for twelve pauses | 52, 2500 | `p24_*` | |
| P25 `p25_floppy_ids` | the spacing of consecutive headers (Read Address) | 132, 1500 | `p25_report` | |
| P26 `p26_floppy_spinup` | the first command after idle times of 0.25 to 5 s | 44, 3000 | `p26_*` | |

`known-gap` rows say in the manifest what differs and where the evidence is: the probes' exact byte equality is the
manifest's comparison, the tests next to it check the property each probe is about.
