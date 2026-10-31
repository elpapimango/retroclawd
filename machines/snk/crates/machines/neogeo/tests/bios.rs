//! Runs the BIOS on the bus until the first access no fact covers. Ignored: needs the owner's files.
//!
//!   RC_NEOGEO_ZIP=path/to/neogeo.zip  RC_BIOS_NAME=sp-s2.sp1  RC_GAME=path/to/game.zip  cargo test -p neogeo --test bios -- --ignored --nocapture

use neogeo::{Bus, system::NeoGeo};

#[derive(Default, Debug)]
#[allow(dead_code)]
struct SpriteLayoutPrint {
    blocks: [u8; 4],
    planes: [u8; 4],
    msb_left: bool,
}
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
    let (set_s, mut set_c) = (set.s.clone(), set.c.clone());
    if std::env::var("RC_CSWAP").is_ok() {
        let half = set_c.len() / 2;
        let (a, b) = set_c.split_at(half);
        set_c = [b, a].concat();
    }
    let graphics = neogeo::system::Graphics {
        sfix: sfix.clone(),
        lo: lo.clone(),
        s: set.s.clone(),
        c: set.c.clone(),
    };
    let mut machine = NeoGeo::new(Bus::new(bios, set.p, sm1, set.m), graphics);
    machine.trace_nth_nmi = std::env::var("RC_NMI").ok().and_then(|v| v.parse().ok());
    if let Ok(path) = std::env::var("RC_LOAD_BACKUP") {
        let data = std::fs::read(path).unwrap();
        machine.bus.backup[..data.len()].copy_from_slice(&data);
    }
    if let Ok(v) = std::env::var("RC_RTC_START") {
        // yy,mm,dd,weekday
        let f: Vec<u32> = v.split(',').map(|x| x.parse().unwrap()).collect();
        machine.bus.rtc_start = ((f[0], f[1], f[2], 0, 0, 0), f[3]);
    }
    machine.bus.lenient = std::env::var("RC_LENIENT").is_ok();
    if let Ok(v) = std::env::var("RC_DIPS") {
        machine.bus.dips = u8::from_str_radix(&v, 16).unwrap();
    }
    let limit = std::env::var("RC_STEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5_000_000u64);
    let mut steps = 0u64;
    let mut recent: Vec<u32> = Vec::new();
    // RC_PCHIST=from_frame: after that frame, count the 68000's program counter (rounded to 4) and print the busiest places
    let hist_from: Option<u64> = std::env::var("RC_PCHIST").ok().and_then(|v| v.parse().ok());
    let mut hist: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
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
        if let Some(from) = hist_from
            && machine.bus.clock >= from * neogeo::LINE_CLOCKS * neogeo::FRAME_LINES
        {
            *hist.entry(machine.cpu.pc & !3).or_default() += 1;
        }
        machine.step();
        steps += 1;
    }
    if let Ok(path) = std::env::var("RC_DUMP_WORK") {
        std::fs::write(path, &machine.bus.work[..]).unwrap();
    }
    println!(
        "irq counts (blank raised, timer raised, blank acked, timer acked): {:?}",
        machine.bus.irq_counts
    );
    if hist_from.is_some() {
        let mut v: Vec<_> = hist.into_iter().collect();
        v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        for (pc, n) in v.iter().take(24) {
            println!("pc {pc:06X}: {n}");
        }
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
    if let Ok(range) = std::env::var("RC_RAW") {
        let (a, b) = range.split_once('-').unwrap();
        for n in a.parse::<usize>().unwrap()..=b.parse::<usize>().unwrap() {
            println!(
                "sprite {n}: scb2 {:04X} scb3 {:04X} scb4 {:04X} first tiles {:04X?}",
                bus.vram[0x8000 + n],
                bus.vram[0x8200 + n],
                bus.vram[0x8400 + n],
                &bus.vram[n * 64..n * 64 + 8]
            );
        }
    }
    if let Ok(spec) = std::env::var("RC_FINDTILE") {
        // "chain,col,row,x,y,reference-luma-file": find which C ROM tile looks like the reference at that place
        let f: Vec<&str> = spec.split(',').collect();
        let (n, row): (usize, usize) = (
            f[0].parse::<usize>().unwrap() + f[1].parse::<usize>().unwrap(),
            f[2].parse().unwrap(),
        );
        let (x0, y0): (usize, usize) = (f[3].parse().unwrap(), f[4].parse().unwrap());
        let reference = std::fs::read(f[5]).unwrap();
        let (scb_tile, attr) = (bus.vram[n * 64 + row * 2], bus.vram[n * 64 + row * 2 + 1]);
        let pal = usize::from(attr >> 8);
        let at = if bus.latch & 0x80 == 0 {
            neogeo::PALETTE_RAM
        } else {
            0
        };
        let colour = |c: usize| {
            let i = at + (pal * 16 + c) * 2;
            neogeo::video::colour(u16::from(bus.palette[i]) << 8 | u16::from(bus.palette[i + 1]))
        };
        let lum = |px: u32| ((px >> 16 & 255) * 3 + (px >> 8 & 255) * 6 + (px & 255)) / 10;
        let layout = neogeo::video::SpriteLayout::default();
        let tiles = set_c.len() / 128;
        let mut results: Vec<(u64, usize)> = Vec::new();
        for t in 0..tiles {
            let mut d = 0u64;
            for y in 0..16 {
                for x in 0..16 {
                    let c = neogeo::video::sprite_pixel(&set_c, &layout, t, x, y);
                    let ours = if c == 0 {
                        0
                    } else {
                        lum(colour(usize::from(c)))
                    };
                    let r = u32::from(reference[(y0 + y) * 304 + x0 + x]);
                    d += u64::from(ours.abs_diff(r));
                }
            }
            results.push((d, t));
        }
        results.sort();
        println!(
            "tile in SCB1: {scb_tile:04X}; best matches (diff, tile): {:X?}",
            &results[..4]
        );
    }
    if let Ok(range) = std::env::var("RC_TILES") {
        let (a, b) = range.split_once('-').unwrap();
        for row in 0..12usize {
            let mut line = String::new();
            for n in a.parse::<usize>().unwrap()..=b.parse::<usize>().unwrap() {
                let (t, at) = (bus.vram[n * 64 + row * 2], bus.vram[n * 64 + row * 2 + 1]);
                line.push_str(&format!("{t:04X}/{:02X}{:X} ", at >> 8, at & 0xFF));
            }
            println!("row {row}: {line}");
        }
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
        let mut vram = bus.vram.clone();
        if std::env::var("RC_NOSPR").is_ok() {
            for n in 0..381 {
                vram[0x8200 + n] = 0;
            }
        }
        let src = neogeo::video::Sources {
            vram: &vram,
            palette: &bus.palette[if bus.latch & 0x80 == 0 {
                neogeo::PALETTE_RAM
            } else {
                0
            }..][..neogeo::PALETTE_RAM],
            fix: if std::env::var("RC_NOFIX").is_ok() {
                &[]
            } else if bus.latch & 0x20 != 0 && !set_s.is_empty() {
                &set_s
            } else {
                &sfix
            },
            sprites: &set_c,
            layout: neogeo::video::SpriteLayout::default(),
            lo: &lo,
            anim: bus.anim,
        };
        if let Ok(reference) = std::env::var("RC_REFLUMA") {
            // try every block and plane order against a 304-wide reference picture (8 columns cut at each side)
            let reference = std::fs::read(reference).unwrap();
            let mut perms = Vec::new();
            for a in 0..4u8 {
                for b in 0..4u8 {
                    for c in 0..4u8 {
                        for d in 0..4u8 {
                            if a != b && a != c && a != d && b != c && b != d && c != d {
                                perms.push([a, b, c, d]);
                            }
                        }
                    }
                }
            }
            let mut scored: Vec<(u64, [u8; 4], [u8; 4], bool)> = Vec::new();
            let score = |layout: neogeo::video::SpriteLayout, dx: i32, dy: i32| -> u64 {
                let trial = neogeo::video::Sources {
                    vram: src.vram,
                    palette: src.palette,
                    fix: src.fix,
                    sprites: src.sprites,
                    layout,
                    lo: src.lo,
                    anim: src.anim,
                };
                let mut out = vec![0u32; neogeo::video::WIDTH * neogeo::video::HEIGHT];
                neogeo::video::render(&trial, &mut out);
                let mut d = 0u64;
                for y in 40..130i32 {
                    for x in 30..240i32 {
                        let (ox, oy) = ((x + 8 + dx) as usize, (y + dy) as usize);
                        let px = out[oy * neogeo::video::WIDTH + ox];
                        let l = ((px >> 16 & 255) * 3 + (px >> 8 & 255) * 6 + (px & 255)) / 10;
                        d += u64::from(
                            l.abs_diff(u32::from(reference[y as usize * 304 + x as usize])),
                        );
                    }
                }
                d
            };
            for blocks in &perms {
                for planes in &perms {
                    for msb_left in [false, true] {
                        let layout = neogeo::video::SpriteLayout {
                            blocks: *blocks,
                            planes: *planes,
                            msb_left,
                        };
                        scored.push((score(layout, 0, 0), *blocks, *planes, msb_left));
                    }
                }
            }
            scored.sort_by_key(|e| e.0);
            let mut best = (u64::MAX, SpriteLayoutPrint::default());
            for (_, blocks, planes, msb_left) in scored.iter().take(40) {
                let layout = neogeo::video::SpriteLayout {
                    blocks: *blocks,
                    planes: *planes,
                    msb_left: *msb_left,
                };
                for dy in -6..=6 {
                    for dx in -6..=6 {
                        let d = score(layout, dx, dy);
                        if d < best.0 {
                            best = (
                                d,
                                SpriteLayoutPrint {
                                    blocks: *blocks,
                                    planes: *planes,
                                    msb_left: *msb_left,
                                },
                            );
                        }
                    }
                }
            }
            println!("top unshifted: {:?}", &scored[..3]);
            println!("best layout: {best:?}");
        }
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
    println!("vram writes past $87FF: {}", bus.vram_oob);
    if bus.lenient {
        println!("unmapped accesses: {}", bus.access_log.len());
        for (addr, write, value, pc) in bus.access_log.iter().take(80) {
            println!(
                "  {} {addr:06X} {value:04X} pc {pc:06X}",
                if *write { "W" } else { "R" }
            );
        }
    }
    if let Ok(path) = std::env::var("RC_DUMP_BACKUP") {
        std::fs::write(path, &bus.backup).unwrap();
    }
    if std::env::var("RC_RTC").is_ok() {
        let log = &bus.rtc_log;
        println!("rtc log entries: {}", log.len());
        let mut last_read = 0xFFu8;
        let mut shown = 0;
        for (i, (k, v, pc)) in log.iter().enumerate() {
            if *k == 'R' {
                if *v == last_read {
                    continue;
                }
                last_read = *v;
            }
            println!("  {i} {k} {v:02X} pc {pc:06X}");
            shown += 1;
            if shown > 400 {
                break;
            }
        }
    }
    if std::env::var("RC_BIOSVARS").is_ok() {
        let w = |at: usize| bus.work[at & 0xFFFF];
        println!(
            "work $10FD80..: {:02X?}; $10FDAE {:02X}; backup $D00100: {:02X?}",
            (0..8).map(|i| w(0xFD80 + i)).collect::<Vec<_>>(),
            w(0xFDAE),
            &bus.backup[0x100..0x110]
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
    // RC_LISTP=hex address: the instructions from there in the cartridge's program
    if let Ok(from) = std::env::var("RC_LISTP") {
        let mut at = u32::from_str_radix(from.trim_start_matches("0x"), 16).unwrap();
        for _ in 0..40 {
            let off = at as usize;
            let (text, len) =
                mc68000::disassemble::disassemble(&bus.p[off..(off + 12).min(bus.p.len())], at);
            println!("P {at:06X}  {text}");
            at += len as u32;
        }
    }
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
