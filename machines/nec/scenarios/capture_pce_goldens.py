#!/usr/bin/env python3
"""Fetches the games `pcengine.manifest.toml` runs (from the owner's RomM server, platform 130) and captures the
pictures it compares with from Beetle PCE Fast, Mednafen's PC Engine core, through its libretro build. Prints the
SHA-256 of every golden.

    capture_pce_goldens.py [--goldens DIR] [--no-fetch]

Paths: RC_ORACLE_CORES (the RetroArch cores folder), RC_ORACLE_SYSTEM (its system folder), RCD_BIN (any retroclawd
build that has the pcengine linked: it fills the RomM cache), ROMM_URL and ROMM_TOKEN. The games land in
machines/nec/software/roms/ and the test cards must be built first (`build.py`). The goldens are never committed;
each is pinned by hash in the manifest, which is how a changed capture is noticed.
"""
import argparse
import hashlib
import os
import subprocess
import sys
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
NEC = HERE.parent
REPO = NEC.parent.parent
CORES = Path(os.environ.get("RC_ORACLE_CORES", r"C:\Users\matti\Workspace\Emulators\RetroArch-Win64\cores"))
SYSTEM = Path(os.environ.get("RC_ORACLE_SYSTEM", r"C:\Users\matti\Workspace\Emulators\RetroArch-Win64\system"))
RUNNER = REPO / "rctools" / "scripts" / "libretro_run.py"
RCD = Path(os.environ.get("RCD_BIN", REPO / "target" / "debug" / "rcd.exe"))
CACHE = Path(os.environ.get("LOCALAPPDATA", "")) / "retroclawd" / "cache" / "romm"
ROMS = NEC / "software" / "roms"
SCEN = NEC / "software" / "scenarios"

# name -> RomM id (platform 130, TurboGrafx-16 / PC Engine)
GAMES = {
    "airzonk": 7161,
    "gradius": 7299,
    "samuraighost": 7466,
    "cadash": 7207,
    "bomberman93": 7189,
    "sf2ce": 7497,
    "ladysword": 7347,
}

# (golden file, ROM path, frame, libretro input script lines or None)
PICTURES = [
    ("S1-airzonk-160.png", ROMS / "airzonk.pce", 160, None),
    ("S1-airzonk-600.png", ROMS / "airzonk.pce", 600, None),
    ("S2-pad.png", SCEN / "pce-input.pce", 40, "10 a,start"),
    ("S3-gradius-600.png", ROMS / "gradius.pce", 600, None),
    ("S3-samuraighost-600.png", ROMS / "samuraighost.pce", 600, None),
    ("S3-cadash-600.png", ROMS / "cadash.pce", 600, None),
    ("S3-bomberman93-600.png", ROMS / "bomberman93.pce", 600, None),
    ("S3-sf2ce-900.png", ROMS / "sf2ce.pce", 900, None),
    ("G-ladysword-600.png", ROMS / "ladysword.pce", 600, None),
]


def fetch(name, game_id):
    target = ROMS / f"{name}.pce"
    if target.exists():
        return
    folder = CACHE / f"rom-{game_id}"
    if not folder.is_dir():
        subprocess.run([str(RCD), "pcengine", "--rom", f"romm:{game_id}", "--headless", "--frames", "1"],
                       capture_output=True, timeout=600)
    path = next(folder.iterdir())
    data = path.read_bytes()
    if data[:2] == b"PK":
        z = zipfile.ZipFile(path)
        members = [m for m in z.namelist() if m.lower().endswith((".pce", ".bin", ".tg16"))] or z.namelist()
        data = z.read(members[0])
    ROMS.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--goldens", default=str(REPO.parent / "goldens"))
    ap.add_argument("--no-fetch", action="store_true")
    args = ap.parse_args()
    if not args.no_fetch:
        for name, game_id in GAMES.items():
            fetch(name, game_id)
    out = Path(args.goldens) / "pcengine"
    out.mkdir(parents=True, exist_ok=True)
    core = CORES / "mednafen_pce_fast_libretro.dll"
    for golden, rom, frame, inputs in PICTURES:
        target = out / golden
        cmd = [sys.executable, "-I", str(RUNNER), str(core), str(rom), "--frames", str(frame), "--device", "0=1",
               "--system-dir", str(SYSTEM), "--out-frame", str(target)]
        if inputs:
            script = out / (golden + ".input.txt")
            script.write_text(inputs + "\n")
            cmd += ["--input", str(script)]
        subprocess.run(cmd, check=True, capture_output=True, text=True, timeout=900)
        print(f"{golden:28} {hashlib.sha256(target.read_bytes()).hexdigest()}")


main()
