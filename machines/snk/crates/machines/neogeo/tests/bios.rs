//! Runs the BIOS on the bus until the first access no fact covers. Ignored: needs the owner's files.
//!
//!   RC_NEOGEO_ZIP=path/to/neogeo.zip  RC_BIOS_NAME=sp-s2.sp1  RC_GAME=path/to/game.zip  cargo test -p neogeo --test bios -- --ignored --nocapture

use mc68000::Cpu;
use neogeo::Bus;
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
    let mut bus = Bus::new(bios, set.p);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    let mut steps = 0u64;
    let mut recent: Vec<u32> = Vec::new();
    while bus.stop.is_none()
        && steps
            < std::env::var("RC_STEPS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5_000_000u64)
    {
        recent.push(cpu.pc);
        if recent.len() > 40 {
            recent.remove(0);
        }
        if let Ok(v) = std::env::var("RC_AT") {
            if cpu.pc == u32::from_str_radix(&v, 16).unwrap() {
                println!(
                    "at {v}, step {steps}: sr {:04X} a7 {:X}",
                    cpu.sr(),
                    cpu.a7()
                );
                println!("work $FD00: {:02X?}", &bus.work[0xFD00..0xFD20]);
                break;
            }
        }
        if (0xC1_6BA0..0xC1_6BB2).contains(&cpu.pc) && std::env::var("RC_TRAIL").is_ok() {
            println!(
                "entered at step {steps}: {:X?} sr {:04X} a7 {:X} irq {} clock {}",
                recent,
                cpu.sr(),
                cpu.a7(),
                bus.irq,
                bus.clock
            );
            break;
        }
        cpu.set_ipl(bus.ipl());
        let clocks = cpu.step(&mut bus);
        bus.advance(u64::from(clocks) * 2); // 12 MHz CPU clocks, 24 MHz master
        steps += 1;
    }
    println!("{steps} instructions, pc {:#08X}: {:?}", cpu.pc, bus.stop);
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
