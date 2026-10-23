#!/usr/bin/env python3
"""mkprobe.py TOOLS_DIR PROBE.s OUT.st: assembles a timing probe and puts it on a boot floppy.

The floppy is a blank 720K disk (rcdisk create) whose boot sector is the shared stub (stub.s, assembled by rcasm and
made executable by rcdisk boot); the probe body, assembled to run at $90000, is spread over side 0 of cylinders 1-7 (the stub
reads them with Floprd). Standard library only."""
import os
import subprocess
import sys
import tempfile

SECTOR = 512
TRACK = 9 * SECTOR      # one side-0 track of 9 sectors
TRACKS = 7              # cylinders 1-7


def main() -> int:
    tools, src, out = sys.argv[1:4]
    here = os.path.dirname(os.path.abspath(__file__))
    exe = ".exe" if os.name == "nt" else ""
    with tempfile.TemporaryDirectory() as tmp:
        stub = os.path.join(tmp, "stub.bin")
        body = os.path.join(tmp, "body.bin")
        rcasm = [os.path.join(tools, "rcasm" + exe), "-c", "68000", "-f", "bin"]
        subprocess.run(rcasm + [src, "-o", body], check=True)
        size = os.path.getsize(body)
        tracks = max(1, -(-size // TRACK))
        stub_src = os.path.join(tmp, "stub.s")
        text = open(os.path.join(here, "stub.s")).read().replace("@END@", str(1 + tracks))
        open(stub_src, "w").write(text)
        subprocess.run(rcasm + [stub_src, "-o", stub], check=True)
        subprocess.run([os.path.join(tools, "rcdisk" + exe), "create", out, "--force"], check=True)
        subprocess.run([os.path.join(tools, "rcdisk" + exe), "boot", out, stub, "--force"], check=True)
        data = open(body, "rb").read()
        if len(data) > TRACKS * TRACK:
            print(f"{src}: {len(data)} bytes, at most {TRACKS * TRACK}", file=sys.stderr)
            return 1
        image = bytearray(open(out, "rb").read())
        # Side 0 of cylinder c starts at sector 18 c of the file (two sides of 9 sectors per cylinder).
        for i in range(0, len(data), TRACK):
            start = (18 * (1 + i // TRACK)) * SECTOR
            chunk = data[i:i + TRACK]
            image[start:start + len(chunk)] = chunk
        open(out, "wb").write(image)
    return 0


if __name__ == "__main__":
    sys.exit(main())
