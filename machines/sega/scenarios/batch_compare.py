#!/usr/bin/env python3
"""compare_frames.py over every game in software/roms/ at several frame counts, a few games at a time.

    batch_compare.py [--frames 100,300,600] [--jobs 5] [ID ...]
"""
import argparse
import concurrent.futures
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)


def games(ids):
    base = os.path.join(ROOT, "software", "roms")
    for gid in sorted(ids or os.listdir(base)):
        d = os.path.join(base, gid)
        files = [f for f in os.listdir(d) if f.startswith("game.")] if os.path.isdir(d) else []
        if files:
            name = open(os.path.join(d, "name.txt"), encoding="utf-8").read().strip() if os.path.exists(os.path.join(d, "name.txt")) else gid
            yield gid, name, os.path.join(d, files[0])


def run(job):
    gid, name, rom, frames, extra = job
    keep = os.path.join(ROOT, "software", "out", "batch", gid)
    r = subprocess.run([sys.executable, os.path.join(HERE, "compare_frames.py"), rom, "--frames", frames, "--keep", keep] + extra,
                       capture_output=True, text=True, timeout=3600)
    cells = []
    for line in r.stdout.splitlines():
        if line.startswith("frame "):
            n, rest = line[6:].split(": ", 1)
            cells.append(f"{n}:" + ("=" if rest == "identical" else rest.split()[0] if "pixels" in rest else rest[:30]))
    return gid, name, cells


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("ids", nargs="*")
    ap.add_argument("--frames", default="100,300,600")
    ap.add_argument("--jobs", type=int, default=5)
    ap.add_argument("--extra", action="append", default=[])
    a = ap.parse_args()
    jobs = [(g, n, r, a.frames, a.extra) for g, n, r in games(a.ids)]
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        for gid, name, cells in pool.map(run, jobs):
            print(f"{gid:6s} {name[:38]:38s} {' '.join(cells)}", flush=True)


if __name__ == "__main__":
    main()
