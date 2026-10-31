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
    let (set_s, set_c) = (set.s.clone(), set.c.clone());
    let mut machine = NeoGeo::new(Bus::new(bios, set.p, sm1, set.m));
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
        };
        if let Ok(reference) = std::env::var("RC_REFLUMA") {
            // search the sprite layouts for the one closest to the reference picture (the reference is 304 wide, 8 px cropped)
            let reference = std::fs::read(reference).unwrap();
            let mut best = (u64::MAX, None);
            let perms4: Vec<[u8; 4]> = {
                let mut v = Vec::new();
                for a in 0..4u8 {
                    for b in 0..4u8 {
                        for c in 0..4u8 {
                            for d in 0..4u8 {
                                if a != b && a != c && a != d && b != c && b != d && c != d {
                                    v.push([a, b, c, d]);
                                }
                            }
                        }
                    }
                }
                v
            };
            for blocks in perms4
                .iter()
                .filter(|b| **b == [2, 0, 3, 1] || std::env::var("RC_SHIFT").is_err())
            {
                for planes in perms4
                    .iter()
                    .filter(|p| **p == [0, 2, 1, 3] || std::env::var("RC_SHIFT").is_err())
                {
                    for msb_left in [false] {
                        let layout = neogeo::video::SpriteLayout {
                            blocks: *blocks,
                            planes: *planes,
                            msb_left,
                        };
                        let src = neogeo::video::Sources {
                            layout,
                            ..neogeo::video::Sources {
                                vram: src.vram,
                                palette: src.palette,
                                fix: src.fix,
                                sprites: src.sprites,
                                layout,
                            }
                        };
                        let mut out = vec![0u32; neogeo::video::WIDTH * neogeo::video::HEIGHT];
                        neogeo::video::render(&src, &mut out);
                        let mut diff = u64::MAX;
                        for dy in -18i32..=18 {
                            for dx in -18i32..=18 {
                                let mut d = 0u64;
                                for y in 50..100i32 {
                                    for x in 40..270i32 {
                                        let (ox, oy) = ((x + 8 + dx) as usize, (y + dy) as usize);
                                        let px = out[oy * neogeo::video::WIDTH + ox];
                                        let l = ((px >> 16 & 255) * 3
                                            + (px >> 8 & 255) * 6
                                            + (px & 255))
                                            / 10;
                                        d += u64::from(l.abs_diff(u32::from(
                                            reference[y as usize * 304 + x as usize],
                                        )));
                                    }
                                }
                                if d < diff {
                                    diff = d;
                                    if std::env::var("RC_SHIFT").is_ok() {
                                        println!("shift dx {dx} dy {dy}: {d}");
                                    }
                                }
                            }
                        }
                        if diff < best.0 {
                            best = (diff, Some(layout));
                        }
                    }
                }
            }
            println!("best sprite layout: {best:?}");
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
