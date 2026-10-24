#!/usr/bin/env python3
"""Tabulate a probe's byte results for each oracle and for retroclawd: hvtable.py PROBE_NAME OURS.txt [--step 4] [--count 120]
OURS.txt is the stdout of `retroclawd genesis --rom software/out/probes/NAME.md --headless --frames N --dump-memory 0xFF0000:LEN`."""
import argparse
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(os.path.dirname(HERE), "software", "out", "probes")


def ours(path):
    data = bytearray()
    for line in open(path, encoding="utf-8", errors="replace"):
        m = re.match(r"\s*0x[0-9A-Fa-f]+: ((?:[0-9A-Fa-f]{2} ?)+)\s*$", line)
        if m:
            data += bytes(int(x, 16) for x in m.group(1).split())
    return bytes(data)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("name")
    ap.add_argument("ours")
    ap.add_argument("--step", type=int, default=4)
    ap.add_argument("--count", type=int, default=120)
    a = ap.parse_args()
    d = {k: open(os.path.join(OUT, f"{a.name}.md.{k}.ram"), "rb").read()[: a.count]
         for k in ("gpgx", "picodrive", "blastem", "clownmdemu")}
    d["ours"] = ours(a.ours)[: a.count]
    print("k   " + " ".join(f"{k[:5]:>5}" for k in d))
    for i in range(0, a.count, a.step):
        print(f"{i:3d} " + " ".join(f"{(v[i] if i < len(v) else -1):5x}" for v in d.values()))


main()
