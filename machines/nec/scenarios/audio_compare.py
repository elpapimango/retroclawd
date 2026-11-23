#!/usr/bin/env python3
"""Compare two WAV files by what an ear hears: RMS and the strongest tones (not sample for sample).

    audio_compare.py ours.wav ref.wav [--from-s 3] [--to-s 10]
"""
import argparse
import cmath
import math
import struct
import wave


def load(path):
    w = wave.open(path)
    ch, rate, n = w.getnchannels(), w.getframerate(), w.getnframes()
    raw = w.readframes(n)
    vals = struct.unpack("<" + "h" * (len(raw) // 2), raw)
    mono = [sum(vals[i:i + ch]) / (ch * 32768.0) for i in range(0, len(vals), ch)]
    return mono, rate


def rms(x):
    return math.sqrt(sum(v * v for v in x) / max(len(x), 1))


def tones(x, rate, count=3):
    """The strongest `count` frequencies of the window by a plain DFT over a decimated copy."""
    n = 4096
    seg = x[len(x) // 2 - n // 2: len(x) // 2 + n // 2]
    if len(seg) < n:
        return []
    win = [seg[i] * (0.5 - 0.5 * math.cos(2 * math.pi * i / n)) for i in range(n)]
    mags = []
    for k in range(1, n // 4):
        s = sum(win[i] * cmath.exp(-2j * math.pi * k * i / n) for i in range(0, n))
        mags.append((abs(s), k * rate / n))
    mags.sort(reverse=True)
    out = []
    for m, f in mags:
        if all(abs(f - g) > 60 for g in out):
            out.append(f)
        if len(out) == count:
            break
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("--from-s", type=float, default=0)
    ap.add_argument("--to-s", type=float, default=1e9)
    args = ap.parse_args()
    for name, path in (("ours", args.a), ("ref ", args.b)):
        x, rate = load(path)
        seg = x[int(args.from_s * rate): int(min(args.to_s, len(x) / rate) * rate)]
        print(f"{name}: {len(x) / rate:6.2f} s at {rate} Hz, rms {rms(seg):.4f}, tones {[round(f) for f in tones(seg, rate)]}")


if __name__ == "__main__":
    main()
