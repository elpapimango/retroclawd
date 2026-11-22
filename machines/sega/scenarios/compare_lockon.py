#!/usr/bin/env python3
"""Genesis Plus GX (Sonic 3 with its Sonic & Knuckles lock-on) against retroclawd (Sonic & Knuckles with `--lock-on` Sonic 3).

    compare_lockon.py S3.md SK.md --frames 300,600 [--tap FRAME:BUTTON]

Needs software/oracles/system_sk/ (bios_MD.bin, sk.bin = the Sonic & Knuckles ROM, sk2chip.bin = 256 KB, any content for a
game that does not use the upmem chip); both ROMs are plain (deinterleaved) dumps.
"""
import argparse
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import pngdiff  # noqa: E402

src = open(os.path.join(HERE, "compare_frames.py"), encoding="utf-8").read().split("def main")[0]
exec(src)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("s3")
    ap.add_argument("sk")
    ap.add_argument("--frames", default="300")
    ap.add_argument("--tap", action="append", default=[])
    a = ap.parse_args()
    out = os.path.join(ROOT, "software", "out", "lock")
    os.makedirs(out, exist_ok=True)
    taps = [(int(t.split(":")[0]), t.split(":")[1], 6) for t in a.tap]
    events = {}
    for f, b, h in taps:
        events.setdefault(f, set()).add(b)
        events.setdefault(f + h, set())
    script = os.path.join(out, "input.txt")
    held = set()
    with open(script, "w") as fh:
        for f in sorted(events):
            for t in taps:
                if t[0] == f:
                    held.add(t[1])
                if t[0] + t[2] == f:
                    held.discard(t[1])
            fh.write(f"{f} {','.join(RETROPAD[b] for b in sorted(held)) or '-'}\n")
    for n in (int(x) for x in a.frames.split(",")):
        gp = os.path.join(out, f"g-{n}.png")
        ours = os.path.join(out, f"o-{n}.png")
        subprocess.run([sys.executable, "-I", RUNNER, os.path.join(CORES, "genesis_plus_gx_libretro.dll"), a.s3, "--frames", str(n),
                        "--system-dir", os.path.join(ROOT, "software", "oracles", "system_sk"), "--input", script,
                        "--option", "genesis_plus_gx_lock_on=sonic & knuckles", "--out-frame", gp],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        cmd = [HOST, "genesis", "--rom", a.sk, "--lock-on", a.s3, "--headless", "--frames", str(n), "--dump-screen", ours]
        for f, b, h in taps:
            if f <= n:
                cmd += ["--at", f"{f}:key_down=Joy1{b}"]
            if f + h <= n:
                cmd += ["--at", f"{f + h}:key_up=Joy1{b}"]
        env = dict(os.environ, RETROCLAWD_MACHINE_PLUGIN_DIRS=PLUGINS)
        subprocess.run(cmd, check=True, capture_output=True, env=env)
        wa, ha, pa = pngdiff.read_png(gp)
        wb, hb, pb = pngdiff.read_png(ours)
        qa, qb = quantize(pa, GPGX_LEVELS), quantize(pb, OUR_LEVELS)
        diff = sum(1 for y in range(ha) for x in range(wa) if qa[y][x] != qb[y][x])
        print(f"frame {n}: {'identical' if diff == 0 else f'{diff} of {wa * ha} pixels differ'}")


main()
