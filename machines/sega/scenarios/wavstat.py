#!/usr/bin/env python3
"""Level and pitch of a stereo WAV: wavstat.py FILE [--from S] [--to S]  (RMS of the left channel, peak, zero-crossing frequency)."""
import math
import struct
import sys
import wave


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    start = float(sys.argv[sys.argv.index("--from") + 1]) if "--from" in sys.argv else 0.5
    end = float(sys.argv[sys.argv.index("--to") + 1]) if "--to" in sys.argv else None
    for path in args:
        if path.replace(".", "", 1).isdigit():
            continue
        w = wave.open(path, "rb")
        n, ch, rate = w.getnframes(), w.getnchannels(), w.getframerate()
        data = struct.unpack("<%dh" % (n * ch), w.readframes(n))
        left = data[0::ch]
        a = int(start * rate)
        b = int((end or n / rate) * rate)
        seg = left[a:b]
        mean = sum(seg) / len(seg)
        rms = math.sqrt(sum((x - mean) ** 2 for x in seg) / len(seg))
        peak = max(abs(x - mean) for x in seg)
        crossings = sum(1 for x, y in zip(seg, seg[1:]) if (x - mean) * (y - mean) < 0)
        print(f"{path.split('/')[-1]:32s} rate {rate} rms {rms:8.1f} peak {peak:8.1f} freq {crossings / 2 / (len(seg) / rate):8.1f} Hz")


main()
