#!/usr/bin/env python3
"""Captures the pictures `neogeo.manifest.toml` compares with, from FinalBurn Neo (its libretro core), and prints their SHA-256.

    capture_neogeo_goldens.py [--goldens DIR]        (default ../goldens relative to the application checkout)

Paths: RC_ORACLE_CORES (the RetroArch cores folder), RC_ORACLE_SYSTEM (its system folder, which holds neogeo.zip), the game sets
under software/sets. The goldens are never committed; each is pinned by hash in the manifest, which is how a changed capture is
noticed."""
import argparse
import hashlib
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SNK = HERE.parent
REPO = SNK.parent.parent
CORES = Path(os.environ.get("RC_ORACLE_CORES", r"C:\Users\matti\Workspace\Emulators\RetroArch-Win64\cores"))
SYSTEM = Path(os.environ.get("RC_ORACLE_SYSTEM", r"C:\Users\matti\Workspace\Emulators\RetroArch-Win64\system"))
RUNNER = REPO.parent / "retroclawd-tools" / "scripts" / "libretro_run.py"
SETS = SNK / "software" / "sets"

# (golden file, set below software/sets, frame)
PICTURES = [
    ("S1-cyberlip-1000.png", "cyberlip.zip", 1000),
    ("S3-fatfury-1200.png", "fatfury1.zip", 1200),
    ("S3-aerofighters2-1200.png", "sonicwi2.zip", 1200),
]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--goldens", default=str(REPO.parent / "goldens"))
    args = ap.parse_args()
    out = Path(args.goldens) / "neogeo"
    out.mkdir(parents=True, exist_ok=True)
    core = CORES / "fbneo_libretro.dll"
    for name, game, frame in PICTURES:
        target = out / name
        cmd = [sys.executable, "-I", str(RUNNER), str(core), str(SETS / game), "--frames", str(frame),
               "--system-dir", str(SYSTEM), "--out-frame", str(target)]
        subprocess.run(cmd, check=True, capture_output=True, text=True, timeout=900)
        print(f"{name:28} {hashlib.sha256(target.read_bytes()).hexdigest()}")


main()
