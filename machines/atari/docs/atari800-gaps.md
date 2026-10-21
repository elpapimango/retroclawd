# Atari 800 experimental acceptance

## Verified slice

`atari-800` is a 48 KiB Atari 800 using the shared NMOS 6502 Core and MOS 6520
PIA, plus initial ANTIC, GTIA and POKEY Chips. It is not an XL/XE profile.
PAL is the default; NTSC is separate. Fast mode batches instruction accesses;
accurate mode advances Chips on every read/write/dummy cycle and stalls reads,
not writes, for ANTIC. Neither mode claims cycle-exact Atari compatibility.

Private verification used explicitly identified, permissively licensed
AltirraOS 800 3.11 and the replacement BASIC ROM that displays **1.59**.
The upstream BASIC C source's older label is not the version shown at boot.
The exact SHA-1/SHA-256 values and sources are in `atari-800.intake.toml`.
No original Atari ROM was available from the configured RomM. No ROM bytes,
reference binaries, diagnostic binaries, goldens or private captures are committed.

The real release Host loads `atari_8bit_machine_plugin.dll`, executes the OS
without PC or RAM injection and reaches BASIC. The ROM-gated test
`replacement_rom_reaches_basic_and_types_through_pokey` boots all four
PAL/NTSC x fast/accurate combinations, types `PRINT 12*34` through POKEY,
reads `408` from the real BASIC screen, restores deterministic future State
and returns to Ready after the Atari 800's warm-reset NMI.
The Host's `--type` path independently executes the same expression.

The manifest records S1, S3 Program, S4 pure tone, S5 State and S6 timing
for both Regions and execution modes. S2 reference input and S7 diagnostics
remain known gaps, not passes. S4 establishes the documented pitch and an
audible signal, **not reference RMS or analogue mixer calibration**.
The executed manifest result is **24 passed, 0 failed, 8 known gaps, 0 closed**.
No game/demo/application compatibility claim follows from these small scenarios.

## Reference capture

Reference: Atari800 7.2.1 Win64, source
`63a381884175e3cbfc745ff00dfb3d70d928eb50`; portable ZIP SHA-256
`82eb30836613a03b3a82d594da71b47667f2eef844d9cd808e3a367f250613ab`.

`retroclawd-tools/scripts/capture_atari800_screen.py` uses a temporary config,
explicit OS/BASIC files, `-nopatchall`, and the reference's own monitor.
It alternates `BLINE 156` / `CONT` / `BLINE 0` / `CONT`: repeated `CONT`
with one BLINE stops on successive instructions within that scanline instead
of advancing whole frames. A process-scoped F10 message asks Atari800 itself
to write its native PNG within two further frames. No foreground focus or
desktop screenshot is needed. The adapter is Windows-only and fails explicitly
elsewhere. Its `.monitor.txt` transcript remains beside each private capture.

Example from a Windows shell, with real paths in place of the placeholders:

```text
python capture_atari800_screen.py OS.rom S1-pal.png --basic BASIC.rom --atari800 atari800.exe --frames 300 --memory 0x9c40:80 --memory-out S1-pal.bin
python capture_atari800_screen.py OS.rom S1-ntsc.png --basic BASIC.rom --atari800 atari800.exe --region ntsc --frames 300 --memory 0x9c40:80 --memory-out S1-ntsc.bin
python capture_atari800_screen.py OS.rom S3-program.png --basic BASIC.rom --atari800 atari800.exe --program atari800-program.bin --org 0x2000 --frames 5 --memory 0x4000:256 --memory-out S3-program.bin
```

For S3 only, the reference's `READ` and `CONT address` deliberately load and
start the raw Program, matching the Host's `--program --org` behavior.
That is **not** used to obtain the cold-boot golden.

S1 checks every pixel of the stable 320x16 banner and Ready prompt, plus the
same complete 80-byte screen-memory range from the reference. It excludes the
following blinking-cursor row rather than tolerating altered glyphs.
The reference's actual PAL PNG is 336x240 with text at y=24; NTSC is
336x224 with text at y=16. The Host canvas is 384x240 in both Regions.
Those measured geometry differences determine the crops. The initial NTSC
run failed at MAD 27.18 when a PAL crop was mistakenly applied; correcting the
NTSC rectangle retains the original 0.01 threshold. Palette normalization
compares content, not the currently uncalibrated GTIA hue.

## S2 reference keyboard

The reference's own screenshots and RAM captures establish the blocker:
posting Win32 key-down/up messages between monitor-controlled frames produces
no printable characters; Return advances the input cursor. Explicit WM_CHAR
messages and bringing the matching SDL window to the foreground did not fix
printable input. Private artifacts are `reference-screen-monitor.txt`,
`screenshot-ram.bin` and the corresponding native PNG; local Host artifacts
are `host-typed.png` and screen RAM at `$9C40`.

The Host screen contains the real BASIC line `PRINT 12*34` and result `408`.
The reference screen still contains only the initial banner, Ready and moved
cursor, so it is **not** a valid typed golden. The manifest omits that golden,
reports S2 as not run, and never substitutes the Host's picture. Further work
must use a working reference input path and capture the actual result.

## S3 Media limits

Only raw Program Media is advertised. `scenarios/build.sh` uses existing
`rcasm` to assemble `scenarios/atari800/program.asm` at `$2000`.
The 46-byte fixture SHA-256 is
`e92710e9354bdab87da92692d23e671280bf978c7d6f9438ce4e3e6de4e7db0e`.
The reference executes it and writes all 256 bytes at `$4000`; the golden
SHA-256 is
`7aefa1511529d5ae13d675c4f032d61d5b0c3df71a31e179d3384c7a8ae734dd`.

XEX loading, ATR disks, CAS tapes, CART headers/banking and external SIO devices
are not implemented or advertised. The optional BASIC ROM is a fixed 8 KiB
cartridge window, not a general cartridge loader. Unsupported Media requests
return an explicit Plugin error. No silent loader trap or generic PRG parser
is used as an Atari format implementation.

## S4 audio and S6 timing limits

The raw Program selects AUDF1=150, AUDC1=$AF, AUDCTL=0, SKCTL=3 and STIMER.
Altirra Hardware Reference chapter 5 gives pure tone
`CPU / (2 * 28 * (AUDF + 1))`: 209.7265 Hz PAL and 211.657 Hz NTSC.
The manifest permits 0.5% for finite-window pitch estimation and requires
amplitude at least 0.01, not an invented reference RMS baseline.
An independent firmware `SOUND 0,150,10,15` run measured 209.72 Hz,
DC-free RMS 0.1250 and tone amplitude 0.1591 in its 10-14 second window.
No reference audio recording was captured; analogue amplitude, mixing and
distortion fidelity remain unverified.

ANTIC uses 114 clocks per line, 312 PAL / 262 NTSC lines, or 35568 / 29868
clocks per Frame. The integer clock API floors the crystal-derived rates by
0.5 Hz to 1773447 / 1789772; the reported exact FrameRate uses that same
integer clock divided by the raster period. This is a stated rounding limit,
not a claim of the exact physical crystal frequency.

## S7 Acid800

Acid800 1.2 was fetched privately from
<https://www.virtualdub.org/downloads/Acid800-1.2.7z>.
Archive SHA-256:
`4074e6a40db1833e97603e334ca130f5923bff34a0d4f8a83e2ecf9617de1b20`.
The included `README.html` licenses the suite under MIT, copyright 2010-2022
Avery Lee, and describes the main suite as a bootable ATR that incrementally
loads tests. The artifact is `out\Release\Acid800\acid800.atr`.

An actual Host attempt with that ATR and `--headless --frames 300 --disk`
exited 1 with:

```text
unsupported Atari 800 operation: only raw Program Media, keyboard, digital joysticks, State and the debug contract are implemented
```

The private `acid800-host-refusal.txt` records that failure. **The suite has not
executed; there is no diagnostic score or result golden.** ATR/SIO support is
the immediate dependency for this route. Known timing/video gaps below are
additional reasons not to imply that Acid800 would pass once it can load.

## Remaining hardware limits

- ANTIC: no P/M DMA, horizontal/vertical scrolling, abnormal DMA, exact refresh
  displacement or mid-scanline GTIA color preservation.
- GTIA: no players/missiles, collision generation, special modes 9/10/11 or
  composite artifacting; the palette is an analytical approximation.
- POKEY: period-level timers rather than complete shared-divider pipelines,
  incomplete linked-low events/two-tone/serial clocking, no serial input or
  external clocks, linear audio mixer, unconnected paddles only.
- IRQ/NMI delivery remains at instruction boundaries. Warm reset uses the
  Atari 800's ANTIC RNMI and preserves RAM rather than resetting the CPU.
- No XL/XE MMU, banked RAM, self-test ROM or additional Machine profiles yet.

C16/Plus4 remain parked. XL/XE work must not be presented as enabled by the
Atari 800 boot alone.
