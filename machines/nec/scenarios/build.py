#!/usr/bin/env python3
"""Builds the test HuCards the PC Engine scenarios use. The cards are assembler sources in `cards/` (HuC6280, one `.toml`
layout each) built with `rcld` from retroclawd-tools (`RCLD=path/to/rcld`, else `rcld` on PATH); without `rcld`, or with `--python`,
a tiny table-driven assembler in this file builds the same cards. `--compare` builds both and checks that the cards are byte for byte
the same. Output goes to the git-ignored `machines/nec/software/scenarios/` folder.

    build.py [--python | --compare] [out_dir]

Cards:
  pce-tone.pce    channel 0 plays a 32-step square wave at register value $0FE (440.4 Hz) at full volume.
  pce-input.pce   shows the first pad's eight buttons as eight bars on the background layer (S2).
"""
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))


class Asm:
    def __init__(self, org=0xE000):
        self.org = org
        self.code = bytearray()
        self.labels = {}
        self.fixups = []

    def here(self):
        return self.org + len(self.code)

    def label(self, name):
        self.labels[name] = self.here()

    def b(self, *bytes_):
        self.code.extend(bytes_)

    def lda(self, v):
        self.b(0xA9, v)

    def ldx(self, v):
        self.b(0xA2, v)

    def sta_abs(self, a):
        self.b(0x8D, a & 0xFF, a >> 8)

    def lda_abs(self, a):
        self.b(0xAD, a & 0xFF, a >> 8)

    def st(self, n, v):
        self.b(0x03 + 0x10 * (0 if n == 0 else 1 if n == 1 else 2), v)

    def tam(self, mask):
        self.b(0x53, mask)

    def branch(self, op, name):
        self.b(op, 0)
        self.fixups.append((len(self.code) - 1, name))

    def jmp(self, name):
        self.b(0x4C, 0, 0)
        self.fixups.append((len(self.code) - 2, name, "abs"))

    def finish(self):
        for fx in self.fixups:
            pos, name = fx[0], fx[1]
            target = self.labels[name]
            if len(fx) == 3:
                self.code[pos] = target & 0xFF
                self.code[pos + 1] = target >> 8
            else:
                off = target - (self.org + pos + 1)
                assert -128 <= off <= 127, (name, off)
                self.code[pos] = off & 0xFF
        return bytes(self.code)


def card(code, size=0x20000, timer_vector=None):
    rom = bytearray([0xEA] * size)
    rom[: len(code)] = code
    rom[0x1FFE] = 0x00
    rom[0x1FFF] = 0xE0
    if timer_vector is not None:
        rom[0x1FFA] = timer_vector & 0xFF
        rom[0x1FFB] = timer_vector >> 8
    return bytes(rom)


def preamble(a):
    a.b(0x78, 0xD4)  # SEI, CSH
    a.lda(0xF8)
    a.tam(0x02)  # MPR1 = RAM
    a.lda(0xFF)
    a.tam(0x01)  # MPR0 = hardware page
    a.ldx(0xFF)
    a.b(0x9A)  # TXS


def tone():
    a = Asm()
    preamble(a)
    for reg, v in ((0x0800, 0), (0x0801, 0xFF), (0x0802, 0xFE), (0x0803, 0x00), (0x0804, 0x00)):
        a.lda(v)
        a.sta_abs(reg)
    for i in range(32):
        a.lda(0x1F if i < 16 else 0x00)
        a.sta_abs(0x0806)
    a.lda(0xFF)
    a.sta_abs(0x0805)
    a.lda(0x9F)
    a.sta_abs(0x0804)
    a.label("spin")
    a.branch(0x80, "spin")  # BRA
    return card(a.finish())


def timer_probe(reload, interrupts=False, ack=False):
    """Starts the timer with the given reload value and stores 48 reads of its counter, ~110 cycles apart, in zero page $20-$4F."""
    a = Asm()
    preamble(a)
    a.lda_abs(0x0C00)
    a.b(0x85, 0x50)  # STA $50: the counter before the timer is ever started
    if interrupts:
        a.lda(0x01)
        a.sta_abs(0x1402)  # IRQ2 off, IRQ1 and the timer on
    a.lda(reload)
    a.sta_abs(0x0C00)
    a.lda(0x01)
    a.sta_abs(0x0C01)
    if ack:
        a.sta_abs(0x1403)
    if interrupts:
        a.b(0x58)  # CLI
    a.ldx(0x00)
    a.label("loop")
    a.lda_abs(0x0C00)
    a.b(0x95, 0x20)  # STA $20,X
    a.b(0xA0, 0x18)  # LDY #$18
    a.label("delay")
    a.b(0x88)  # DEY
    a.branch(0xD0, "delay")
    a.b(0xE8)  # INX
    a.b(0xE0, 0x30)  # CPX #$30
    a.branch(0xD0, "loop")
    a.label("spin")
    a.branch(0x80, "spin")
    if interrupts:
        a.label("isr")
        a.sta_abs(0x1403)  # acknowledge
        a.b(0x40)  # RTI
        return card(a.finish(), timer_vector=a.labels["isr"])
    return card(a.finish())


def input_bars():
    """Writes a palette, one tile (solid colour 1) and a background map; then each frame draws pad bits as tiles."""
    a = Asm()
    preamble(a)
    # palette: entry $100 (the border) = black
    for reg, v in ((0x0402, 0), (0x0403, 1), (0x0404, 0), (0x0405, 0)):
        a.lda(v)
        a.sta_abs(reg)
    # palette: entry 0 = black, entry 1 = white
    for reg, v in ((0x0402, 0), (0x0403, 0), (0x0404, 0), (0x0405, 0), (0x0404, 0xFF), (0x0405, 0x01)):
        a.lda(v)
        a.sta_abs(reg)
    # VRAM is not cleared at power on: blank tile 0 (words 0-15) and the whole first 32x32 map (words 0-1023
    # hold map entries here, so clear 0-0x3FF; tile data of tile 1 follows below at words 16-31)
    a.st(0, 0x00)
    a.lda(0x00)
    a.sta_abs(0x0002)
    a.sta_abs(0x0003)
    a.st(0, 0x02)
    a.ldx(0x00)
    a.label("clear")
    for _ in range(4):
        a.lda(0x00)
        a.sta_abs(0x0002)
        a.sta_abs(0x0003)
    a.b(0xE8)  # INX
    a.branch(0xD0, "clear")  # BNE: 256 x 4 words
    a.ldx(0xFF)
    # tile 1 at VRAM word 16..31: plane 0 all ones
    a.st(0, 0x00)  # MAWR
    a.lda(0x10)
    a.sta_abs(0x0002)
    a.lda(0x00)
    a.sta_abs(0x0003)
    a.st(0, 0x02)
    for _ in range(8):
        a.lda(0xFF)
        a.sta_abs(0x0002)
        a.lda(0x00)
        a.sta_abs(0x0003)
    for _ in range(8):
        a.lda(0x00)
        a.sta_abs(0x0002)
        a.sta_abs(0x0003)
    # control: background on, vblank interrupt off
    a.st(0, 0x05)
    a.lda(0x80)
    a.sta_abs(0x0002)
    a.lda(0x00)
    a.sta_abs(0x0003)
    # display registers: 256 wide, 224 high
    for r, lo, hi in ((0x0B, 0x1F, 0x00), (0x0C, 0x02, 0x0F), (0x0D, 0xDF, 0x00), (0x0E, 0x04, 0x00), (0x09, 0x00, 0x00)):
        a.st(0, r)
        a.lda(lo)
        a.sta_abs(0x0002)
        a.lda(hi)
        a.sta_abs(0x0003)
    a.label("frame")
    # wait for vblank: read status until bit 5 would be set -- polling the line counter is not available, so
    # just repeat the read forever; the picture is drawn once per loop iteration, timing does not matter
    # the port needs a few cycles to settle after a write: reset the multitap counter (CLR), then read directions
    # with SEL = 1 and buttons with SEL = 0, as games do
    for v in (0x01, 0x03, 0x01):
        a.lda(v)
        a.sta_abs(0x1000)
    a.b(0x48, 0x68, 0xEA)  # PHA PLA NOP
    a.lda_abs(0x1000)
    a.b(0x85, 0x01)  # directions in bits 0-3 (up right down left)
    a.lda(0x00)
    a.sta_abs(0x1000)
    a.b(0x48, 0x68, 0xEA)
    a.lda_abs(0x1000)
    a.b(0x85, 0x00)  # buttons in bits 0-3 (I II select run)
    # draw 8 bars: tile at map position (bit * 2, 4): written as VRAM words; set MAWR then data
    for bit in range(8):
        zp = 0x00 if bit < 4 else 0x01
        shift = bit & 3
        a.st(0, 0x00)
        a.lda(0x40 + bit * 2)
        a.sta_abs(0x0002)
        a.lda(0x00 + 0x01)  # row 4 of a 32-wide map: word 4*32 = 128 = 0x80 -> high byte 0
        a.sta_abs(0x0003)
        a.st(0, 0x02)
        a.b(0xA5, zp)  # LDA zp
        for _ in range(shift):
            a.b(0x4A)  # LSR A
        a.b(0x29, 0x01)  # AND #1  (1 = released)
        a.b(0x49, 0x01)  # EOR #1  (1 = pressed)
        a.sta_abs(0x0002)
        a.lda(0x00)
        a.sta_abs(0x0003)
    a.jmp("frame")
    return card(a.finish())


# name -> (layout in cards/, -D defines)
CARDS = {
    "pce-tone.pce": ("tone", []),
    "pce-input.pce": ("input", []),
    "pce-timer0.pce": ("timer", ["RELOAD=0"]),
    "pce-timer5.pce": ("timer", ["RELOAD=5"]),
    "pce-timer0i.pce": ("timer", ["RELOAD=0", "IRQ=1"]),
    "pce-timer5i.pce": ("timer", ["RELOAD=5", "IRQ=1"]),
    "pce-timer0a.pce": ("timer", ["RELOAD=0", "ACK=1"]),
    "pce-timer5a.pce": ("timer", ["RELOAD=5", "ACK=1"]),
}


def python_cards():
    return {
        "pce-tone.pce": tone(),
        "pce-input.pce": input_bars(),
        "pce-timer0.pce": timer_probe(0),
        "pce-timer5.pce": timer_probe(5),
        "pce-timer0i.pce": timer_probe(0, True),
        "pce-timer5i.pce": timer_probe(5, True),
        "pce-timer0a.pce": timer_probe(0, False, True),
        "pce-timer5a.pce": timer_probe(5, False, True),
    }


def find_rcld():
    path = os.environ.get("RCLD")
    if path and os.path.exists(path):
        return path
    return shutil.which("rcld")


def tool_cards(rcld):
    """Builds every card with rcld and returns name -> bytes."""
    cards = {}
    with tempfile.TemporaryDirectory() as tmp:
        for name, (layout, defines) in CARDS.items():
            out = os.path.join(tmp, name)
            cmd = [rcld, "build", os.path.join(HERE, "cards", layout + ".toml"), "-o", out, "--force"]
            for d in defines:
                cmd += ["-D", d]
            subprocess.run(cmd, check=True, stderr=subprocess.DEVNULL)
            with open(out, "rb") as f:
                cards[name] = f.read()
    return cards


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    out = args[0] if args else os.path.join(ROOT, "machines", "nec", "software", "scenarios")
    use_python = "--python" in sys.argv
    rcld = None if use_python else find_rcld()
    if "--compare" in sys.argv:
        if not rcld:
            sys.exit("--compare needs rcld (RCLD=path or on PATH)")
        mine, theirs = python_cards(), tool_cards(rcld)
        bad = 0
        for name in sorted(mine):
            same = mine[name] == theirs[name]
            print(("same     " if same else "DIFFERENT"), name, len(mine[name]), len(theirs[name]))
            bad += not same
        sys.exit(1 if bad else 0)
    cards = tool_cards(rcld) if rcld else python_cards()
    print("built with", rcld if rcld else "the Python assembler in build.py")
    os.makedirs(out, exist_ok=True)
    for name, data in cards.items():
        with open(os.path.join(out, name), "wb") as f:
            f.write(data)
        print("wrote", os.path.join(out, name))


if __name__ == "__main__":
    main()
