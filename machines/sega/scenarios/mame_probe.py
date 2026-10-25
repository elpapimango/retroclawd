#!/usr/bin/env python3
"""Run a probe ROM in MAME (`genesis` driver) and print the first bytes of work RAM after N frames: mame_probe.py ROM.md FRAMES LEN"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
MAME = os.environ.get("RC_MAME", r"C:\Users\matti\Workspace\Emulators\Mame")
LUA = os.path.join(HERE, "mame_dump.lua")


def run(rom, frames, length, driver="genesis"):
    out = os.path.join(ROOT, "software", "out", "mame")
    os.makedirs(out, exist_ok=True)
    env = dict(os.environ, MAME_FRAMES=str(frames), MAME_OUT=out)
    subprocess.run([os.path.join(MAME, "mame.exe"), driver, "-cart", os.path.abspath(rom), "-video", "none", "-sound", "none",
                    "-nothrottle", "-seconds_to_run", "120", "-autoboot_script", LUA], cwd=MAME, env=env,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    return open(os.path.join(out, f"m{frames}.ram"), "rb").read()[:length]


if __name__ == "__main__":
    print(run(sys.argv[1], int(sys.argv[2]), int(sys.argv[3])).hex())
