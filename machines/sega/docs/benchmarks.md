# Mega Drive plugin speed

`scenarios/bench.py` runs a game headless for 1000 frames three times and reports the fastest (Windows 11, release build, sound on,
no window; one run varies by about 10% with what else the machine does). A frame is 1/59.92 s.

| game | before the render work | now (accurate) | now (fast) |
|---|---|---|---|
| Sonic the Hedgehog 2 | 247 frames/s, 4.1x real time | 315 frames/s, 5.2x | 311 frames/s, 5.2x |
| Sonic the Hedgehog | 228, 3.8x | 297, 5.0x | 290, 4.8x |
| Ecco the Dolphin | 217, 3.6x | 272, 4.5x | 274, 4.6x |

Where a frame of Sonic 2 spent its 4.05 ms before the work (measured by switching parts off):

| part | share |
|---|---|
| picture (planes, sprites, colours) | 55% |
| 68000, bus, VDP bookkeeping, DMA | 26% |
| YM2612 (41 ns a chip cycle, 21334 a frame) | 12% |
| Z80 | 6% |

What changed: the picture decoded every pixel's tile again (8 times per tile); it now decodes a tile row once and uses it for its
eight pixels (+35%), and the VDP returns at once from `advance` when no line or interrupt is due. A look-up table for the
brightness levels, a reused sprite buffer and per-cell window/scroll tests changed nothing measurable.

`--execution-mode fast` leaves out the 68000's bus refresh, the Z80's accesses to its bus and the VDP's FIFO slots. Those cost
little to model, so fast mode is not faster; its purpose is behaviour (what emulators without these do), not speed.

Not done: nothing here has been profiled with a sampling profiler (none is installed), the figures come from switching parts off.
The 68000 core and the YM2612 are the next candidates; the YM2612 is a per-cycle pipeline and 41 ns a cycle is already close to what
that costs.
