# Neo Geo hardware facts

Every hardware fact the Neo Geo code uses is a row here. Tags: `[D]` datasheet, `[H]` documentation, `[O]` observed in the
reference emulator (FinalBurn Neo, a black box through its libretro core), `[H?]` unresolved. No code, table or constant is
copied from any emulator.

| ID | Fact | Source | Used by |
|---|---|---|---|
| N1 | A cartridge set is a zip whose file extensions name the regions: `.p<n>` the 68000 program, `.s<n>` the fix layer, `.c<n>` sprite data in pairs (odd-numbered file the even bytes, the next file the odd bytes), `.m<n>` the Z80 program, `.v1<n>` the first sample region and `.v2<n>` the second; a set with only single-digit `.v` files has one region. Numbers order the files. Sets seen: Cyber-Lip (P 512 KB, S 128 KB, C 3 MB, M 128 KB, V1 2 MB, V2 512 KB), Fatal Fury (P 640 KB: a 512 KB and a 128 KB file, whose place in the address space is `[H?]`), Aero Fighters 2 (P 2 MB, C 8 MB, V 3 MB) | [O] the file names and sizes of the owner's sets (RomM platform 79) | `neo-set` |
| N2 | The BIOS is one zip of 34 files (`neogeo.zip`: `sp-s2.sp1`, `sp-u2.sp1`, `uni-bios_*.rom`, `000-lo.lo`, `sfix.sfix`, `sm1.sm1` and others); the machine picks one 128 KB 68000 BIOS, the fix tiles `sfix.sfix`, the zoom table `000-lo.lo` and the Z80 BIOS `sm1.sm1`. Which BIOS file FBNeo uses by default and what each is called by the hardware is `[H?]` | [O] the zip's listing; [H?] the roles | none yet |
| N3 | Reference output: FinalBurn Neo v1.0.0.03 (GIT2b0c1c0) libretro core reports 304 x 224 pixels, 59.18 fps, 47995 Hz audio; Cyber-Lip shows the NEO-GEO logo screen (Pro-Gear Spec) at frame 600 | [O] `libretro_run.py --info`, 2026-10-31 | scenarios |
| N4 | Clocks: master 24 MHz; 68000 at 12 MHz, Z80 at 4 MHz, YM2610 at 8 MHz, pixel clock 6 MHz; a frame of 384 x 264 pixel clocks is about 59.185 Hz | [H?] neogeodev wiki from memory, to confirm; [O] 59.18 fps agrees | none yet |
