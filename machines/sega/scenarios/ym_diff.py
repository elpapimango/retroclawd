#!/usr/bin/env python3
"""Compare the Rust YM2612 with the reference harness (software/tools/nuked/oracle.exe), cycle by cycle.

    ym_diff.py [--chip 2612|3438] [--cases basic,timer,random] [--seed N] [--length SAMPLES]

Each case is a script of register writes and chip cycles run through both programs; every cycle's left and right output is
compared. The first difference is reported with the cycle, sample and the writes that preceded it. Needs the reference built
(see docs/ym2612-validation.md) and `cargo build --release -p ym2612 --example ymscript`.
"""
import argparse
import os
import random
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
ORACLE = os.path.join(ROOT, "software", "tools", "nuked", "oracle.exe")
OURS = os.path.join(ROOT, "target", "release", "examples", "ymscript.exe")
OUT = os.path.join(ROOT, "software", "out", "ym")


class Script:
    def __init__(self):
        self.lines = []
        self.log = []

    def write(self, bank, reg, value, gap=24):
        self.lines += [f"W {bank * 2} {reg}", "T 8", f"W {bank * 2 + 1} {value}", f"T {gap}"]
        self.log.append((len(self.lines), f"bank {bank} reg {reg:02X} = {value:02X}"))

    def run(self, cycles):
        self.lines.append(f"T {cycles}")

    def text(self):
        return "\n".join(self.lines) + "\n"


def basic():
    s = Script()
    for op in range(4):
        base = op * 4
        s.write(0, 0x30 + base, 0x01 + op)
        s.write(0, 0x40 + base, 0x10 * (op == 0))
        s.write(0, 0x50 + base, 0x1F)
        s.write(0, 0x60 + base, 0x08)
        s.write(0, 0x70 + base, 0x04)
        s.write(0, 0x80 + base, 0x2F)
    s.write(0, 0xB0, 0x3C)
    s.write(0, 0xB4, 0xC0)
    s.write(0, 0xA4, 0x22)
    s.write(0, 0xA0, 0x69)
    s.write(0, 0x28, 0xF0)
    s.run(24 * 600)
    s.write(0, 0x28, 0x00)
    s.run(24 * 400)
    return s


def timers():
    s = Script()
    s.write(0, 0x24, 0xF0)
    s.write(0, 0x25, 0x01)
    s.write(0, 0x26, 0xE0)
    s.write(0, 0x27, 0x0F)
    s.run(24 * 300)
    s.lines.append("R 0")
    s.write(0, 0x27, 0x3F)
    s.lines.append("R 0")
    s.run(24 * 100)
    s.lines.append("R 0")
    return s


def dac():
    s = Script()
    s.write(0, 0x2B, 0x80)
    for v in (0x00, 0x40, 0x7F, 0x80, 0xFF, 0x20):
        s.write(0, 0x2A, v)
        s.run(24 * 3)
    s.write(0, 0xB4, 0x80)
    s.write(0, 0x2A, 0x90)
    s.run(24 * 5)
    return s


def random_case(seed, samples):
    rnd = random.Random(seed)
    s = Script()
    regs = [0x22, 0x27, 0x28, 0x2A, 0x2B] + [b + o for b in (0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90) for o in (0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14)] + [0xA0, 0xA1, 0xA2, 0xA4, 0xA5, 0xA6, 0xA8, 0xA9, 0xAA, 0xAC, 0xAD, 0xAE, 0xB0, 0xB1, 0xB2, 0xB4, 0xB5, 0xB6]
    # a playable start: some operators audible
    for ch in range(3):
        for op in range(4):
            s.write(0, 0x40 + op * 4 + ch, rnd.randrange(0x30))
            s.write(0, 0x50 + op * 4 + ch, rnd.randrange(0x20))
        s.write(0, 0xB0 + ch, rnd.randrange(64))
        s.write(0, 0xB4 + ch, 0xC0 | rnd.randrange(64))
    total = 0
    while total < samples:
        reg = rnd.choice(regs)
        value = rnd.randrange(256)
        if reg == 0x27:
            value &= 0xCF if rnd.random() < 0.7 else 0xFF
        if reg == 0x28:
            value = rnd.choice([0xF0, 0xF1, 0xF2, 0xF4, 0xF5, 0xF6, 0x00, 0x01, 0x02, 0x04, 0x05, 0x06, rnd.randrange(256)])
        if reg == 0x2B:
            value = rnd.choice([0, 0x80])
        bank = rnd.randrange(2) if reg >= 0x30 and reg not in (0x22, 0x27, 0x28, 0x2A, 0x2B) else 0
        s.write(bank, reg, value, gap=rnd.randrange(8, 60))
        n = rnd.randrange(1, 40)
        s.run(24 * n)
        total += n
    return s


def run(script, chip, name):
    os.makedirs(OUT, exist_ok=True)
    sa, sb = os.path.join(OUT, name + ".oracle.raw"), os.path.join(OUT, name + ".ours.raw")
    a = subprocess.run([ORACLE, chip, sa], input=script.text(), capture_output=True, text=True)
    b = subprocess.run([OURS, chip, sb], input=script.text(), capture_output=True, text=True)
    da, db = open(sa, "rb").read(), open(sb, "rb").read()
    if a.stdout != b.stdout:
        print(f"  reads differ: oracle {a.stdout.split()}, ours {b.stdout.split()}")
    if len(da) != len(db):
        print(f"  lengths differ: {len(da)} against {len(db)}")
        return False
    if da == db and a.stdout == b.stdout:
        print(f"  {name}: identical ({len(da) // 4} cycles)")
        return True
    for i in range(0, len(da), 4):
        if da[i:i + 4] != db[i:i + 4]:
            cycle = i // 4
            l1, r1 = int.from_bytes(da[i:i + 2], "little", signed=True), int.from_bytes(da[i + 2:i + 4], "little", signed=True)
            l2, r2 = int.from_bytes(db[i:i + 2], "little", signed=True), int.from_bytes(db[i + 2:i + 4], "little", signed=True)
            # which write was last before the difference
            consumed, last = 0, ""
            t = 0
            for line in script.lines:
                if line.startswith("T "):
                    t += int(line.split()[1])
                    if t > cycle:
                        break
            differing = sum(1 for j in range(0, len(da), 4) if da[j:j + 4] != db[j:j + 4])
            print(f"  {name}: first difference at cycle {cycle} (sample {cycle // 24}, slot {cycle % 24}): oracle {l1},{r1} ours {l2},{r2}; {differing} cycles differ")
            return False
    return False


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--chip", default="3438")
    ap.add_argument("--cases", default="basic,timers,dac,random")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--length", type=int, default=3000)
    ap.add_argument("--seeds", type=int, default=1)
    a = ap.parse_args()
    ok = True
    for case in a.cases.split(","):
        if case == "random":
            for k in range(a.seeds):
                ok &= run(random_case(a.seed + k, a.length), a.chip, f"random-{a.seed + k}-{a.chip}")
        else:
            ok &= run({"basic": basic, "timers": timers, "dac": dac}[case](), a.chip, f"{case}-{a.chip}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
