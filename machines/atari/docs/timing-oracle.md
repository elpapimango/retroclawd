# The timing oracle (Phase 3)

No document reachable from here gives the ST's cycle-level timing (Atari's hardware specification is behind a
Cloudflare challenge). Every timing fact of Phase 3 is therefore **observed**: a small 68000 program (a *probe*) runs in
Hatari 2.6.1 and in retroclawd, and the memory it leaves behind must be byte-identical. Facts found this way are
labelled `[O]` in `hardware-facts.md` and name their probe. Hatari is a black box: its source is never read, nothing of
it is copied.

## A probe

`timing/pNN_name.s`, assembled by `rcasm -c 68000 -f bin` to run at `$90000`. `timing/mkprobe.py` puts it on a blank
720K floppy whose boot sector is the shared stub `timing/stub.s` (`rcdisk boot`): the stub reads the body from side 0 of
cylinders 1.. with the XBIOS `Floprd` call (which names cylinder, side and sector; a BIOS `Rwabs` maps logical sectors
through TOS's own geometry and read other sectors than expected) and jumps to it. EmuTOS reads one sector per disk
revolution, so a track costs about two seconds of machine time and the boot about five: a probe says in its header
how many VBLs it needs.

The body runs in supervisor mode, sets `SR=$2700` once its setup is done (nothing else may run), writes its results to
`$C000-$FFFF` (16 KB, below the 64 KB limit of the scenario runner's memory dump; nothing else uses it once interrupts are
masked) and ends in `bra.s *`.

## The two sides

- Hatari: `retroclawd-tools/scripts/capture_hatari_mem.py ROM FLOPPY VBLS ADDR LEN OUT.bin`. It starts Hatari with a debugger
  script (`--parse`) holding a breakpoint at VBL number VBLS whose action `savebin`s the memory and quits. Options used:
  `--machine st --memsize 1 --cpu-exact true --video-timing ws3 --monitor rgb --sound off --fast-forward on`, and an empty
  throw-away `--configfile`, so the user's configuration cannot change the result. `ws3` is Hatari's own default wake-up
  state (manual: "default ws3"); the other states are not modelled (plan decision 3). Verified 2026-10-22 with Hatari 2.6.1.
- retroclawd: `rc-scenarios run machines/atari/atari-st.manifest.toml` (a `memory` scenario per probe: the floppy as `disk`,
  `start`, `len`, the golden, `compare = exact`), and `tests/timing_probes.rs` (`run_probe`, `assert_matches_hatari`),
  which also takes the accuracy mode.

Goldens are Hatari's dumps (`RC_GOLDENS/atari-st/timing/pNN.bin`), never committed. To regenerate one:

```
timing/build.sh
python retroclawd-tools/scripts/capture_hatari_mem.py <etos192uk.img> software/scenarios/timing/pNN.st VBLS 0xC000 LEN \
    ../goldens/atari-st/timing/pNN.bin --hatari <hatari.exe>
```

Then put the dump's SHA-256 in the manifest.

## Findings while building the harness

- Hatari's `--configfile` needs an existing file; `--monitor` takes `mono/rgb/vga/tv`.
- A boot sector is at most 480 bytes (`rcdisk boot`), hence the stub and the body on the following tracks.
- The first version read the body with `Rwabs` from logical sector 20 and got zeros in both emulators: EmuTOS seeks to
  the track `sector / 9`, not the cylinder, for that call. Explicit geometry (`Floprd`) removes the question.
