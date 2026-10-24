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

## Reference emulators (oracles)

Installed in RetroArch: `C:\Users\matti\Workspace\Emulators\RetroArch-Win64\cores`. Source clones in `software/refsrc/`.

| Core | Version | File | SHA-256 |
|---|---|---|---|
| Genesis Plus GX | v1.7.4 `60bee4c` | `genesis_plus_gx_libretro.dll` | recorded by `scenarios/oracle_info.py` |
| Genesis Plus GX Wide | v1.7.4 `b7ad005` | `genesis_plus_gx_wide_libretro.dll` | |
| PicoDrive | 2.05 `1890c29` | `picodrive_libretro.dll` | |
| BlastEm | 1.0.1-pre | `blastem_libretro.dll` | |
| clownmdemu | v1.6.12 `d43c270` | `clownmdemu_libretro.dll` | |
| MAME | installed in `Emulators/Mame` | `mame.exe` (driver `genesis`) | tie-break |
