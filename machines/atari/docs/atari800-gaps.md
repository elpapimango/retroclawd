# Atari 8-bit experimental acceptance

## Verified slice

`atari-800` is a 48 KiB Atari 800 using the shared NMOS 6502 Core and MOS 6520
PIA, plus ANTIC, GTIA and POKEY Chips. The same implementation also exposes
800XL/65XE (64 KiB) and 130XE (128 KiB), with their own MMU and reset behavior.
PAL is the default; NTSC is separate. Fast mode batches instruction accesses;
accurate mode advances Chips on every read/write/dummy cycle. ANTIC DMA HALT
stops all accesses; WSYNC RDY stops only reads.
Neither mode claims complete Atari compatibility. The current close-out and
remaining limits are in [XL/XE and Media close-out](#xlxe-and-media-close-out).
Earlier diagnostic tables below are retained as historical evidence, not current scores.

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

Each Machine's manifest records S1, S3 Program/Media, S4 pure tone, S5 State and S6 timing
for both Regions and execution modes. S2 reference input and S7 diagnostics
remain known gaps, not passes. S4 establishes the documented pitch and an
audible signal, **not reference RMS or analogue mixer calibration**.
The pre-close-out manifest result was **36 passed, 0 failed, 8 known gaps, 0 closed**.
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

Raw and segmented XEX Programs, writable ATR/XFD disks, standard/XEGS
cartridges and standard CAS playback are advertised.
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
it through PIA CB2 COMMAND and POKEY serial input/output. The D1:/D2: sector peripherals implement checksummed status/read/PUT/WRITE,
ACK/NAK/completion, 128/256-byte sectors, dirty/export/mark-saved and read-only
protection. There is no CPU/SIO ROM trap or disk CPU/FDC/rotation model.
The reference independently boots the ATR and loads the XEX in both Regions;
each complete `$4000:256` capture has the unchanged S3 golden hash above.
Capture adapter options are `--disk FILE` and `--xex FILE`; neither uses raw
monitor injection.

Machine State version 4 originally included the disk, serial bit/queue phase, pending
XEX lifecycle, the Core's delayed IRQ-mask sample and explicit ANTIC, GTIA,
POKEY and Machine timing extensions. It reads versions 1, 2 and 3;
legacy States restore no poll history, matching their original live-I check.
Loading a version 1 State with a disk currently attached explicitly requires
ejecting first. Older Plugins cannot read version 4: preserve original files
for rollback. Original Chip and Core serialized field layouts are unchanged.
Version 4 retains scroll/refresh/DMA history, graphics shifters and delayed
writes/output, noise and divider/borrow/resync pipelines, serial clock phase,
the sampled IRQ line, pending NMI detection and the bus latch.
Validation occurs before replacing the running Machine. Legacy POKEY countdowns
continue with their original semantics until their first reload or STIMER;
they are never clipped to the newly selected divisor. Unrecorded legacy
pipeline history cannot be recovered. Preserved published version-2 and
version-3 files load through the real Plugin and produce identical duplicate
upgraded continuations; their original hashes remain unchanged.
The ROM-gated integration
test saves during the second sector transfer and checks exact continuation in
all four Region/mode combinations.

Legacy compatibility is covered against the published version 1 field schema.
An earlier private development `host-boot.state`, written before the first
published Atari 800 commit, fails with `InvalidBooleanValue(8)` and is not
silently repaired. Its exact development schema is unverified. A separate
build of the old published Plugin for additional binary-to-binary proof was
blocked by execution approval; do not treat that additional check as passed.

State v5 and the supported CAS/CAR/XFD/D2 surfaces are described in the close-out
below. Unsupported Media requests return an explicit Plugin error. No silent
loader trap or generic PRG parser is used as an Atari format implementation.

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

The real release Host boots the ATR without ROM patches and completes all
58 test entries by 5000 Frames. The BRK-only baseline, before the fidelity
increment below, was:

| Execution | Passed | Failed | Skipped |
|---|---:|---:|---:|
| Host fast | 24 | 33 | 1 |
| Host accurate | 25 | 32 | 1 |
| Atari800 7.2.1, patches disabled, identical replacement ROMs | 25 | 32 | 1 |

These are diagnostic observations, **not acceptance passes**. The reference
itself fails hardware tests; equal totals would not establish conformance.
Host mode totals do not establish cycle-exact timing.
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

The accurate-mode BRK test now passes in both Regions. Its `cpu_bugs.s`
enables VBI just before a BRK and requires NMI to take over the BRK vector,
while retaining the BRK return address and stacked flags. Two missing pieces
caused the failure: the Core unconditionally selected the IRQ vector, and
ANTIC raised NMI at cycle 12 instead of documented cycle 8.

The Core samples `Bus::take_nmi_edge` after cycle 4, before pushing status;
BRK still pushes PC+2/B=1, IRQ still pushes PC/B=0, and both cost seven bus
cycles. A connected Bus consumes only an early edge; late edges stay pending.
Sources: NESdev's Visual6502-derived
[interrupt hijacking sequence](https://www.nesdev.org/wiki/CPU_interrupts#Interrupt_hijacking)
and Altirra Hardware Reference chapter 3, "Overlapping interrupts", plus
section 4.8, "DLI timing". ANTIC's nominal NMI now occurs at cycle 8.
Core tests inject an edge after each of the seven BRK/IRQ cycles. The Machine
regression replays the diagnostic's instruction prefix at VBI, checks the
stack frame and proves deterministic continuation from a State saved before
the BRK. Both regressions failed before the fix.

No new latch or State schema is needed: the Machine's existing serialized
Hardware NMI edge survives until consumed. CPU serde and the Plugin ABI are
unchanged. The shared pin for this increment is
`293f6d33c9a1648c199e0d6c2a67cba05a87a6b2`.
Fast mode still fails the BRK case because it advances Chips only after an
instruction; its 24/33/1 result is unchanged. Accurate mode reaches 25/32/1,
matching the reference's total, **not its complete results or conformance**.
Those BRK-only results preceded the fidelity increment below.

That increment's observed failures included ANTIC DMA/scroll/interrupt timing,
POKEY polynomial/pipeline behavior and GTIA graphics. See the current limits
below rather than interpreting that historical list as the present status.

Private `acid-brk-{pal,ntsc}-{fast,accurate}.txt` transcripts retain that baseline;
`acid-dma-*`, `acid-irq-*`, `acid-store-*` and `acid-host-*` retain preceding baselines. Existing
`--capture-at 0xfaab:A=FILE` records the replacement OS's real
E: output entry, obtained from IOCB0 ICPTL, rather than synthesizing results.
Reference monitor transcripts, native PNGs and RAM retain independent evidence.
The manifest compares the complete final 960-byte screen at `$9C40` with the
reference's actual capture: both Region goldens hash to
`9c075145199268332e04a16509c16c72a89a9b7530fed04db5f650e5d444d0c2`.
All four S7 cases execute and report a mismatch, not an uncaptured placeholder.
No golden or tolerance was weakened.

## Ten-area fidelity increment

The final release captures `acid-ten-final-{pal,ntsc}-{fast,accurate}.txt`
complete all 58 entries with identical Region totals:

| Execution | Passed | Failed | Skipped |
|---|---:|---:|---:|
| Host fast | 36 | 21 | 1 |
| Host accurate | 46 | 11 | 1 |
| Atari800 7.2.1 baseline | 25 | 32 | 1 |

Accurate mode gains 21 passing cases and fast mode gains 12. No previously
passing case regresses in either mode. Accurate mode passes every POKEY
case, normal/virtual DMA, NMIST/NMIRES, VCOUNT, WSYNC, DLI, blocked NMI,
vertical scrolling/VSCROL+NMI and normal GTIA collision/retrigger/resize/overlap.
Fast mode remains instruction-batched and does not inherit those exact-phase
claims. These are hardware-diagnostic observations, not full acceptance.

The following mechanisms are implemented, not merely scheduled:

| Area | Behavior and focused proof |
|---|---|
| NMIST/NMIRES | Mutually exclusive VBI/DLI status, retained RNMI and dominant status-update/write phases. |
| VCOUNT | Cycle-111 increment and the one-clock end-of-frame anomaly. |
| WSYNC | One following clock, read-only hold, consecutive RMW writes and horizontal-blank release. |
| DLI | Normal detection delay and the extra clock when enabled at the deadline. |
| Blocked NMI | A short pulse during status push is lost; an earlier pulse hijacks the vector without changing the stack frame. |
| DMA/refresh | Normal fetch slots, displaced/coalesced refresh, virtual DMA sampling CPU data without HALT, and current-clock GTIA collision readback. |
| Scrolling | Expanded horizontal fetch/crop, entry/interior/exit row behavior, VSCROL deadlines and scroll-region continuity across vertical blank. |
| POKEY noise | Shared 9/17-bit register, initialization shifts, RANDOM taps and per-channel delays. |
| POKEY timers | Shared prescalers, byte counters, borrow/IRQ/STIMER pipelines, linked channels, two-tone resync and serial propagation. |
| PM graphics | ANTIC one/two-line PM addressing and enable history feed live GTIA shifters, sizing, VDELAY, priorities and collision latches. |

The normal PM path has a whole-Machine DMA/collision and in-flight State
regression, not only disconnected Chip tests. All new phases are part of
State version 4. Truncated and invalid timing extensions are rejected
without mutating the running Machine.

Two integration failures required Machine/ANTIC fixes rather than tuning
GTIA to a diagnostic. Reading collision latches before the current video
clock dropped the last `$DD` collision; GTIA reads now sample after that
clock. DMA between the two writes of INC WSYNC consumed RDY's delay, but
the second write incorrectly rearmed it. The next opcode then sometimes
executed before the hold, shifting subsequent SIZE writes by one CPU clock.
Keeping the existing hold fixes both refresh alignments and the actual
resize test, without position-dependent GTIA delays. Both mechanisms have
failing-before/passing-after regressions and real diagnostic proof.

These changes do not implement abnormal DMA rings, every mid-line
DMACTL/HSCROL transition, GTIA special modes or all hardware variants.
Passing the normal mechanisms does not close every associated Acid800 test.

## XL/XE and Media close-out

Four profiles share the same Core and Chips: `atari-800` (48 KiB/10 KiB OS),
`atari-800xl` and `atari-65xe` (64 KiB/16 KiB OS), and `atari-130xe`
(128 KiB/16 KiB OS). Optional BASIC is 8 KiB. PORTB drives OS/BASIC/self-test
selection and independent 130XE CPU/ANTIC banks. Cartridge mapping wins over
BASIC. ROM writes do not reach hidden RAM. XL/XE have Help and two joysticks;
800 retains four joysticks and RNMI reset. XL/XE reset CPU/PIA and ANTIC's
resettable latches, preserving RAM, raster, GTIA and POKEY.

Private firmware proof used the intake-pinned Altirra XL OS SHA-1
`738b9590a29f424ad2c9d3f9cb1d170de79241f7`. The two original ROM-gated tests
now run all four profiles in PAL/NTSC and fast/accurate mode. They prove BASIC
input/result/reset, ATR boot, in-flight serial restore and deterministic future
State. Additional real-OS tests prove:

- SIOV writes D2 sector 1, reads it into another buffer, checks status/data,
  exports the modified ATR and marks it saved; D1 remains unchanged.
- OS cartridge entry runs a complete 256-byte memory pattern for CAR types
  1/2/12/13/14 on every profile. Unused lower cartridge bytes are `$FF`, so a
  16K/XEGS fixture does not accidentally advertise another 800 cartridge.
- CIO opens `C:`, loads all 128 bytes through the PIA motor and POKEY serial
  pins, and calls the loaded Program, which writes the same complete memory
  pattern. The initial CAS leader is 20 seconds, not the 2-second between-record
  gap: the shorter fixture was consumed during the OS startup delay.

No PC/ROM loader trap supplies Media bytes. The firmware tests call documented
OS vectors only after real cold boot. These tests are integration evidence,
not reference goldens. A separate one-record cassette boot attempt remained
in the OS cassette read loop and did not execute; cassette boot remains
unverified, not silently equated with the working CIO load path.

Writable ATR/XFD D1:/D2: preserve their export format and protection. D2,
pending write/checksum/completion, dirty flags and partial serial phase are
included in State. CAS supports FUJI/baud/data records and real 8N1 playback;
motor-off freezes its position. Standard 8K/16K and XEGS 32K/64K/128K CARs are
supported. Raw dumps have only the unambiguous 8K/16K default. Other mappers,
protected/extended disk layouts, tape recording/turbo and speed other than 1
fail explicitly.

State v5 adds separate DMA/GTIA/PIA control, profile/MMU and Media extensions.
The original serialized Chip/Core and v1-v4 field shapes remain unchanged.
All extension candidates validate before live mutation. Profiles, firmware,
Region and timing must match. Old States load only on 800; v1 requires both
disks ejected. A published-v4-shaped regression proves migration and exact
future continuation; existing v1-v3 regressions remain. The actual published
v2, v3 and stamped-v4 private files also load through the release Host and
produce byte-identical duplicate v5 continuations; their original hashes
are unchanged. Earlier pre-stamped private v4 captures fail timing decoding
with `InvalidBooleanValue(252)` and are not guessed or silently repaired.
Older Plugins cannot read v5, so preserve original files for rollback.

HSCROL now uses a bitmap shift-clock feedback ring, latched playfield
start/stop deadlines and line-buffer continuity. GTIA implements modes 9/10/11,
pseudo-E, delayed PRIOR and phantom PM data. Virtual-DMA collision bytes were
already correct; the newly exposed failure was HITCLR being applied before
its current bus clock. Completing that clock before the mirrored HITCLR
strobe prevents a stale same-clock collision from relatching.

Final actual Acid800 1.2 output, **passed/failed/skipped**:

| Machine | PAL fast | PAL accurate | NTSC fast | NTSC accurate |
|---|---|---|---|---|
| 800 | 43/14/1 | 56/1/1 | 43/14/1 | 56/1/1 |
| 800XL | 44/13/1 | 57/0/1 | 44/13/1 | 57/0/1 |
| 130XE | not separately captured | not separately captured | not separately captured | 57/0/1 |

The 800's sole accurate failure is the XL-only banking test. The 65XE uses
the same 64K decoder as XL; no separate full-suite transcript is claimed.
Fast mode's 13 phase-sensitive failures are CPU Bugs (NMI/BRK), WSYNC timing,
DLI timing, DMA pattern, blocked NMI, virtual DMA, playfield stop timing,
POKEY IRQ timing, POKEY init timing, phantom PM DMA, P/M retriggering,
player resizing and pseudo-E. These were not converted into accurate-mode
behavior or hidden by aggregate totals.

The adapter independently captured every profile in both Regions: cold
boot PNG/80-byte memory, raw/XEX/ATR/XFD/standard CAR execution and the XL/XE
diagnostic screen. Every S3 complete 256-byte capture has SHA-256
`7aefa1511529d5ae13d675c4f032d61d5b0c3df71a31e179d3384c7a8ae734dd`.
The same S1 crop/tolerance remains unchanged. Reference 65XE uses Atari800's
equivalent 64K XL mapping, explicitly selected by `--machine atari-65xe`.
Atari800 7.2.1 itself scores 25/32/1 on 800 and 26/31/1 on XL/XE.
Its captured failing screen remains pinned; a better hardware result is still
a reference mismatch, not a reason to rewrite a golden.

Each manifest also records uncaptured S2 input, S3 CAS and reference RMS.
The final release runs report **44 passed, 0 failed, 10 known gaps and
0 newly closed gaps on each of 800, 800XL, 65XE and 130XE**: 176 passes and
40 gap variants total. Per profile, the gaps are four input variants,
four diagnostic-screen mismatches, one CAS reference and one reference RMS.
These are repeated scenario variants, not forty distinct hardware bugs.

Native cassette load/execution works locally, but matching reference CIO
automation is not captured. `scenarios/build.sh` uses Tools to build
raw/XEX/standard CAR Programs, an ATR boot disk and its raw-sector XFD
equivalent. The CAR matches the native firmware test fixture byte for byte.
The CAS fixture still comes from the ROM-gated firmware test; no CAS creation
CLI exists yet. That output remains a P1 tooling limit, not a claimed pass.

## Remaining hardware limits

- ANTIC: overlapping abnormal character, playfield/PM/display-list address-AND
  effects and all consecutive abnormal bitmap-read aliases are incomplete.
  The implemented bitmap ring closes the actual HSCROL test; it is not every
  possible abnormal DMA stream.
- GTIA: composite artifacting and analogue color calibration are not implemented;
  the palette remains an analytical approximation.
- POKEY: external serial clocks, sub-cycle high-pass behavior, nonlinear
  analogue mixing and connected paddles are not implemented. Passing the
  diagnostic cases is not full analogue or temperature-variant validation.
- Ordinary IRQ/NMI delivery remains at instruction boundaries, with explicit
  pin/detection history; accurate BRK/IRQ vector selection can consume an NMI
  raised during the sequence. This is not a complete transistor-level Core
  interrupt pipeline.
- Original Atari OS ROMs, game/demo compatibility, cassette boot and native
  cassette reference automation remain unverified.

C16/Plus4 remain parked. Playbook completion with these recorded gaps is not
a claim that every Atari hardware behavior has been modeled.
