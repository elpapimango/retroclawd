#!/usr/bin/env python3
"""Writes an audio CD image (a cue sheet and a BIN) of sine tones, for the Sega CD audio scenario.

    make_tone_disc.py OUT_DIR [--hz 440] [--right-hz 660] [--seconds 20] [--tracks 2] [--amplitude 12000]

Each track plays `--seconds` of a tone, `--hz` on the left channel and `--right-hz` on the right, 16-bit little-endian samples at
44.1 kHz; track n has INDEX 01 right after track n-1. Standard library only (the sine comes from `math`)."""
import argparse
import math
import struct
from pathlib import Path

SAMPLES_PER_SECTOR = 588  # 2352 bytes / 4 bytes per stereo frame


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("out_dir")
    ap.add_argument("--hz", type=float, default=440.0)
    ap.add_argument("--right-hz", type=float, default=660.0)
    ap.add_argument("--tracks", type=int, default=2)
    ap.add_argument("--seconds", type=int, default=20)
    ap.add_argument("--amplitude", type=int, default=12000)
    args = ap.parse_args()
    sectors = args.seconds * 75
    out = Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    data = bytearray()
    for n in range(sectors * SAMPLES_PER_SECTOR * args.tracks):
        left = int(args.amplitude * math.sin(2 * math.pi * args.hz * n / 44100))
        right = int(args.amplitude * math.sin(2 * math.pi * args.right_hz * n / 44100))
        data += struct.pack("<hh", left, right)
    (out / "segacd-tone.bin").write_bytes(data)
    lines = ['FILE "segacd-tone.bin" BINARY']
    for t in range(args.tracks):
        s = t * args.seconds
        lines += [f"  TRACK {t + 1:02d} AUDIO", f"    INDEX 01 {s // 60:02d}:{s % 60:02d}:00"]
    cue = "\n".join(lines) + "\n"
    (out / "segacd-tone.cue").write_text(cue)
    print(f"{out / 'segacd-tone.cue'}: {args.tracks} tracks of {sectors} sectors, {args.hz} Hz left, {args.right_hz} Hz right")


main()
