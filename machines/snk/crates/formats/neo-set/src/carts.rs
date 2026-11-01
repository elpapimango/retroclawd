//! Program and sample ROM descrambling of the cartridge boards that carry an SMA, PVC or NEO-PCM2 chip (fact N53).
//!
//! The one documented exception, after NEO-CMC (see `CREDITS.md`): the bit orders, tables and block moves here follow MAME's
//! `prot_sma.cpp`, `prot_pvc.cpp` and `prot_pcm2.cpp` (BSD-3-Clause; copyright S. Smith, David Haywood, Fabio Priuli, Razoola,
//! Mr.K), which the owner approved to read for these chips. The data is handled as the set files store it: a 16-bit word is
//! `u16::from_le_bytes` of its file pair, as the program files are byte-swapped (fact N10).

/// Which protection board a set is on, as far as the machine has to model it after the ROMs are descrambled.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Board {
    /// Nothing the machine models beyond the plain bank register.
    #[default]
    Plain,
    /// Metal Slug 3's SMA chip: a scrambled bank register at `$2FFFE4`, a constant read at `$2FE446`.
    SmaMslug3,
    /// Metal Slug 5's PVC chip: 8 KiB of cartridge RAM at `$2FE000` with colour packing and the bank register inside it.
    PvcMslug5,
}

/// Output bit `n - 1` takes input bit `order[0]`, ..., output bit 0 takes input bit `order[n - 1]`, with `n = order.len()`.
fn bitswap(v: u32, order: &[u32]) -> u32 {
    order
        .iter()
        .fold(0, |acc, &src| (acc << 1) | ((v >> src) & 1))
}

fn word(rom: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([rom[2 * i], rom[2 * i + 1]])
}

fn set_word(rom: &mut [u8], i: usize, v: u16) {
    rom[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
}

/// Metal Slug 3: the image is 1 MiB of fixed program (its last 256 KiB the `neo-sma` file) and 8 MiB of banks. Returns `false`
/// and leaves the data when the image is not 9 MiB.
pub fn mslug3_decrypt_68k(rom: &mut [u8]) -> bool {
    if rom.len() != 0x90_0000 {
        return false;
    }
    // swap data lines on the banked part
    for i in 0..0x80_0000 / 2 {
        let w = word(&rom[0x10_0000..], i);
        let w = bitswap(
            u32::from(w),
            &[4, 11, 14, 3, 1, 13, 0, 7, 2, 8, 12, 15, 10, 9, 5, 6],
        ) as u16;
        set_word(&mut rom[0x10_0000..], i, w);
    }
    // swap address lines and relocate the fixed part
    for i in 0..0x0c_0000 / 2 {
        let from = 0x5d_0000 / 2
            + bitswap(
                i as u32,
                &[
                    18, 15, 2, 1, 13, 3, 0, 9, 6, 16, 4, 11, 5, 7, 12, 17, 14, 10, 8,
                ],
            ) as usize;
        let w = word(rom, from);
        set_word(rom, i, w);
    }
    // swap address lines inside each 64 KiB of the banked part
    for block in (0..0x80_0000 / 2).step_by(0x1_0000 / 2) {
        let bank = &mut rom[0x10_0000 + 2 * block..0x10_0000 + 2 * block + 0x1_0000];
        let copy = bank.to_vec();
        for j in 0..0x1_0000 / 2 {
            let from = bitswap(
                j as u32,
                &[2, 11, 0, 14, 6, 4, 13, 8, 9, 3, 10, 7, 5, 12, 1],
            ) as usize;
            bank[2 * j..2 * j + 2].copy_from_slice(&copy[2 * from..2 * from + 2]);
        }
    }
    true
}

/// The start of each bank of Metal Slug 3 in the program image, by the bank number the game writes (already unscrambled).
const MSLUG3_BANKS: [u32; 49] = [
    0x000000, 0x020000, 0x040000, 0x060000, 0x070000, 0x090000, 0x0b0000, 0x0d0000, 0x0e0000,
    0x0f0000, 0x120000, 0x130000, 0x140000, 0x150000, 0x180000, 0x190000, 0x1a0000, 0x1b0000,
    0x1e0000, 0x1f0000, 0x200000, 0x210000, 0x240000, 0x250000, 0x260000, 0x270000, 0x2a0000,
    0x2b0000, 0x2c0000, 0x2d0000, 0x300000, 0x310000, 0x320000, 0x330000, 0x360000, 0x370000,
    0x380000, 0x390000, 0x3c0000, 0x3d0000, 0x400000, 0x410000, 0x440000, 0x450000, 0x460000,
    0x470000, 0x4a0000, 0x4b0000, 0x4c0000,
];

/// Where the window at `$200000` starts in the image after Metal Slug 3's game writes `sel` to `$2FFFE4`.
pub fn mslug3_bank_base(sel: u16) -> usize {
    let n = bitswap(u32::from(sel), &[9, 3, 6, 15, 12, 14]) as usize;
    0x10_0000 + MSLUG3_BANKS.get(n).copied().unwrap_or(0) as usize
}

/// Metal Slug 5 (and the other PVC boards' first generation): the whole 8 MiB program is scrambled. Returns `false` and
/// leaves the data when it is not 8 MiB.
pub fn mslug5_decrypt_68k(rom: &mut [u8]) -> bool {
    const XOR1: [u8; 0x20] = [
        0xc2, 0x4b, 0x74, 0xfd, 0x0b, 0x34, 0xeb, 0xd7, 0x10, 0x6d, 0xf9, 0xce, 0x5d, 0xd5, 0x61,
        0x29, 0xf5, 0xbe, 0x0d, 0x82, 0x72, 0x45, 0x0f, 0x24, 0xb3, 0x34, 0x1b, 0x99, 0xea, 0x09,
        0xf3, 0x03,
    ];
    const XOR2: [u8; 0x20] = [
        0x36, 0x09, 0xb0, 0x64, 0x95, 0x0f, 0x90, 0x42, 0x6e, 0x0f, 0x30, 0xf6, 0xe5, 0x08, 0x30,
        0x64, 0x08, 0x04, 0x00, 0x2f, 0x72, 0x09, 0xa0, 0x13, 0xc9, 0x0b, 0xa0, 0x3e, 0xc2, 0x00,
        0x40, 0x2b,
    ];
    if rom.len() != 0x80_0000 {
        return false;
    }
    for (i, b) in rom.iter_mut().enumerate() {
        *b ^= if i < 0x10_0000 { XOR1 } else { XOR2 }[i % 0x20];
    }
    for i in (0x10_0000..0x80_0000).step_by(4) {
        let v = u32::from(rom[i + 1]) | u32::from(rom[i + 2]) << 8;
        let v = bitswap(v, &[15, 14, 13, 12, 10, 11, 8, 9, 6, 7, 4, 5, 3, 2, 1, 0]);
        rom[i + 1] = v as u8;
        rom[i + 2] = (v >> 8) as u8;
    }
    let buf = rom.to_vec();
    for i in 0..0x10_0000 / 0x1_0000 {
        let ofst = (i & 0xf0) + bitswap((i & 0x0f) as u32, &[7, 6, 5, 4, 1, 0, 3, 2]) as usize;
        rom[i * 0x1_0000..(i + 1) * 0x1_0000]
            .copy_from_slice(&buf[ofst * 0x1_0000..(ofst + 1) * 0x1_0000]);
    }
    for i in (0x10_0000..0x80_0000).step_by(0x100) {
        let ofst = (i & 0xf0_00ff)
            + ((i & 0x00_0f00) ^ 0x00_0700)
            + ((bitswap(((i & 0x0f_f000) >> 12) as u32, &[5, 4, 7, 6, 1, 0, 3, 2]) as usize) << 12);
        rom[i..i + 0x100].copy_from_slice(&buf[ofst..ofst + 0x100]);
    }
    let buf = rom.to_vec();
    rom[0x10_0000..0x20_0000].copy_from_slice(&buf[0x70_0000..0x80_0000]);
    rom[0x20_0000..0x80_0000].copy_from_slice(&buf[0x10_0000..0x70_0000]);
    true
}

/// The bank base the PVC chip hands out from its RAM words `$FF8` and `$FF9`, which it also rewrites.
pub fn pvc_bank_base(ram: &mut [u16]) -> usize {
    let bank = usize::from(ram[0xff8] >> 8) | usize::from(ram[0xff9]) << 8;
    ram[0xff8] = (ram[0xff8] & 0xfe00) | 0x00a0;
    ram[0xff9] &= 0x7fff;
    bank + 0x10_0000
}

/// What the PVC chip does when the game writes its RAM word `offset`: a palette word is unpacked into words `$FF1`/`$FF2`
/// (write `$FF0`), or packed back from `$FF4`/`$FF5` into `$FF6`.
pub fn pvc_after_write(ram: &mut [u16], offset: usize) {
    match offset {
        0xff0 => {
            let pen = ram[0xff0];
            let b = ((pen & 0x000f) << 1) | ((pen & 0x1000) >> 12);
            let g = ((pen & 0x00f0) >> 3) | ((pen & 0x2000) >> 13);
            let r = ((pen & 0x0f00) >> 7) | ((pen & 0x4000) >> 14);
            let s = (pen & 0x8000) >> 15;
            ram[0xff1] = (g << 8) | b;
            ram[0xff2] = (s << 8) | r;
        }
        0xff4 | 0xff5 => {
            let (gb, sr) = (ram[0xff4], ram[0xff5]);
            ram[0xff6] = ((gb & 0x001e) >> 1)
                | ((gb & 0x1e00) >> 5)
                | ((sr & 0x001e) << 7)
                | ((gb & 0x0001) << 12)
                | ((gb & 0x0100) << 5)
                | ((sr & 0x0001) << 14)
                | ((sr & 0x0100) << 7);
        }
        _ => {}
    }
}

/// NEO-PCM2, first generation: the words of each `value` bytes swap in halves of halves (`value` is 4, 8 or 16 on the boards).
pub fn pcm2_decrypt(rom: &mut [u8], value: usize) {
    if value < 4 || !value.is_power_of_two() {
        return;
    }
    for block in rom.chunks_exact_mut(value) {
        let copy = block.to_vec();
        let words = value / 2;
        for j in 0..words {
            let from = j ^ (value / 4);
            block[2 * j..2 * j + 2].copy_from_slice(&copy[2 * from..2 * from + 2]);
        }
    }
}

/// NEO-PCM2, later generation: the 16 MiB sample ROM is moved, address-scrambled and xored; `kind` picks the board's constants
/// (Metal Slug 5 is 2). Returns `false` and leaves the data when the ROM is not 16 MiB or `kind` is unknown.
pub fn pcm2_swap(rom: &mut [u8], kind: usize) -> bool {
    const ADDRS: [(u32, u32); 7] = [
        (0x000000, 0xa5000),
        (0xffce20, 0x01000),
        (0xfe2cf6, 0x4e001),
        (0xffac28, 0xc2000),
        (0xfeb2c0, 0x0a000),
        (0xff14ea, 0xa7001),
        (0xffb440, 0x02000),
    ];
    const XORS: [[u8; 8]; 7] = [
        [0xf9, 0xe0, 0x5d, 0xf3, 0xea, 0x92, 0xbe, 0xef],
        [0xc4, 0x83, 0xa8, 0x5f, 0x21, 0x27, 0x64, 0xaf],
        [0xc3, 0xfd, 0x81, 0xac, 0x6d, 0xe7, 0xbf, 0x9e],
        [0xc3, 0xfd, 0x81, 0xac, 0x6d, 0xe7, 0xbf, 0x9e],
        [0xcb, 0x29, 0x7d, 0x43, 0xd2, 0x3a, 0xc2, 0xb4],
        [0x4b, 0xa4, 0x63, 0x46, 0xf0, 0x91, 0xea, 0x62],
        [0x4b, 0xa4, 0x63, 0x46, 0xf0, 0x91, 0xea, 0x62],
    ];
    if rom.len() != 0x100_0000 || kind >= ADDRS.len() {
        return false;
    }
    let buf = rom.to_vec();
    let (add, xor_addr) = ADDRS[kind];
    for i in 0..0x100_0000u32 {
        let j = bitswap(
            i,
            &[
                23, 22, 21, 20, 19, 18, 17, 0, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 16,
            ],
        ) ^ xor_addr;
        let d = (i.wrapping_add(add) & 0xff_ffff) as usize;
        rom[j as usize] = buf[d] ^ XORS[kind][(j & 7) as usize];
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitswap_puts_the_first_listed_bit_on_top() {
        // bit 0 moved to the top of a 4-bit word
        assert_eq!(bitswap(0b0001, &[0, 3, 2, 1]), 0b1000);
        assert_eq!(bitswap(0b1000, &[0, 3, 2, 1]), 0b0100);
    }

    #[test]
    fn the_wrong_size_is_left_alone() {
        let mut a = vec![5u8; 100];
        assert!(!mslug3_decrypt_68k(&mut a));
        assert!(!mslug5_decrypt_68k(&mut a));
        assert!(!pcm2_swap(&mut a, 2));
        assert_eq!(a, vec![5u8; 100]);
    }

    #[test]
    fn pcm2_decrypt_swaps_the_halves_of_each_block_and_undoes_itself() {
        let mut a: Vec<u8> = (0..16).collect();
        pcm2_decrypt(&mut a, 8);
        // words 0,1,2,3 of a block of 8 bytes become 2,3,0,1
        assert_eq!(&a[..8], &[4, 5, 6, 7, 0, 1, 2, 3]);
        pcm2_decrypt(&mut a, 8);
        assert_eq!(a, (0..16).collect::<Vec<u8>>());
    }

    #[test]
    fn the_mslug5_decrypt_is_a_function_of_its_input_only() {
        let mut a: Vec<u8> = (0..0x80_0000u32).map(|i| (i * 31 + 7) as u8).collect();
        let mut b = a.clone();
        assert!(mslug5_decrypt_68k(&mut a));
        assert!(mslug5_decrypt_68k(&mut b));
        assert_eq!(a, b);
    }

    #[test]
    fn the_mslug3_decrypt_keeps_the_neo_sma_part_of_the_fixed_program() {
        let mut rom = vec![0u8; 0x90_0000];
        for b in &mut rom[0x0c_0000..0x10_0000] {
            *b = 0xA5;
        }
        assert!(mslug3_decrypt_68k(&mut rom));
        assert!(rom[0x0c_0000..0x10_0000].iter().all(|b| *b == 0xA5));
    }

    #[test]
    fn mslug3_banks_come_from_the_scrambled_register() {
        // sel with all the listed bits clear selects bank 0
        assert_eq!(mslug3_bank_base(0), 0x10_0000);
        // bit 14 is the least significant bank bit: bank 1 starts 0x20000 further
        assert_eq!(mslug3_bank_base(1 << 14), 0x10_0000 + 0x2_0000);
    }

    #[test]
    fn pvc_unpacks_a_colour_and_hands_out_a_bank() {
        let mut ram = vec![0u16; 0x1000];
        ram[0xff0] = 0x8FFF;
        pvc_after_write(&mut ram, 0xff0);
        assert_eq!(ram[0xff2] >> 8, 1, "the dark bit");
        ram[0xff8] = 0x0200;
        ram[0xff9] = 0x0001;
        assert_eq!(pvc_bank_base(&mut ram), 0x10_0000 + 0x0102);
        assert_eq!(ram[0xff8], 0x02a0, "the low byte becomes $A0, the top seven bits stay");
        ram[0xff9] = 0x8001;
        pvc_bank_base(&mut ram);
        assert_eq!(ram[0xff9], 0x0001, "bit 15 is cleared");
    }
}
