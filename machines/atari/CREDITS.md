# Credits and sources

- **EmuTOS 1.4** (GPL-2.0, https://emutos.sourceforge.io/, files at
  https://sourceforge.net/projects/emutos/files/emutos/1.4/): the firmware the machine is developed and tested
  with. Fetched by the host's `--firmware auto` from the official zip, never committed. Facts used from the release
  (readme.txt in `emutos-192k-1.4.zip`, and the image header): the 192 KB images are for the ST, STf and Mega ST,
  start with `BRA.S` to `$30`, and have their reset PC at `$00FC0030` (ROM at `$FC0000`).
- **The Atari ST Internals** (transcription, https://atarimuseum.ctrl-alt-rees.com/computers/16bits/STINTERN.htm):
  memory map, vector table, MFP and ACIA assignments, register addresses. A secondary source; every fact used in code
  is checked against a primary source where one is available and cited there.
- **MC68000**: the core is `mc68000` of retroclawd-crates (its own CREDITS).
