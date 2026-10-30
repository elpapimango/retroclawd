#!/usr/bin/env python3
"""Captures the pictures `segacd.manifest.toml` compares with, from Genesis Plus GX (libretro), and prints their SHA-256.

    capture_segacd_goldens.py [--goldens DIR]        (default ../goldens relative to the application checkout)

Paths: RC_ORACLE_CORES (the RetroArch cores folder), the Sega CD BIOS as `bios_CD_U.bin` in software/oracles/system_cd, the
discs under software/discs (see TODO.md "Sega Mega Drive" for how they are obtained). The goldens are never committed; each is
pinned by hash in the manifest, which is how a changed capture is noticed."""
import argparse
import hashlib
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SEGA = HERE.parent
REPO = SEGA.parent.parent
CORES = Path(os.environ.get("RC_ORACLE_CORES", r"C:\Users\matti\Workspace\Emulators\RetroArch-Win64\cores"))
RUNNER = REPO.parent / "retroclawd-tools" / "scripts" / "libretro_run.py"
SYSTEM = SEGA / "software" / "oracles" / "system_cd"
DISCS = SEGA / "software" / "discs"

# (golden file, disc below software/discs, frame)
PICTURES = [
    ("S1-300.png", "dl/Dragon's Lair (USA).cue", 300),
    ("S1-600.png", "dl/Dragon's Lair (USA).cue", 600),
    ("S1-900.png", "dl/Dragon's Lair (USA).cue", 900),
    ("S3-dl-1200.png", "dl/Dragon's Lair (USA).cue", 1200),
    ("S3-silpheed-1200.png", "Silpheed (USA)/Silpheed (USA).cue", 1200),
    ("S3-lunar-1200.png", "Lunar - The Silver Star (USA)/Lunar - The Silver Star (USA).cue", 1200),
    ("S3-ffcd-2400.png", "Final Fight CD (USA)/Final Fight CD (USA).cue", 2400),
    ("S3-snatcher-1900.png", "Snatcher (USA)/Snatcher (USA).cue", 1900),
]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--goldens", default=str(REPO.parent / "goldens"))
    args = ap.parse_args()
    out = Path(args.goldens) / "segacd"
    out.mkdir(parents=True, exist_ok=True)
    core = CORES / "genesis_plus_gx_libretro.dll"
    for name, disc, frame in PICTURES:
        target = out / name
        cmd = [sys.executable, "-I", str(RUNNER), str(core), str(DISCS / disc), "--frames", str(frame),
               "--system-dir", str(SYSTEM), "--out-frame", str(target)]
        subprocess.run(cmd, check=True, capture_output=True, text=True, timeout=900)
        print(f"{name:24} {hashlib.sha256(target.read_bytes()).hexdigest()}")


main()
