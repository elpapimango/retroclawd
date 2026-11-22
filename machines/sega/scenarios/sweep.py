#!/usr/bin/env python3
"""Run every game in software/roms/ for some frames and report machine stops, crashes and blank pictures.

    sweep.py [--frames 600] [--jobs 6] [ID ...]

A stop is the machine meeting an address no fact covers (`genesis: stop: ...`). A picture is blank when every pixel is the same.
"""
import argparse
import concurrent.futures
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import pngdiff  # noqa: E402

REPO = os.path.dirname(os.path.dirname(ROOT))
HOST = os.environ.get("RETROCLAWD_BIN", os.path.join(REPO, "target", "release", "retroclawd.exe"))
PLUGINS = os.environ.get("RETROCLAWD_MACHINE_PLUGIN_DIRS", os.path.join(ROOT, "target", "release"))
OUT = os.path.join(ROOT, "software", "out", "sweep")


def games(ids):
    base = os.path.join(ROOT, "software", "roms")
    for gid in sorted(ids or os.listdir(base)):
        d = os.path.join(base, gid)
        files = [f for f in os.listdir(d) if f.startswith("game.")] if os.path.isdir(d) else []
        if files:
            name = open(os.path.join(d, "name.txt"), encoding="utf-8").read().strip() if os.path.exists(os.path.join(d, "name.txt")) else gid
            yield gid, name, os.path.join(d, files[0])


def run(job):
    gid, name, rom, frames = job
    os.makedirs(OUT, exist_ok=True)
    png = os.path.join(OUT, f"{gid}.png")
    env = dict(os.environ, RETROCLAWD_MACHINE_PLUGIN_DIRS=PLUGINS)
    r = subprocess.run([HOST, "genesis", "--rom", rom, "--headless", "--frames", str(frames), "--dump-screen", png],
                       capture_output=True, text=True, env=env, timeout=600)
    notes = []
    stops = [l for l in r.stderr.splitlines() if "stop:" in l]
    if stops:
        notes.append(stops[0].replace("genesis: stop: ", "STOP "))
    if r.returncode != 0 and not stops:
        notes.append(f"exit {r.returncode}: {(r.stderr or r.stdout).strip()[:120]}")
    try:
        w, h, px = pngdiff.read_png(png)
        colours = {p for row in px for p in row}
        if len(colours) == 1:
            notes.append("blank picture")
        else:
            notes.append(f"{w}x{h}, {len(colours)} colours")
    except Exception as e:
        notes.append(f"no picture ({e})")
    return gid, name, notes


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("ids", nargs="*")
    ap.add_argument("--frames", type=int, default=600)
    ap.add_argument("--jobs", type=int, default=6)
    a = ap.parse_args()
    jobs = [(gid, name, rom, a.frames) for gid, name, rom in games(a.ids)]
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        for gid, name, notes in pool.map(run, jobs):
            print(f"{gid:6s} {name[:42]:42s} {'; '.join(notes)}")


if __name__ == "__main__":
    main()
