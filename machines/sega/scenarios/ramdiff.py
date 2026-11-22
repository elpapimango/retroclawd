#!/usr/bin/env python3
"""Compare a work-RAM dump of the oracle (libretro_run.py --swap16) with retroclawd's --dump-memory text output.

    ramdiff.py ORACLE.ram OURS.txt [--max 40]

OURS.txt is the stdout of `retroclawd genesis ... --dump-memory 0xFF0000:65536`. Prints the differing runs (address, both bytes).
"""
import argparse
import re


def read_ours(path):
    data = bytearray(0x10000)
    for line in open(path, encoding="utf-8", errors="replace"):
        m = re.match(r"\s*0x([0-9A-Fa-f]+): ((?:[0-9A-Fa-f]{2} ?)+)\s*$", line)
        if m:
            base = int(m.group(1), 16) - 0xFF0000
            for i, b in enumerate(m.group(2).split()):
                if 0 <= base + i < len(data):
                    data[base + i] = int(b, 16)
    return data


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("oracle")
    ap.add_argument("ours")
    ap.add_argument("--max", type=int, default=40)
    a = ap.parse_args()
    o = open(a.oracle, "rb").read()
    u = read_ours(a.ours)
    runs = []
    i = 0
    while i < len(u):
        if o[i] != u[i]:
            j = i
            while j < len(u) and o[j] != u[j]:
                j += 1
            runs.append((i, j))
            i = j
        else:
            i += 1
    print(f"{sum(j - i for i, j in runs)} bytes differ in {len(runs)} runs")
    for i, j in runs[: a.max]:
        n = min(j - i, 16)
        print(f"  {0xFF0000 + i:06X}+{j - i}: oracle {o[i:i + n].hex(' ')}  ours {u[i:i + n].hex(' ')}")


if __name__ == "__main__":
    main()
