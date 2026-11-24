//! `ROM=game.pce START=E2E0 COUNT=24 cargo run --release -p pcengine --example dis`: disassembles bank 0 (logical `$E000-$FFFF`).
fn main() {
    let rom = std::fs::read(std::env::var("ROM").expect("ROM")).unwrap();
    let rom = if rom.len() % 8192 == 512 {
        &rom[512..]
    } else {
        &rom[..]
    };
    let mut pc = u16::from_str_radix(&std::env::var("START").expect("START"), 16).unwrap();
    let count: usize = std::env::var("COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    for _ in 0..count {
        let off = usize::from(pc.wrapping_sub(0xE000)) & 0x1FFF;
        let bytes: Vec<u8> = (0..7)
            .map(|i| rom.get(off + i).copied().unwrap_or(0))
            .collect();
        let (text, len) = huc6280::disassemble::disassemble(&bytes, pc);
        println!("{pc:04X}  {text}");
        pc = pc.wrapping_add(len as u16);
    }
}
