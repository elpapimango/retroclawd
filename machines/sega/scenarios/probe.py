#!/usr/bin/env python3
"""Build a probe (a 68000 program), run it on the oracle cores and compare what it leaves in work RAM.

    probe.py scenarios/probes/pNN_name.s [--frames 30] [--ram START:LEN] [--region usa|europe|japan]
             [--cores gpgx,picodrive,blastem,clownmdemu] [--z80 FILE] [--save GOLDEN_DIR]

The probe's results are bytes at RAM offset START.. (default 0:32), read in the 68000's byte order. Prints one line per core and
says whether they agree. --save writes the first agreeing core's dump to GOLDEN_DIR/<name>.bin (goldens are never committed).
"""
import argparse
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
CORES_DIR = os.environ.get("RC_ORACLE_CORES", r"C:\Users\matti\Workspace\Emulators\RetroArch-Win64\cores")
RUNNER = os.environ.get("RC_LIBRETRO_RUN", os.path.join(ROOT, "..", "..", "..", "retroclawd-tools", "scripts", "libretro_run.py"))
RCASM = os.environ.get("RCASM", os.path.join(ROOT, "..", "..", "..", "retroclawd-tools", "target", "release", "rcasm.exe"))
SYSTEM = os.path.join(ROOT, "software", "oracles", "system")
OUT = os.path.join(ROOT, "software", "out", "probes")

CORES = {
    "gpgx": "genesis_plus_gx",
    "picodrive": "picodrive",
    "blastem": "blastem",
    "clownmdemu": "clownmdemu",
}
REGION_OPTIONS = {
    "gpgx": {"usa": ["genesis_plus_gx_region_detect=ntsc-u"], "europe": ["genesis_plus_gx_region_detect=pal"],
             "japan": ["genesis_plus_gx_region_detect=ntsc-j"]},
    "picodrive": {"usa": ["picodrive_region=US"], "europe": ["picodrive_region=Europe"],
                  "japan": ["picodrive_region=Japan NTSC"]},
    "blastem": {"usa": ["blastem_region=U", "blastem_force_region=on"], "europe": ["blastem_region=E", "blastem_force_region=on"],
                "japan": ["blastem_region=J", "blastem_force_region=on"]},
    "clownmdemu": {"usa": ["clownmdemu_overseas_region=elsewhere"], "europe": ["clownmdemu_overseas_region=elsewhere"],
                   "japan": ["clownmdemu_overseas_region=japan"]},
}


def build(source, header_region="JUE", size=0):
    name = os.path.splitext(os.path.basename(source))[0]
    os.makedirs(OUT, exist_ok=True)
    binary = os.path.join(OUT, name + ".bin")
    rom = os.path.join(OUT, name + ".md")
    companion = os.path.splitext(source)[0] + ".z80.s"
    if os.path.exists(companion):
        # a Z80 program the probe copies into Z80 RAM: `.incbin "NAME.z80.bin"`
        subprocess.run([RCASM, "-c", "z80", "-f", "bin", "--force", "-o", os.path.join(OUT, name + ".z80.bin"), companion],
                       check=True, stdout=subprocess.DEVNULL)
        subprocess.run([sys.executable, "-I", os.path.join(HERE, "bin2dc.py"), os.path.join(OUT, name + ".z80.bin"),
                        os.path.join(OUT, name + ".z80.inc")], check=True)
    subprocess.run([RCASM, "-c", "68000", "-f", "bin", "--force", "-I", OUT, "-o", binary, source], check=True,
                   stdout=subprocess.DEVNULL)
    subprocess.run([sys.executable, "-I", os.path.join(HERE, "mkrom.py"), binary, rom, "--name", name,
                    "--region", header_region]
                   + (["--size", str(size), "--pattern"] if size else []), check=True)
    return name, rom


def run_core(key, rom, frames, region, extra):
    out = os.path.join(OUT, f"{os.path.basename(rom)}.{key}.ram")
    cmd = [sys.executable, "-I", RUNNER, os.path.join(CORES_DIR, CORES[key] + "_libretro.dll"), rom, "--frames", str(frames),
           "--system-dir", SYSTEM, "--swap16", "--out-ram", out]
    for opt in (REGION_OPTIONS[key][region] if region else []) + extra:
        cmd += ["--option", opt]
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return open(out, "rb").read()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("source")
    ap.add_argument("--frames", type=int, default=30)
    ap.add_argument("--ram", default="0:32")
    ap.add_argument("--region", choices=["usa", "europe", "japan"])
    ap.add_argument("--cores", default=",".join(CORES))
    ap.add_argument("--option", action="append", default=[])
    ap.add_argument("--header-region", default="JUE", help="the region field of the probe ROM's header")
    ap.add_argument("--rom-size", type=lambda v: int(v, 0), default=0, help="pad the probe ROM to this size with a block pattern")
    ap.add_argument("--save")
    a = ap.parse_args()
    start, length = (int(x, 0) for x in a.ram.split(":"))
    name, rom = build(a.source, a.header_region, a.rom_size)
    results = {}
    for key in a.cores.split(","):
        try:
            data = run_core(key, rom, a.frames, a.region, a.option)[start:start + length]
        except subprocess.CalledProcessError:
            print(f"{key:11s} FAILED to run")
            continue
        results[key] = data
        print(f"{key:11s} {data.hex()}")
    values = set(results.values())
    print("AGREE" if len(values) == 1 and results else "DIFFER")
    if a.save and results:
        os.makedirs(a.save, exist_ok=True)
        open(os.path.join(a.save, name + ".bin"), "wb").write(next(iter(results.values())))
    return 0 if len(values) == 1 else 1


if __name__ == "__main__":
    sys.exit(main())
