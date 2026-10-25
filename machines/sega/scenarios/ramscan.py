#!/usr/bin/env python3
"""Compare work RAM of Genesis Plus GX and retroclawd at several frames of a ROM: ramscan.py ROM 86,88,90 [--max 6]"""
import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(ROOT))
CORE = os.path.join(os.environ.get("RC_ORACLE_CORES", r"C:\Users\matti\Workspace\Emulators\RetroArch-Win64\cores"),
                    "genesis_plus_gx_libretro.dll")
RUNNER = os.path.join(REPO, "..", "retroclawd-tools", "scripts", "libretro_run.py")
HOST = os.path.join(REPO, "target", "release", "retroclawd.exe")
OUT = os.path.join(ROOT, "software", "out", "ramscan")


def ours(rom, n):
    env = dict(os.environ, RETROCLAWD_MACHINE_PLUGIN_DIRS=os.path.join(ROOT, "target", "release"))
    r = subprocess.run([HOST, "genesis", "--rom", rom, "--headless", "--frames", str(n), "--dump-memory", "0xFF0000:65536"],
                       capture_output=True, text=True, env=env)
    d = bytearray(0x10000)
    for line in r.stdout.splitlines():
        m = re.match(r"\s*0x([0-9A-Fa-f]+): ((?:[0-9A-Fa-f]{2} ?)+)\s*$", line)
        if m:
            base = int(m.group(1), 16) - 0xFF0000
            for i, b in enumerate(m.group(2).split()):
                d[base + i] = int(b, 16)
    return bytes(d)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("rom")
    ap.add_argument("frames")
    ap.add_argument("--max", type=int, default=6)
    a = ap.parse_args()
    os.makedirs(OUT, exist_ok=True)
    frames = [int(x) for x in a.frames.split(",")]
    cmd = [sys.executable, "-I", RUNNER, CORE, a.rom, "--frames", str(max(frames)), "--system-dir",
           os.path.join(ROOT, "software", "oracles", "system"), "--swap16"]
    for f in frames:
        cmd += ["--ram-at", f"{f}:{OUT}/g{f}.ram"]
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for f in frames:
        g = open(f"{OUT}/g{f}.ram", "rb").read()
        o = ours(a.rom, f)
        runs = []
        i = 0
        while i < len(o):
            if g[i] != o[i]:
                j = i
                while j < len(o) and g[j] != o[j]:
                    j += 1
                runs.append((i, j))
                i = j
            else:
                i += 1
        print(f"frame {f}: {sum(j - i for i, j in runs)} bytes in {len(runs)} runs")
        for i, j in runs[: a.max]:
            n = min(j - i, 12)
            print(f"   {0xFF0000 + i:06X}+{j - i}: gpgx {g[i:i + n].hex(' ')}  ours {o[i:i + n].hex(' ')}")


main()
