#!/usr/bin/env python3
"""Fit the VDP slot tables to Genesis Plus GX's p33 results by coordinate search.

    fitslots.py h40|h32 "slot,slot,..."

Needs a build of the plugin whose fifo.rs reads RC_SLOTS_H40 / RC_SLOTS_H32 (a tuning hook, not kept in the tree), and the
probe ROMs of scenarios/build.sh. The cost is the sum of the differences (master clocks, capped at 400) between the times
the n-th data-port write was accepted in Genesis Plus GX and in retroclawd, n = 5 .. 44, for the VRAM and CRAM probes.
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(ROOT))
HOST = os.path.join(REPO, "target", "release", "retroclawd.exe")
LINE = 3420


def clock_of(h, h40):
    off = 780 if h40 else 774
    first = 0xE4 if h40 else 0xE9
    base = 18 if h40 else 15
    steps = 46 if h40 else 38
    if h >= first:
        return (h - first + base) * off // steps - off
    lo = 0xA5 if h40 else 0x85
    hi = 0xB6 if h40 else 0x93
    if lo <= h <= hi:
        return (h - lo) * off // steps - off
    return h * (16 if h40 else 20)


def times(data, h40):
    out = []
    for n in range(44):
        w0 = int.from_bytes(data[4 * n:4 * n + 2], "big")
        w1 = int.from_bytes(data[4 * n + 2:4 * n + 4], "big")
        dv = ((w1 >> 8) - (w0 >> 8)) & 255
        out.append(dv * LINE + clock_of(w1 & 255, h40) - clock_of(w0 & 255, h40))
    return out


def ours(rom, env):
    e = dict(os.environ, RETROCLAWD_MACHINE_PLUGIN_DIRS=os.path.join(ROOT, "target", "release"), **env)
    r = subprocess.run([HOST, "genesis", "--rom", rom, "--headless", "--frames", "5", "--dump-memory", "0xFF0000:176"],
                       capture_output=True, text=True, env=e)
    d = bytearray()
    for line in r.stdout.splitlines():
        m = re.match(r"\s*0x[0-9A-Fa-f]+: ((?:[0-9A-Fa-f]{2} ?)+)\s*$", line)
        if m:
            d += bytes(int(x, 16) for x in m.group(1).split())
    return bytes(d[:176])


def gold(name):
    return open(os.path.join(ROOT, "software", "out", "probes", name + ".md.gpgx.ram"), "rb").read()[:176]


STATUS = {"h40": ["p31a_fifo_vram_h40", "p31b_fifo_cram_h40", "p31e_fifo_vram_h40_d40", "p31f_fifo_cram_h40_d40"],
          "h32": ["p31c_fifo_vram_h32", "p31d_fifo_cram_h32_d0", "p31g_fifo_vram_h32_d40", "p31h_fifo_cram_h32_d40"]}


def first_empty(data):
    return next((i for i, b in enumerate(data[:88]) if b == 2), 88)


def status_ours(rom, env):
    e = dict(os.environ, RETROCLAWD_MACHINE_PLUGIN_DIRS=os.path.join(ROOT, "target", "release"), **env)
    r = subprocess.run([HOST, "genesis", "--rom", rom, "--headless", "--frames", "5", "--dump-memory", "0xFF0000:88"],
                       capture_output=True, text=True, env=e)
    d = bytearray()
    for line in r.stdout.splitlines():
        m = re.match(r"\s*0x[0-9A-Fa-f]+: ((?:[0-9A-Fa-f]{2} ?)+)\s*$", line)
        if m:
            d += bytes(int(x, 16) for x in m.group(1).split())
    return bytes(d[:88])


CASES = {"h40": [("p33a_slots_vram_h40", True), ("p33b_slots_cram_h40", True)],
         "h32": [("p33c_slots_vram_h32", False), ("p33d_slots_cram_h32", False)]}


def cost(mode, slots):
    env = {"RC_SLOTS_H40" if mode == "h40" else "RC_SLOTS_H32": ",".join(map(str, slots))}
    c = 0
    for name, h40 in CASES[mode]:
        g = times(gold(name), h40)
        o = times(ours(os.path.join(ROOT, "software", "scenarios", name + ".md"), env), h40)
        c += sum(min(abs(a - b), 400) for a, b in zip(g[4:], o[4:]))
    for name in STATUS[mode]:
        g = first_empty(open(os.path.join(ROOT, "software", "out", "probes", name + ".md.gpgx.ram"), "rb").read())
        o = first_empty(status_ours(os.path.join(ROOT, "software", "scenarios", name + ".md"), env))
        c += min(abs(g - o), 30) * 28 * 2
    return c


def main():
    mode = sys.argv[1]
    slots = [int(x) for x in sys.argv[2].split(",")]
    best = cost(mode, slots)
    print("start", best, flush=True)
    for step in (64, 32, 16, 8, 4):
        improved = True
        while improved:
            improved = False
            for i in range(len(slots)):
                for d in (-step, step):
                    t = slots[:]
                    t[i] = (t[i] + d) % LINE
                    t = sorted(t)
                    c = cost(mode, t)
                    if c < best:
                        best = c
                        slots = t
                        improved = True
                        print(best, ",".join(map(str, slots)), flush=True)
                        break
    print("final", best, ",".join(map(str, slots)))


main()
