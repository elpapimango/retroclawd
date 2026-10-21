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
The executed manifest result is **36 passed, 0 failed, 8 known gaps, 0 closed**.
The added twelve variants cover XEX, ATR boot and disk-bearing State in both
Regions and modes. S7 now executes and differs from the independently captured
reference; S2 still has no reference golden.
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

## S3 Media and State

Raw and segmented XEX Programs and read-only ATR disks are advertised.
`scenarios/build.sh` uses existing
`rcasm` to assemble `scenarios/atari800/program.asm` at `$2000`.
The 46-byte fixture SHA-256 is
`e92710e9354bdab87da92692d23e671280bf978c7d6f9438ce4e3e6de4e7db0e`.
The reference executes it and writes all 256 bytes at `$4000`; the golden
SHA-256 is
`7aefa1511529d5ae13d675c4f032d61d5b0c3df71a31e179d3384c7a8ae734dd`.

The same build uses `rcld build` with `scenarios/atari800/program.toml` to make
a genuine segmented XEX with RUNAD `$2000`. The Plugin handles segment addresses,
ordered INITAD calls on the real Core, and final RUNAD; `--org` does not replace
the file's addresses. Later segments are not written until INITAD returns.
Loading validates all segments before mutation. A focused test saves midway
through INITAD and proves the restored loader produces the identical future.
Like raw `--program`, this starts standalone content without first booting the
OS. OS-dependent XEX applications are not yet proven.

`rcasm` builds a standard one-sector boot Program at `$0700`; `rcdisk create`
and `rcdisk add --sector 1` put it in a 720-sector ATR. The replacement OS boots
it through PIA CB2 COMMAND and POKEY serial input/output. The read-only D1:
peripheral implements checksummed status/read commands, ACK/NAK/completion,
128/256-byte sectors and explicit refusal of writes. There is no CPU/SIO ROM
trap, disk CPU, FDC/rotation model, additional drive or cassette implementation.
The reference independently boots the ATR and loads the XEX in both Regions;
each complete `$4000:256` capture has the unchanged S3 golden hash above.
Capture adapter options are `--disk FILE` and `--xex FILE`; neither uses raw
monitor injection.

Machine State version 3 includes the disk, serial bit/queue phase, pending
XEX lifecycle and the Core's delayed IRQ-mask sample. It reads versions 1 and 2;
legacy States restore no poll history, matching their original live-I check.
Loading a version 1 State with a disk currently attached explicitly requires
ejecting first. Older Plugins cannot read version 3: preserve original files
for rollback. The ROM-gated integration
test saves during the second sector transfer and checks exact continuation in
all four Region/mode combinations.

Legacy compatibility is covered against the published version 1 field schema.
An earlier private development `host-boot.state`, written before the first
published Atari 800 commit, fails with `InvalidBooleanValue(8)` and is not
silently repaired. Its exact development schema is unverified. A separate
build of the old published Plugin for additional binary-to-binary proof was
blocked by execution approval; do not treat that additional check as passed.

CAS tapes, CART headers/banking and other external SIO devices
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

The 92176-byte ATR SHA-256 is
`331541731e2e1070a59fa886cc21dfda09628f6217834d6f294be7ca06877484`.
Extract the archive locally and put that file at
`software/diagnostics/acid800.atr`; do not commit it.

The real release Host now boots the ATR without ROM patches and completes all
58 test entries by 5000 Frames. PAL and NTSC give the same respective totals:

| Execution | Passed | Failed | Skipped |
|---|---:|---:|---:|
| Host fast | 24 | 33 | 1 |
| Host accurate | 24 | 33 | 1 |
| Atari800 7.2.1, patches disabled, identical replacement ROMs | 25 | 32 | 1 |

These are diagnostic observations, **not acceptance passes**. The reference
itself fails hardware tests; equal totals would not establish conformance.
Equal Host mode totals do not establish cycle-exact timing.
The asynchronous POKEY receive test exposed timers 3/4 running while waiting
for the start bit. Holding/resetting them until that edge fixes that case;
a regression exercises the actual timer hold and subsequent receive.

The shared Core high-byte store fix makes the illegal-instruction test pass
in both Regions and modes. The actual failure wrote `$01` to `$40CD` instead
of `$00` to `$00CD`: SHY masked with the indexed high byte and never applied
the page-carry destination corruption. SHY/SHX/SHA/TAS now use the unindexed
base high byte plus one and the masked destination on carry, matching the
pinned Atari800 implementation's deterministic common case. The Core regression
failed on that exact bus write before the fix and now covers all five opcodes,
crossing/non-crossing/wrapping addresses and unchanged cycle counts. CPU and
Machine State layouts were unchanged by that store fix. Its shared pin was
`ae53253565d8bd34ebba7bdde8acebe55b9c028c`.

CLI/SEI now passes in all four Region/mode runs. The Core retains I from before
CLI/SEI/PLP for the next boundary; RTI uses restored I immediately. Every
affected Machine and Drive saves the latch in an explicit new envelope and
still reads its old files. Core serde bytes and the Plugin ABI stay unchanged.
New-format continuation, legacy loading and truncated-latch refusal have
focused regressions. This is not a cycle-exact IRQ/NMI/BRK pipeline.
The release Host also loads a preserved published version-2 Acid800 State
and produces byte-identical upgraded continuations on two runs. The original
file remains unchanged. The new shared pin is
`bf0faf6e36950b43ff1443074487fad7075320fa`.
The accurate-mode illegal-instruction timing failure was Machine arbitration,
not a missing Core cycle or a POKEY timer defect. `cpu_illtiming.s` repeats the
instruction 210 times and measures VCOUNT, allowing nine refresh clocks per
scan line. The Core already emitted and reported eight cycles for SLO `(zp,X)`.
The Machine incorrectly treated ANTIC DMA HALT like WSYNC RDY, so a write at
a refresh slot proceeded instead of stalling. Lost refresh holds made VCOUNT
advance too little, yielding the apparent seven-cycle instruction.

DMA now stops every CPU bus access; WSYNC still only holds reads. Hardware
sources are Altirra Hardware Reference chapter 2 (HALT, page 30), section 4.14
(refresh at cycles 25, 29, ..., 57, page 87) and the WSYNC RDY discussion
(pages 77-78). Atari800 7.2.1's `src/antic.c` independently reserves nine
refresh cycles on an otherwise blank line. A focused regression initially
reported six elapsed clocks for a six-cycle INC crossing refresh slot 25;
it now reports seven for either RMW write. The existing RMW-to-WSYNC regression
still passes. No Core cycle, POKEY logic, State schema or Plugin ABI changed.

Real PAL/NTSC runs now pass both ordinary and illegal-instruction timing.
The accurate-mode serial-port timing diagnostic also passes after this
arbitration fix, without a serial timing adjustment. Release workspace tests,
strict Clippy, real firmware input/reset/in-flight SIO tests and all 36
unchanged-golden manifest passes remain intact; the same eight gaps stay open.

Remaining observed failures include BRK interrupt timing,
ANTIC DMA/scroll/WSYNC/NMIST/VCOUNT/DLI, POKEY
polynomial/pipeline/IRQ/serial/two-tone behavior, PIA control bit 6 and GTIA
players/missiles/collisions/special modes. XL banking checks also run against
this non-XL Machine. These are not all newly diagnosed Machine bugs.

Private `acid-dma-{pal,ntsc}-{fast,accurate}.txt` transcripts retain the current
failures; `acid-irq-*`, `acid-store-*` and `acid-host-*` retain preceding baselines. Existing
`--capture-at 0xfaab:A=FILE` records the replacement OS's real
E: output entry, obtained from IOCB0 ICPTL, rather than synthesizing results.
Reference monitor transcripts, native PNGs and RAM retain independent evidence.
The manifest compares the complete final 960-byte screen at `$9C40` with the
reference's actual capture: both Region goldens hash to
`9c075145199268332e04a16509c16c72a89a9b7530fed04db5f650e5d444d0c2`.
All four S7 cases execute and report a mismatch, not an uncaptured placeholder.
No golden or tolerance was weakened.

## Remaining hardware limits

- ANTIC: no P/M DMA, horizontal/vertical scrolling, abnormal DMA, exact refresh
  displacement or mid-scanline GTIA color preservation.
- GTIA: no players/missiles, collision generation, special modes 9/10/11 or
  composite artifacting; the palette is an analytical approximation.
- POKEY: period-level timers rather than complete shared-divider pipelines,
  incomplete linked-low events/two-tone/serial clocking, no external clocks,
  linear audio mixer, unconnected paddles only. Basic framed serial I/O works;
  exact pipeline timing remains incomplete.
- IRQ/NMI delivery remains at instruction boundaries. Warm reset uses the
  Atari 800's ANTIC RNMI and preserves RAM rather than resetting the CPU.
- No XL/XE MMU, banked RAM, self-test ROM or additional Machine profiles yet.

C16/Plus4 remain parked. XL/XE work must not be presented as enabled by the
Atari 800 boot alone.
