import struct
import unittest

import mkrom


class MkromTests(unittest.TestCase):
    def test_the_checksum_is_the_sum_of_the_words_from_0x200(self):
        rom = mkrom.build(b"\x4E\x71" * 10)
        words = struct.unpack(">%dH" % ((len(rom) - 0x200) // 2), rom[0x200:])
        self.assertEqual(struct.unpack(">H", rom[0x18E:0x190])[0], sum(words) & 0xFFFF)

    def test_header_fields_sit_where_the_documentation_puts_them(self):
        rom = mkrom.build(b"\x4E\x71", region="U")
        self.assertEqual(rom[0x100:0x110], b"SEGA MEGA DRIVE ")
        self.assertEqual(struct.unpack(">I", rom[0x1A4:0x1A8])[0], len(rom) - 1)
        self.assertEqual(rom[0x1F0:0x1F3], b"U  ")
        self.assertEqual(struct.unpack(">II", rom[0:8]), (0x00FFFE00, 0x200))

    def test_the_size_is_a_multiple_of_128_and_odd_code_is_padded(self):
        rom = mkrom.build(b"\x4E\x71\x4E")
        self.assertEqual(len(rom) % 128, 0)


if __name__ == "__main__":
    unittest.main()
