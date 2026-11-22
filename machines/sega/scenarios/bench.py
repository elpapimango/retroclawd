#!/usr/bin/env python3
"""Speed of the Mega Drive plugin: the fastest of N headless runs of a few games, as frames per second and times real time.

    bench.py [--frames 1000] [--runs 3] [ID ...]      (game ids as in software/roms/, default: three)
"""
import argparse
import os
import re
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(ROOT))
HOST = os.path.join(REPO, "target", "release", "retroclawd.exe")
ENV = dict(os.environ, RETROCLAWD_MACHINE_PLUGIN_DIRS=os.path.join(ROOT, "target", "release"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("ids", nargs="*", default=["6759", "6764", "6375"])
    ap.add_argument("--frames", type=int, default=1000)
    ap.add_argument("--runs", type=int, default=3)
    a = ap.parse_args()
    for gid in a.ids:
        d = os.path.join(ROOT, "software", "roms", gid)
        rom = os.path.join(d, [f for f in os.listdir(d) if f.startswith("game.")][0])
        name = open(os.path.join(d, "name.txt"), encoding="utf-8").read().strip() if os.path.exists(os.path.join(d, "name.txt")) else gid
        best = None
        for _ in range(a.runs):
            r = subprocess.run([HOST, "genesis", "--rom", rom, "--headless", "--frames", str(a.frames)], capture_output=True,
                               text=True, env=ENV)
            m = re.search(r"([0-9.]+)s elapsed", r.stdout + r.stderr)
            if m:
                t = float(m.group(1))
                best = t if best is None else min(best, t)
        if best:
            fps = a.frames / best
            print(f"{name[:40]:40s} {fps:6.0f} frames/s  {fps / 59.92:5.1f}x real time")


main()
