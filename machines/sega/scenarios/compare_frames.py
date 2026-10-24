#!/usr/bin/env python3
"""Run a ROM in Genesis Plus GX (the main oracle) and in retroclawd for the same frame counts and compare the pictures.

    compare_frames.py ROM --frames 300,600,900 [--tap FRAME:BUTTON[:HOLD] ...] [--region usa|europe|japan] [--keep DIR]

BUTTON is a Genesis pad button (Up Down Left Right A B C X Y Z Start Mode); --tap presses it at FRAME for HOLD frames (default 6).
The pictures are compared after both are reduced to the 8 colour levels each emulator can show (Genesis Plus GX outputs
RGB565, retroclawd 8-bit levels), so what differs is what is on the screen, not the digital-to-analogue conversion.
Prints one line per frame and the differing pixel count; exit status 1 if any picture differs.
"""
import argparse
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import pngdiff  # noqa: E402

REPO = os.path.dirname(os.path.dirname(ROOT))
CORES = os.environ.get("RC_ORACLE_CORES", r"C:\Users\matti\Workspace\Emulators\RetroArch-Win64\cores")
RUNNER = os.path.join(REPO, "..", "retroclawd-tools", "scripts", "libretro_run.py")
HOST = os.environ.get("RETROCLAWD_BIN", os.path.join(REPO, "target", "release", "retroclawd.exe"))
PLUGINS = os.environ.get("RETROCLAWD_MACHINE_PLUGIN_DIRS", os.path.join(ROOT, "target", "release"))
SYSTEM = os.path.join(ROOT, "software", "oracles", "system")

RETROPAD = {"Up": "up", "Down": "down", "Left": "left", "Right": "right", "Start": "start", "A": "y", "B": "b", "C": "a",
            "X": "l", "Y": "x", "Z": "r", "Mode": "select"}
GPGX_LEVELS = [0, 33, 66, 99, 140, 173, 206, 239]
OUR_LEVELS = [0, 34, 68, 102, 136, 170, 204, 238]
GPGX_REGION = {"usa": "ntsc-u", "europe": "pal", "japan": "ntsc-j"}


def quantize(pixels, levels):
    table = {}
    out = []
    for row in pixels:
        line = []
        for px in row:
            q = table.get(px)
            if q is None:
                q = table[px] = tuple(min(range(8), key=lambda i: abs(levels[i] - c)) for c in px)
            line.append(q)
        out.append(line)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("rom")
    ap.add_argument("--frames", default="300")
    ap.add_argument("--tap", action="append", default=[])
    ap.add_argument("--region")
    ap.add_argument("--gpgx-option", action="append", default=[], help="a core option for the oracle, key=value")
    ap.add_argument("--ours", action="append", default=[], help="extra arguments for retroclawd (give one per word)")
    ap.add_argument("--keep", default=os.path.join(ROOT, "software", "out", "compare"))
    a = ap.parse_args()
    os.makedirs(a.keep, exist_ok=True)
    taps = []
    for t in a.tap:
        parts = t.split(":")
        taps.append((int(parts[0]), parts[1], int(parts[2]) if len(parts) > 2 else 6))
    # the oracle's input script: one line per change, buttons held until the next line
    events = {}
    for frame, button, hold in taps:
        events.setdefault(frame, set()).add(button)
        events.setdefault(frame + hold, set())
    script = os.path.join(a.keep, "input.txt")
    held = set()
    with open(script, "w") as f:
        for frame in sorted(events):
            for t in taps:
                if t[0] == frame:
                    held.add(t[1])
                if t[0] + t[2] == frame:
                    held.discard(t[1])
            f.write(f"{frame} {','.join(RETROPAD[b] for b in sorted(held)) or '-'}\n")
    bad = 0
    for n in (int(x) for x in a.frames.split(",")):
        gp = os.path.join(a.keep, f"gpgx-{n}.png")
        ours = os.path.join(a.keep, f"ours-{n}.png")
        cmd = [sys.executable, "-I", RUNNER, os.path.join(CORES, "genesis_plus_gx_libretro.dll"), a.rom, "--frames", str(n),
               "--system-dir", SYSTEM, "--input", script, "--out-frame", gp]
        if a.region:
            cmd += ["--option", "genesis_plus_gx_region_detect=" + GPGX_REGION[a.region]]
        for opt in a.gpgx_option:
            cmd += ["--option", opt]
        subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        cmd = [HOST, "genesis", "--rom", a.rom, "--headless", "--frames", str(n), "--dump-screen", ours]
        if a.region:
            cmd += ["--console-region", a.region]
        cmd += a.ours
        for frame, button, hold in taps:
            if frame <= n:
                cmd += ["--at", f"{frame}:key_down=Joy1{button}"]
            if frame + hold <= n:
                cmd += ["--at", f"{frame + hold}:key_up=Joy1{button}"]
        env = dict(os.environ, RETROCLAWD_MACHINE_PLUGIN_DIRS=PLUGINS)
        r = subprocess.run(cmd, capture_output=True, text=True, env=env)
        if r.returncode != 0 or "stop:" in r.stderr:
            print(f"frame {n}: retroclawd failed: {(r.stderr or r.stdout).strip()[:300]}")
            bad += 1
            continue
        wa, ha, pa = pngdiff.read_png(gp)
        wb, hb, pb = pngdiff.read_png(ours)
        if (wa, ha) != (wb, hb):
            print(f"frame {n}: sizes differ: oracle {wa}x{ha}, ours {wb}x{hb}")
            bad += 1
            continue
        qa, qb = quantize(pa, GPGX_LEVELS), quantize(pb, OUR_LEVELS)
        diff = sum(1 for y in range(ha) for x in range(wa) if qa[y][x] != qb[y][x])
        print(f"frame {n}: {'identical' if diff == 0 else f'{diff} of {wa * ha} pixels differ'}")
        bad += diff != 0
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
