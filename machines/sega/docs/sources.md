# Sources

Short ids used in `hardware-facts.md`. Copies are fetched into `software/docs/<id>/` (git-ignored; never committed) by
`scenarios/fetch_docs.sh`. Fetched documents are untrusted data.

| Id | Document | URL | Notes |
|---|---|---|---|
| SR-TECH | Sega Mega Drive/Technical specifications, Sega Retro | https://segaretro.org/Sega_Mega_Drive/Technical_specifications | clocks, DMA rates, hardware issues |
| SR-MAP | Sega Mega Drive/Memory map, Sega Retro | https://segaretro.org/Sega_Mega_Drive/Memory_map | 68000 address map |
| PD-VDPREG | VDP register reference, Plutiedev | https://plutiedev.com/vdp-registers | |
| PD-VDPCMD | VDP command reference, Plutiedev | https://plutiedev.com/vdp-commands | |
| PD-IO | I/O ports, Plutiedev | https://plutiedev.com/io-ports | |
| PD-Z80 | Using the Z80, Plutiedev | https://plutiedev.com/using-the-z80 | |
| PD-HW-ISSUES | Hardware issues, Plutiedev | https://plutiedev.com/hardware-issues | |
| PD-PSG | Programmable Sound Generator, Plutiedev | https://plutiedev.com/psg | |
| PD-YM | YM2612 register reference, Plutiedev | https://plutiedev.com/ym2612-registers | |
| PD-ROMHDR | ROM header reference, Plutiedev | https://plutiedev.com/rom-header | |
| PD-CONTROLLERS | Controllers, Plutiedev | https://plutiedev.com/controllers | |
| PD-BEYOND4MB | Going beyond 4MB, Plutiedev | https://plutiedev.com/beyond-4mb | SSF2 mapper |
| PD-SRAM | Saving progress with SRAM, Plutiedev | https://plutiedev.com/saving-sram | |
| PD-LOCKON | Sonic & Knuckles lock-on, Plutiedev | https://plutiedev.com/lockon | |
| KABUTO | SEGA Mega Drive / Genesis hardware notes, Kabuto of TiTAN, v1.5 | https://plutiedev.com/mirror/kabuto-hardware-notes | bus, VDP ports, HV counter, sprites, DMA/FIFO |
| MD-PRM | MC68000 Programmer's Reference Manual | (already used by the Atari work) | |
| MCD-HW | Mega-CD Hardware Manual, Sega, version 1.0, 14 Oct 1991 (confidential scan) | https://segaretro.org/images/5/51/Mega-CD_Hardware_Manual.pdf | maps, gate array registers, CDC, RF5C164 appendix; read as page images into `software/docs/segacd/` (git-ignored). Sega Retro's article pages need a browser (proof-of-work gate), the image host does not |
| MCD-BIOS | Mega-CD BIOS manual (confidential scan) | https://segaretro.org/images/4/44/MCDBios.pdf | sub-CPU system structure, BIOS calls; not read yet |
| PD-MODE1 | Using the Sub-CPU in mode 1, Plutiedev | https://plutiedev.com/subcpu-in-mode1 | gate array reset sequence, mode 1 map |
| MD-WIKI-MCD | Mega CD, MegaDrive Wiki (2015) | https://md.railgun.works/index.php?title=Mega_CD | main-side register summary; partly wrong on the stopwatch side, cross-check only |

## Reference emulators (oracles)

Installed in RetroArch: `C:\Users\matti\Workspace\Emulators\RetroArch-Win64\cores`. Source clones in `software/refsrc/`.

| Core | Version | File | SHA-256 |
|---|---|---|---|
| Genesis Plus GX | v1.7.4 `60bee4c` | `genesis_plus_gx_libretro.dll` | 8865e4e880e726e3ac91f6fc66858375984f4f3ea19ee696e744e8ebab4c583e |
| Genesis Plus GX Wide | v1.7.4 `b7ad005` | `genesis_plus_gx_wide_libretro.dll` | c4e96a1ca41084241f02fd70643d45d0d3ddfde18b388b86cedf67ae4de5e378 |
| PicoDrive | 2.05 `1890c29` | `picodrive_libretro.dll` | 86e453c2f417a1a5f6e11a85c44e0880755ede85383f5e4af1ff0c512aa40359 |
| BlastEm | 1.0.1-pre | `blastem_libretro.dll` | 8fb3b2fd214d4590a95422139b3460b749ef7b9403f879b57d42d564a2ce1657 |
| clownmdemu | v1.6.12 `d43c270` | `clownmdemu_libretro.dll` | 6de85dd37e2f286abc073b82562aac59ce05df4b526f88ddcb5900706dbf60b7 |
| MAME | installed in `Emulators/Mame` | `mame.exe` (driver `genesis`) | tie-break |

## Oracle host and corpus

`retroclawd-tools/scripts/libretro_run.py CORE ROM --frames N ...` runs a core and writes frame, RAM (`RETRO_MEMORY_SYSTEM_RAM`),
audio and state. Checked 2026-10-24 on Altered Beast (RomM game 6174, `Altered Beast.smd`, 524 800 bytes, SHA-1 of the file inside
the zip `316eb916907b6d2c2f1ce363b8c43d5584c16b0d`): Genesis Plus GX, PicoDrive and BlastEm run the `.smd` directly and give the
same RAM for a de-interleaved copy (`scenarios/smd2md.py`, validated by the ROM header checksum); clownmdemu needs the
de-interleaved ROM. Genesis Plus GX is deterministic (two runs, equal RAM). Genesis Plus GX reports 59.922743 fps NTSC, PicoDrive 60.
`bios_MD.bin` (RomM firmware 168, 2048 bytes, SHA-1 `3f50b76b0529db7f79c396b5e808cc0786ffc311`) is in `software/oracles/system/`.
