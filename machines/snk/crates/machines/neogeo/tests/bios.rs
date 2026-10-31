//! Runs the BIOS on the bus until the first access no fact covers. Ignored: needs the owner's files.
//!
//!   RC_NEOGEO_ZIP=path/to/neogeo.zip  RC_BIOS_NAME=sp-s2.sp1  RC_GAME=path/to/game.zip  cargo test -p neogeo --test bios -- --ignored --nocapture

use neogeo::{Bus, system::NeoGeo};
use std::io::Read;

#[test]
#[ignore]
fn first_fetches() {
    let (Ok(zip_path), Ok(game)) = (std::env::var("RC_NEOGEO_ZIP"), std::env::var("RC_GAME"))
    else {
        return;
    };
    let name = std::env::var("RC_BIOS_NAME").unwrap_or_else(|_| "sp-s2.sp1".to_string());
    let mut zip = zip::ZipArchive::new(std::fs::File::open(zip_path).unwrap()).unwrap();
    let mut bios = Vec::new();
    zip.by_name(&name).unwrap().read_to_end(&mut bios).unwrap();
    let set = neo_set::load(&std::fs::read(game).unwrap()).unwrap();
    let mut sm1 = Vec::new();
    zip.by_name("sm1.sm1")
        .unwrap()
        .read_to_end(&mut sm1)
        .unwrap();
    let mut sfix = Vec::new();
    zip.by_name("sfix.sfix")
        .unwrap()
        .read_to_end(&mut sfix)
        .unwrap();
    let mut lo = Vec::new();
    zip.by_name("000-lo.lo")
        .unwrap()
        .read_to_end(&mut lo)
        .unwrap();
    let (set_s, set_c) = (set.s.clone(), set.c.clone());
    let graphics = neogeo::system::Graphics {
        sfix: sfix.clone(),
        lo: lo.clone(),
        s: set.s.clone(),
        c: set.c.clone(),
    };
    let mut machine = NeoGeo::new(Bus::new(bios, set.p, sm1, set.m), graphics);
    machine.trace_nth_nmi = std::env::var("RC_NMI").ok().and_then(|v| v.parse().ok());
    let limit = std::env::var("RC_STEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5_000_000u64);
    let mut steps = 0u64;
    let mut recent: Vec<u32> = Vec::new();
    let frames_wanted: Option<u64> = std::env::var("RC_FRAMES").ok().and_then(|v| v.parse().ok());
    while machine.bus.stop.is_none()
        && steps < limit
        && frames_wanted
            .is_none_or(|f| machine.bus.clock < f * neogeo::LINE_CLOCKS * neogeo::FRAME_LINES)
    {
        recent.push(machine.cpu.pc);
        if recent.len() > 40 {
            recent.remove(0);
        }
        if let Ok(v) = std::env::var("RC_AT")
            && machine.cpu.pc == u32::from_str_radix(&v, 16).unwrap()
        {
            println!("at {v}, step {steps}: {recent:X?}");
            break;
        }
        machine.step();
        steps += 1;
    }
    if std::env::var("RC_AUDIO").is_ok() {
        let samples = machine.drain_audio();
        let peak = samples.iter().fold(0f32, |m, v| m.max(v.abs()));
        let rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len().max(1) as f32).sqrt();
        println!(
            "audio: {} samples, peak {peak:.4}, rms {rms:.4}",
            samples.len()
        );
    }
    let (cpu, bus) = (&machine.cpu, &machine.bus);
    println!("{steps} instructions, pc {:#08X}: {:?}", cpu.pc, bus.stop);
    let frames = bus.clock / (neogeo::LINE_CLOCKS * neogeo::FRAME_LINES);
    let nz = |r: std::ops::Range<usize>| bus.vram[r].iter().filter(|w| **w != 0).count();
    println!(
        "frame {frames}: sr {:04X} irq {} vram nonzero: scb1 {} fix {} scb2 {} scb3 {} scb4 {}; palette {}",
        cpu.sr(),
        bus.irq,
        nz(0..0x7000),
        nz(0x7000..0x7500),
        nz(0x8000..0x8200),
        nz(0x8200..0x8400),
        nz(0x8400..0x8600),
        bus.palette.iter().filter(|b| **b != 0).count()
    );
    if std::env::var("RC_PALETTE").is_ok() {
        for bank in 0..2usize {
            for pal in [0usize, 1, 254, 255] {
                let at = bank * neogeo::PALETTE_RAM + pal * 32;
                let words: Vec<String> = (0..16)
                    .map(|c| {
                        format!(
                            "{:04X}",
                            u16::from(bus.palette[at + c * 2]) << 8
                                | u16::from(bus.palette[at + c * 2 + 1])
                        )
                    })
                    .collect();
                println!("bank {bank} palette {pal}: {}", words.join(" "));
            }
        }
        println!("latch {:02X}", bus.latch);
    }
    if std::env::var("RC_CHAINS").is_ok() {
        // the chains: first sprite, how many follow, top line, height in tiles, shrink
        let mut n = 0;
        while n < 381 {
            let b3 = bus.vram[0x8200 + n];
            if b3 & 0x40 == 0 && b3 & 0x3F != 0 {
                let mut len = 1;
                while n + len < 381 && bus.vram[0x8200 + n + len] & 0x40 != 0 {
                    len += 1;
                }
                let b2 = bus.vram[0x8000 + n];
                println!(
                    "chain {n}+{len}: top {} height {} shrink h {} v {} x {}",
                    (496 - i32::from(b3 >> 7)) & 0x1FF,
                    b3 & 0x3F,
                    b2 >> 8 & 0xF,
                    b2 & 0xFF,
                    bus.vram[0x8400 + n] >> 7
                );
                n += len;
            } else {
                n += 1;
            }
        }
    }
    if std::env::var("RC_SPRITES").is_ok() {
        // sprites with a height: n, scb2 (shrink), scb3 (y, sticky, height), scb4 (x)
        let mut shown = 0;
        for n in 0..381usize {
            let (b2, b3, b4) = (
                bus.vram[0x8000 + n],
                bus.vram[0x8200 + n],
                bus.vram[0x8400 + n],
            );
            if b3 & 0x3F != 0 && (b2 != 0x0FFF || std::env::var("RC_ALL").is_ok()) && shown < 60 {
                println!(
                    "sprite {n}: shrink h {} v {}, y {} sticky {} height {}, x {}",
                    b2 >> 8 & 0xF,
                    b2 & 0xFF,
                    b3 >> 7,
                    b3 >> 6 & 1,
                    b3 & 0x3F,
                    b4 >> 7
                );
                shown += 1;
            }
        }
    }
    if let Ok(path) = std::env::var("RC_OUT") {
        let sfix = sfix.clone();
        let src = neogeo::video::Sources {
            vram: &bus.vram,
            palette: &bus.palette[if bus.latch & 0x80 == 0 {
                neogeo::PALETTE_RAM
            } else {
                0
            }..][..neogeo::PALETTE_RAM],
            fix: if bus.latch & 0x20 != 0 && !set_s.is_empty() {
                &set_s
            } else {
                &sfix
            },
            sprites: &set_c,
            layout: neogeo::video::SpriteLayout::default(),
            lo: &lo,
            anim: bus.anim,
        };
        let mut out = vec![0u32; neogeo::video::WIDTH * neogeo::video::HEIGHT];
        neogeo::video::render(&src, &mut out);
        let mut ppm = format!(
            "P6
{} {}
255
",
            neogeo::video::WIDTH,
            neogeo::video::HEIGHT
        )
        .into_bytes();
        for px in out {
            ppm.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, px as u8]);
        }
        std::fs::write(path, ppm).unwrap();
    }
    if !machine.z80_trace.is_empty() {
        // the Z80 instructions of the traced NMI, with the sound program's bytes
        let mut seen = std::collections::BTreeSet::new();
        for pc in &machine.z80_trace {
            seen.insert(*pc);
        }
        let rom = if bus.sound.use_cart {
            &bus.sound.m1_cart
        } else {
            &bus.sound.m1_board
        };
        let mut lines = Vec::new();
        for pc in &seen {
            let at = usize::from(*pc);
            if at < 0x8000 {
                let (text, _) =
                    z80::disassemble::disassemble_at(&rom[at..(at + 4).min(rom.len())], *pc);
                lines.push(format!("{pc:04X}  {text}"));
            }
        }
        println!(
            "z80 nmi trace ({} steps, {} distinct pcs):
{}",
            machine.z80_trace.len(),
            seen.len(),
            lines.join(
                "
"
            )
        );
    }
    if std::env::var("RC_YM").is_ok() {
        let seen: Vec<String> = bus
            .sound
            .ym_writes
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .map(|(i, n)| format!("{}:{:02X}x{n}", i / 256, i % 256))
            .collect();
        println!("ym registers written: {}", seen.join(" "));
        println!("key on log: {:02X?}", bus.sound.key_log);
        println!("z80 banks: {:02X?}", bus.sound.banks);
        println!(
            "z80 regs: {:?}; ym status {:02X}",
            machine.z80.registers(),
            bus.sound.ym.read(0)
        );
        println!("commands and replies: {:02X?}", bus.sound.command_log);
    }
    println!(
        "z80 pc {:#06X}, reply {:#04X}, command {:#04X}",
        machine.z80.pc, bus.sound.reply, bus.sound.command
    );
    // RC_LIST=hex address: the instructions from there (the system ROM only)
    if let Ok(from) = std::env::var("RC_LIST") {
        let mut at = u32::from_str_radix(from.trim_start_matches("0x"), 16).unwrap();
        for _ in 0..16 {
            let off = (at & 0x1_FFFF) as usize;
            let (text, len) = mc68000::disassemble::disassemble(
                &bus.bios[off..(off + 12).min(bus.bios.len())],
                at,
            );
            println!("{at:06X}  {text}");
            at += len as u32;
        }
    }
}
