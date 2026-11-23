#!/usr/bin/env python3
"""Boot sweep: runs games in the probe (our machine) and in the Mednafen PCE Fast libretro core (the black-box
reference) for N frames and compares the pictures by a 64x48 luminance grid, as the manifests do.

    sweep.py [--frames 600] [--step 14] [--ids 7161,7189 ...]

Needs the built probe (`cargo build --release -p pcengine --example probe`), a RomM server in the environment
(ROMM_URL, ROMM_TOKEN; the games are platform 130), RetroArch's cores (RC_ORACLE_CORES) and Pillow. Games are
fetched into retroclawd's cache by running `rcd` once on them (RCD_BIN, default target/debug/rcd.exe: any build with the pcengine linked).
"""
import argparse
import os
import subprocess
import sys
import tempfile
import zipfile

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
PROBE = os.path.join(REPO, "target", "release", "examples", "probe.exe")
RCD = os.environ.get("RCD_BIN", os.path.join(REPO, "target", "debug", "rcd.exe"))
CORES = os.environ.get("RC_ORACLE_CORES", r"C:\Users\matti\Workspace\Emulators\RetroArch-Win64\cores")
CORE = os.path.join(CORES, "mednafen_pce_fast_libretro.dll")
SYSTEM = os.path.join(os.path.dirname(CORES), "system")
RUNNER = os.path.join(REPO, "rctools", "scripts", "libretro_run.py")
CACHE = os.path.join(os.environ.get("LOCALAPPDATA", ""), "retroclawd", "cache", "romm")


def listing():
    out = subprocess.run([RCD, "romm", "games", "130"], capture_output=True, text=True).stdout.splitlines()[2:]
    return [(line.split()[0], " ".join(line.split()[1:-2])) for line in out if line.strip()]


def fetch(game_id, work):
    """The ROM bytes of a RomM game, written to `work`; the cache is filled by running rcd once."""
    folder = os.path.join(CACHE, f"rom-{game_id}")
    if not os.path.isdir(folder):
        subprocess.run([RCD, "pcengine", "--rom", f"romm:{game_id}", "--headless", "--frames", "1"], capture_output=True)
    if not os.path.isdir(folder):
        return None
    name = os.listdir(folder)[0]
    path = os.path.join(folder, name)
    data = open(path, "rb").read()
    if data[:2] == b"PK":
        z = zipfile.ZipFile(path)
        members = [m for m in z.namelist() if m.lower().endswith((".pce", ".bin", ".tg16"))] or z.namelist()
        data = z.read(members[0])
    rom = os.path.join(work, f"{game_id}.pce")
    open(rom, "wb").write(data)
    return rom


def grid(img):
    return list(img.convert("L").resize((64, 48), Image.BILINEAR).get_flattened_data())


def mad(a, b):
    return sum(abs(x - y) for x, y in zip(a, b)) / len(a)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--frames", type=int, default=600)
    ap.add_argument("--step", type=int, default=14)
    ap.add_argument("--ids", default="")
    ap.add_argument("--side-by-side", default="", help="folder to write ours|reference pictures into")
    args = ap.parse_args()
    games = listing()
    if args.ids:
        wanted = set(args.ids.split(","))
        games = [g for g in games if g[0] in wanted]
    else:
        games = games[:: args.step]
    work = tempfile.mkdtemp(prefix="pce-sweep-")
    results = []
    for game_id, title in games:
        rom = fetch(game_id, work)
        if not rom:
            print(f"{game_id} {title}: no ROM")
            continue
        ours = os.path.join(work, f"{game_id}-ours.ppm")
        ref = os.path.join(work, f"{game_id}-ref.png")
        env = dict(os.environ, ROM=rom, FRAMES=str(args.frames), OUT=ours)
        r = subprocess.run([PROBE], env=env, capture_output=True, text=True, timeout=600)
        if r.returncode != 0 or not os.path.exists(ours):
            print(f"{game_id} {title}: our run failed: {(r.stderr or r.stdout)[-200:].strip()}")
            results.append((game_id, title, None))
            continue
        subprocess.run([sys.executable, RUNNER, CORE, rom, "--frames", str(args.frames), "--out-frame", ref,
                        "--system-dir", SYSTEM], capture_output=True)
        if not os.path.exists(ref):
            print(f"{game_id} {title}: reference run failed")
            continue
        d = mad(grid(Image.open(ours)), grid(Image.open(ref)))
        results.append((game_id, title, d))
        if args.side_by_side:
            os.makedirs(args.side_by_side, exist_ok=True)
            a = Image.open(ours).convert("RGB")
            b = Image.open(ref).convert("RGB").resize(a.size)
            both = Image.new("RGB", (a.width * 2, a.height))
            both.paste(a, (0, 0))
            both.paste(b, (a.width, 0))
            both.save(os.path.join(args.side_by_side, f"{game_id}.png"))
        print(f"{game_id:>5} {d:6.2f}  {title}")
    ok = [r for r in results if r[2] is not None and r[2] < 8]
    print(f"\n{len(ok)} of {len(results)} within 8.0 mean luminance difference")


if __name__ == "__main__":
    main()
