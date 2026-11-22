#!/usr/bin/env python3
"""Genesis Plus GX against retroclawd for the mouse (probe p46) and the Menacer (probe p47): compare_devices.py

The motion is injected the way each side takes it (libretro_run.py --mouse/--gun, `--at mouse_move=` here); the work RAM the probes
fill is compared. Needs the probe ROMs of scenarios/build.sh and a release build."""
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
SYSTEM = os.path.join(ROOT, "software", "oracles", "system")
ENV = dict(os.environ, RETROCLAWD_MACHINE_PLUGIN_DIRS=os.path.join(ROOT, "target", "release"))
TMP = os.path.join(ROOT, "software", "out", "devices.ram")


def oracle(rom, frames, length, extra):
    subprocess.run([sys.executable, "-I", RUNNER, CORE, rom, "--frames", str(frames), "--system-dir", SYSTEM, "--swap16",
                    "--out-ram", TMP] + extra, check=True, capture_output=True)
    return open(TMP, "rb").read()[:length]


def ours(rom, frames, length, extra):
    r = subprocess.run([HOST, "genesis", "--rom", rom, "--headless", "--frames", str(frames), "--dump-memory",
                        f"0xFF0000:{length}"] + extra, capture_output=True, text=True, env=ENV)
    data = bytearray()
    for line in r.stdout.splitlines():
        m = re.match(r"\s*0x[0-9A-Fa-f]+: ((?:[0-9A-Fa-f]{2} ?)+)\s*$", line)
        if m:
            data += bytes(int(x, 16) for x in m.group(1).split())
    return bytes(data[:length])


def main():
    rom = os.path.join(ROOT, "software", "scenarios", "p46_mouse.md")
    # our frame numbers are one less than the oracle's (it counts the frame the program starts in)
    g = oracle(rom, 45, 19, ["--device", "0=2", "--mouse", "11:5,-3:l", "--mouse", "31:-20,40:r"])
    o = ours(rom, 45, 19, ["--port1", "mouse", "--at", "9:mouse_move=5,-3", "--at", "9:mouse_down=left", "--at", "10:mouse_up=left",
                          "--at", "29:mouse_move=-20,40", "--at", "29:mouse_down=right", "--at", "30:mouse_up=right"])
    print("mouse  ", "identical" if g == o else f"DIFFERENT gpgx {g.hex(' ')} ours {o.hex(' ')}")
    rom = os.path.join(ROOT, "software", "scenarios", "p47_menacer.md")
    for x, y in ((100, 120), (10, 5), (300, 200), (160, 112)):
        g = oracle(rom, 20, 6, ["--device", "1=516", "--gun", f"{x},{y},320,224"])
        o = ours(rom, 20, 6, ["--port2", "menacer", "--at", f"0:mouse_move={x - 160},{y - 112}"])
        print(f"menacer {x},{y}", "identical" if g == o else f"DIFFERENT gpgx {g.hex(' ')} ours {o.hex(' ')}")


main()
