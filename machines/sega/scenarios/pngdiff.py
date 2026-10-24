#!/usr/bin/env python3
"""Compare two 8-bit RGB PNG files (what libretro_run.py and retroclawd --dump-screen write): pngdiff.py A.png B.png [--out D.png]

Prints the sizes, the number of pixels that differ, the mean absolute difference per channel and the bounding box of the
differences. Exit status 0 when the pictures are identical. With --out, writes a picture of the differences (white = differs).
"""
import struct
import sys
import zlib


def read_png(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG: " + path
    pos, width, height, depth, ctype, idat = 8, 0, 0, 0, 0, b""
    palette = None
    while pos < len(data):
        n, tag = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        pos += 12 + n
        if tag == b"IHDR":
            width, height, depth, ctype = struct.unpack(">IIBB", body[:10])
        elif tag == b"PLTE":
            palette = body
        elif tag == b"IDAT":
            idat += body
    raw = zlib.decompress(idat)
    channels = {2: 3, 6: 4, 0: 1, 3: 1}[ctype]
    assert depth == 8, "only 8-bit PNGs"
    stride = width * channels
    out, prev = [], bytearray(stride)
    i = 0
    for _ in range(height):
        f = raw[i]
        row = bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        for x in range(stride):
            a = row[x - channels] if x >= channels else 0
            b = prev[x]
            c = prev[x - channels] if x >= channels else 0
            if f == 1:
                row[x] = (row[x] + a) & 255
            elif f == 2:
                row[x] = (row[x] + b) & 255
            elif f == 3:
                row[x] = (row[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                row[x] = (row[x] + pr) & 255
        out.append(bytes(row))
        prev = row
    pixels = []
    for row in out:
        line = []
        for x in range(width):
            if ctype == 3:
                k = row[x] * 3
                line.append(tuple(palette[k:k + 3]))
            elif ctype == 0:
                line.append((row[x],) * 3)
            else:
                line.append(tuple(row[x * channels:x * channels + 3]))
        pixels.append(line)
    return width, height, pixels


def write_png(path, width, height, rows):
    def chunk(tag, body):
        c = tag + body
        return struct.pack(">I", len(body)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + bytes(v for px in row for v in px) for row in rows)
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    out = sys.argv[sys.argv.index("--out") + 1] if "--out" in sys.argv else None
    if out:
        args.remove(out)
    wa, ha, pa = read_png(args[0])
    wb, hb, pb = read_png(args[1])
    print(f"A {wa}x{ha}  B {wb}x{hb}")
    if (wa, ha) != (wb, hb):
        print("sizes differ")
        return 2
    differing, total, box = 0, 0, [wa, ha, -1, -1]
    diff_rows = []
    for y in range(ha):
        row = []
        for x in range(wa):
            d = sum(abs(a - b) for a, b in zip(pa[y][x], pb[y][x]))
            total += d
            if d:
                differing += 1
                box = [min(box[0], x), min(box[1], y), max(box[2], x), max(box[3], y)]
            row.append((255, 255, 255) if d else (0, 0, 0))
        diff_rows.append(row)
    print(f"differing pixels {differing} of {wa * ha}; mean abs difference {total / (wa * ha * 3):.3f}; box {box}")
    if out:
        write_png(out, wa, ha, diff_rows)
    return 0 if differing == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
