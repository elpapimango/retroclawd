# PC Engine: notes for review

Written at the end of the first implementation run (branch `pcengine`, 2026-11-22). Things the next person should look at, most important first.

1. **Hardware facts are from memory.** `hardware-facts.md` rows tagged `[H]` were written from the author's knowledge of the HuC6280/HuC6270/HuC6260 documentation, not
   re-read during the run (the environment had no document fetch tried). Many were confirmed by the reference ([T]/[O]); the unconfirmed ones are listed
   under `[H?]` (collision for sprite 0 only, noise period, LFO, I/O port bit 6 polarity, VRAM DMA time). Check them against a primary source and add the URLs
   to `CREDITS.md`.
2. **No HuC6280 conformance suite.** Correctness rests on unit tests of the semantics (9 in the crate) and agreement with the reference over 30 games. Instruction
   cycle counts (fact H6) were written from memory; RAM counters in Air Zonk agree with the reference within 2 percent over 100 frames, which bounds them loosely. A
   per-instruction timing check needs a trace from a reference with one (Geargrafx has a debugger; not tried).
3. **Frame window.** The picture is the reference's window (3 border lines + 240; cropped width 256/352/512), not what a television shows for taller displays.
   The first-active-line formula (`VSW + VDS + 2`) was fitted to the reference's row positions on one game and is consistent on thirty; mid-frame changes of
   `VPR` and the display-start register (`HSR` bits 8-14) are not modelled.
4. **Sound level** is calibrated on a single channel against the reference (`audio_compare.py`); game music through several channels was not compared beyond a rough level.
5. **Defects found by the sweep and fixed, worth a second look:** status flags only raised with their enable bit (V4), the frame length from `VPR/VDW/VCR` (V6), `CLI` delay (H9),
   the 384 KB two-chip mapping (M7). If another game misbehaves look here first.
6. **Not done** (see `pcengine-gaps.md`): CD-ROM, SuperGrafx, backup RAM (Lady Sword), HuC6280 `rcasm/rcdis` targets, host gamepad layout for the new machine
   (keys are named `Joy1I`, `Joy1II`, `Joy1Select`, `Joy1Run`; the default keyboard binding is Z/X, right shift, enter; no pad layout was added in `gui/pad_layouts.rs`).
7. **Security review:** an automated review of the first commit flagged a possible underflow (row computed from a live register); fixed by returning the row from `start_line`. Untrusted
   ROM sizes are limited to 2.5 MB + header; every VRAM/ROM index is masked.
8. **Process notes:** the work is on branch `pcengine`; nothing was pushed to `master`. `target/pce/*` (sweep output) is scratch.
