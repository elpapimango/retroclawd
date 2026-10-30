//! Runs a real BIOS (`RC_BIOS=path`) and reports where both CPUs are. Ignored by default: the BIOS is firmware and never committed.
//! `RC_BIOS=... RC_FRAMES=1200 [RC_LIST=904,1146] [RC_SUBLIST=5e8] cargo test --release -p segacd --test bios -- --ignored --nocapture`

use genesis::Region;
use genesis::mc68000::disassemble::disassemble;
use machine::Machine;
use segacd::{SegaCdConfig, SegaCdExp, new_machine};
use std::collections::BTreeMap;

fn listing(read: impl Fn(u32) -> u8, pc: u32, lines: usize) -> String {
    let mut out = String::new();
    let mut at = pc.saturating_sub(24) & !1;
    for _ in 0..lines {
        let raw: Vec<u8> = (0..10).map(|i| read(at + i)).collect();
        let (text, len) = disassemble(&raw, at);
        let mark = if at == pc { ">" } else { " " };
        out.push_str(&format!("{mark} {at:06X}  {text}\n"));
        at += len.clamp(2, 10) as u32;
    }
    out
}

fn top(hist: BTreeMap<u32, u32>, n: usize) -> Vec<(u32, u32)> {
    let mut v: Vec<_> = hist.into_iter().collect();
    v.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
    v.truncate(n);
    v
}

fn io_filter() -> (u32, u32) {
    let v = std::env::var("RC_IOFILTER").unwrap_or_else(|_| "FF804C-FF8067".into());
    let (a, b) = v.split_once('-').unwrap();
    (
        u32::from_str_radix(a, 16).unwrap(),
        u32::from_str_radix(b, 16).unwrap(),
    )
}

fn list_env(name: &str) -> Vec<u32> {
    std::env::var(name)
        .map(|v| {
            v.split(',')
                .filter_map(|a| u32::from_str_radix(a, 16).ok())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[ignore]
fn where_are_both_cpus() {
    let Ok(path) = std::env::var("RC_BIOS") else {
        return;
    };
    let frames: u64 = std::env::var("RC_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let bios = std::fs::read(path).unwrap();
    let mut config = SegaCdConfig::new(bios, Region::Usa);
    config.disc = std::env::var("RC_DISC").ok().map(Into::into);
    let mut m = new_machine(config).unwrap();
    let budget = frames * m.clock_hz() / m.frame_hz();
    let mut spent = 0u64;
    let stops = list_env("RC_TRACE");
    if !stops.is_empty() {
        m.bus_mut()
            .expansion_mut()
            .unwrap()
            .as_any_mut()
            .downcast_mut::<SegaCdExp>()
            .unwrap()
            .trace_sub(stops);
    }
    if std::env::var("RC_IOLOG").is_ok() {
        m.bus_mut()
            .expansion_mut()
            .unwrap()
            .as_any_mut()
            .downcast_mut::<SegaCdExp>()
            .unwrap()
            .log_main_io();
        m.bus_mut()
            .expansion_mut()
            .unwrap()
            .as_any_mut()
            .downcast_mut::<SegaCdExp>()
            .unwrap()
            .log_sub_io();
        let (lo, hi) = io_filter();
        m.bus_mut()
            .expansion_mut()
            .unwrap()
            .as_any_mut()
            .downcast_mut::<SegaCdExp>()
            .unwrap()
            .filter_sub_io(lo, hi);
        if let Ok(w) = std::env::var("RC_SUBWATCH") {
            let (a, b) = w.split_once('-').unwrap();
            m.bus_mut()
                .expansion_mut()
                .unwrap()
                .as_any_mut()
                .downcast_mut::<SegaCdExp>()
                .unwrap()
                .watch_sub_writes(
                    u32::from_str_radix(a, 16).unwrap(),
                    u32::from_str_radix(b, 16).unwrap(),
                );
        }
    }
    let (mut main_hist, mut sub_hist) = (BTreeMap::new(), BTreeMap::new());
    let main_watch = list_env("RC_MAINWATCH");
    let ram_watch = std::env::var("RC_RAMWATCH").ok().map(|v| {
        let (a, n) = v.split_once(',').unwrap();
        (
            u64::from_str_radix(a, 16).unwrap(),
            n.parse::<u64>().unwrap(),
        )
    });
    let mut last_frame = u64::MAX;
    let mut last_ram: Vec<u8> = Vec::new();
    let mut watched: BTreeMap<u32, u32> = BTreeMap::new();
    // RC_PRESS=900:Start:1,910:Start:0 presses and releases pad 1 buttons at the given frames
    let mut presses: Vec<(u64, String, bool)> = std::env::var("RC_PRESS")
        .map(|v| {
            v.split(',')
                .filter_map(|e| {
                    let mut p = e.split(':');
                    Some((
                        p.next()?.parse().ok()?,
                        p.next()?.to_string(),
                        p.next()? == "1",
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    presses.reverse();
    while spent < budget {
        let now_frame = spent / (m.clock_hz() / m.frame_hz());
        while presses.last().is_some_and(|p| p.0 <= now_frame) {
            let (_, name, down) = presses.pop().unwrap();
            m.set_pad_button(0, &name, down).unwrap();
        }
        spent += u64::from(m.step());
        if let Some(range) = &ram_watch {
            let frame = spent / (m.clock_hz() / m.frame_hz());
            if frame != last_frame {
                last_frame = frame;
                let now: Vec<u8> = (range.0..range.0 + range.1)
                    .map(|a| m.read_memory(0xFF0000 + a))
                    .collect();
                if now != last_ram {
                    println!("  ram frame {frame}: {now:02X?}");
                    last_ram = now;
                }
            }
        }
        if main_watch.contains(&(m.pc() as u32)) {
            *watched.entry(m.pc() as u32).or_default() += 1;
        }
        if spent > budget - 3_000_000 {
            *main_hist.entry(m.pc() as u32).or_default() += 1;
            let e = m
                .bus()
                .expansion()
                .unwrap()
                .as_any()
                .downcast_ref::<SegaCdExp>()
                .unwrap();
            *sub_hist.entry(e.sub_cpu().pc).or_default() += 1;
        }
    }
    println!("main watch hits {watched:X?}");
    if let Ok(path) = std::env::var("RC_DUMP_RAM") {
        std::fs::write(path, m.bus().ram()).unwrap();
    }
    println!("stop: {:?}", m.take_stop_report());
    {
        let regs = m.registers();
        let a7 = regs.iter().find(|(n, _)| *n == "A7").map_or(0, |(_, v)| *v) as u32;
        let stack: Vec<u32> = (0..12)
            .map(|i| {
                let at = u64::from(a7) + 4 * i;
                (0..4).fold(0u32, |w, k| w << 8 | u32::from(m.read_memory(at + k)))
            })
            .collect();
        println!("main a7 {a7:06X} stack {stack:06X?}");
    }
    println!("main pc {:06X}, hot {:X?}", m.pc(), top(main_hist, 8));
    let pc = m.pc() as u32;
    println!("{}", listing(|a| m.read_memory(u64::from(a)), pc, 14));
    for at in list_env("RC_LIST") {
        println!(
            "--- main listing {at:X}\n{}",
            listing(|x| m.read_memory(u64::from(x)), at + 24, 36)
        );
    }
    let e = m
        .bus()
        .expansion()
        .unwrap()
        .as_any()
        .downcast_ref::<SegaCdExp>()
        .unwrap();
    let sub_pc = e.sub_cpu().pc;
    println!(
        "sub pc {sub_pc:06X} sr {:04X} hung {} hot {:X?}",
        e.sub_cpu().sr(),
        e.sub_hung(),
        top(sub_hist, 10)
    );
    println!("{}", listing(|a| e.sub_peek(a), sub_pc, 14));
    for at in list_env("RC_SUBLIST") {
        println!(
            "--- sub listing {at:X}\n{}",
            listing(|x| e.sub_peek(x), at + 24, 36)
        );
    }
    if let Some((frozen, ring)) = e.sub_trace() {
        println!(
            "sub trace frozen {frozen}, last {} instructions:",
            ring.len()
        );
        for (pc, d, a, sr) in ring.iter().rev().take(40).rev() {
            println!("  {pc:06X} sr {sr:04X} d {d:X?} a {a:X?}");
        }
    }
    if let Ok(path) = std::env::var("RC_DUMP_PRG") {
        std::fs::write(path, e.prg_ram()).unwrap();
    }
    for (pc, addr, size, value) in e
        .sub_io_log()
        .iter()
        .filter(|(_, a, _, _)| {
            let (lo, hi) = io_filter();
            (lo..=hi).contains(&(a & 0xFF_FFFF))
        })
        .skip(
            std::env::var("RC_IOSKIP")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
        )
        .take(
            std::env::var("RC_IOMAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(400),
        )
    {
        let kind = if addr & 0x8000_0000 != 0 { "R" } else { "W" };
        println!(
            "  io pc {pc:06X} {kind} {:06X} {size:?} {value:X}",
            addr & 0xFF_FFFF
        );
    }
    if std::env::var("RC_MAINIO").is_ok() {
        for (t, addr, size, value) in e.main_io_log().iter().take(3000) {
            println!("  main io t {t} {addr:06X} {size:?} {value:X}");
        }
    }
    if std::env::var("RC_CDDLOG").is_ok() {
        let mut last = None;
        for (i, (tc, rs)) in e.gate_array().cdd_log().iter().enumerate() {
            let line = if std::env::var("RC_CDDLOG").unwrap() == "short" {
                format!(
                    "cmd {:X}{:X}{:X}{:X} -> {:X}{:X} {:X?}",
                    tc[0],
                    tc[3],
                    tc[4],
                    tc[5],
                    rs[0],
                    rs[1],
                    &rs[2..9]
                )
            } else {
                format!("cmd {tc:X?} -> {rs:X?}")
            };
            if last.as_deref() != Some(line.as_str()) {
                println!("  cdd #{i}: {line}");
                last = Some(line);
            }
        }
    }
    println!("asic trace {:04X?}", e.asic_trace());
    {
        let w = e.word_ram();
        let entry = |row: usize, col: usize| {
            let at = 0x20000 + (row * 256 + col) * 2;
            u16::from_be_bytes([w[at], w[at + 1]])
        };
        for row in 6..11 {
            println!(
                "map row {row}: {:04X?}",
                (0..22).map(|c| entry(row, c)).collect::<Vec<_>>()
            );
        }
    }
    println!(
        "sub starts {} ran {} of {}",
        e.sub_starts(),
        e.sub_clocks_run(),
        e.sub_clocks_due()
    );
    println!("sub vectors 60..80 {:02X?}", &e.prg_ram()[0x60..0x80]);
    println!("sub 5f30..5f80 {:02X?}", &e.prg_ram()[0x5F30..0x5FA8]);
    println!("sub d {:X?} a {:X?}", e.sub_cpu().d, e.sub_cpu().a);
    let mut g = e.gate_array().clone();
    println!(
        "a12000 {:04X?} halted {} due {}",
        g.main_read_word(0),
        g.sub_halted(),
        e.sub_clocks_due()
    );
    println!(
        "sub regs: mask {:?} cdd ctrl {:?} timer {:?} irq {:?} rs {:?} tc {:?}",
        g.sub_read_word(0x32),
        g.sub_read_word(0x36),
        g.sub_read_word(0x30),
        g.sub_irq_level(),
        (0..5)
            .map(|i| g.sub_read_word(0x38 + 2 * i).unwrap())
            .collect::<Vec<_>>(),
        (0..5)
            .map(|i| g.sub_read_word(0x42 + 2 * i).unwrap())
            .collect::<Vec<_>>()
    );
    println!(
        "rs bytes {:X?}",
        (0..10)
            .map(|i| g.sub_read_byte(0x38 + i).unwrap())
            .collect::<Vec<_>>()
    );
    let mut words = Vec::new();
    for off in (0x0E..=0x0E)
        .chain((0x10..=0x1E).step_by(2))
        .chain((0x20..=0x2E).step_by(2))
    {
        words.push(g.main_read_word(off).unwrap());
    }
    println!("main flag, commands, status: {words:04X?}");
    println!(
        "audio activity (non-silent PCM samples, CD audio frames): {:?}",
        e.audio_activity()
    );
}

/// A real BIOS and disc (`RC_BIOS`, `RC_DISC`): run, save, load into a fresh machine, run both on: the bytes of their states and
/// the pictures stay equal. `RC_FRAMES` (default 400) frames before the save and 120 after.
#[test]
#[ignore]
fn a_saved_state_resumes_the_real_bios_and_disc_exactly() {
    let (Ok(bios), Ok(disc)) = (std::env::var("RC_BIOS"), std::env::var("RC_DISC")) else {
        return;
    };
    let frames: u64 = std::env::var("RC_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    let make = || {
        let mut config = SegaCdConfig::new(std::fs::read(&bios).unwrap(), Region::Usa);
        config.disc = Some(disc.clone().into());
        new_machine(config).unwrap()
    };
    let run = |m: &mut genesis::Genesis, frames: u64| {
        let per = m.clock_hz() / m.frame_hz();
        let mut spent = 0u64;
        while spent < frames * per {
            spent += u64::from(m.step());
        }
    };
    let mut a = make();
    run(&mut a, frames);
    let state = a.save_state();
    println!("state: {} bytes", state.len());
    let mut b = make();
    b.load_state(&state).unwrap();
    run(&mut a, 120);
    run(&mut b, 120);
    assert_eq!(a.save_state(), b.save_state(), "the states diverged");
    assert_eq!(a.render_frame(), b.render_frame(), "the pictures differ");
}

/// Where the game's software error check starts (`RC_STOP`, hex, default 4E74): the registers there and the bytes the first
/// register points at.
#[test]
#[ignore]
fn registers_where_the_sub_cpu_reaches_an_address() {
    let (Ok(bios), Ok(disc)) = (std::env::var("RC_BIOS"), std::env::var("RC_DISC")) else {
        return;
    };
    let stops: Vec<u32> = std::env::var("RC_STOP")
        .unwrap_or_else(|_| "4E74".into())
        .split(',')
        .map(|v| u32::from_str_radix(v, 16).unwrap())
        .collect();
    let mut config = SegaCdConfig::new(std::fs::read(bios).unwrap(), Region::Usa);
    config.disc = Some(disc.into());
    let mut m = new_machine(config).unwrap();
    m.bus_mut()
        .expansion_mut()
        .unwrap()
        .as_any_mut()
        .downcast_mut::<SegaCdExp>()
        .unwrap()
        .trace_sub(stops);
    let mut spent = 0u64;
    let mut steps = 0u64;
    let budget = 1500 * (m.clock_hz() / m.frame_hz());
    while spent < budget {
        spent += u64::from(m.step());
        steps += 1;
        if !steps.is_multiple_of(2000) {
            continue;
        }
        let e = m
            .bus()
            .expansion()
            .unwrap()
            .as_any()
            .downcast_ref::<SegaCdExp>()
            .unwrap();
        if let Some((true, ring)) = e.sub_trace() {
            let (pc, d, a, sr) = ring.last().unwrap();
            println!("frozen at {pc:06X} sr {sr:04X}\n d {d:08X?}\n a {a:08X?}");
            let at = a[0];
            let bytes: Vec<u8> = (0..96).map(|i| e.sub_peek(at + i)).collect();
            println!("a0 {at:06X}: {bytes:02X?}");
            // the way in: the last instructions before the stop, runs of consecutive addresses written as ranges
            let mut path = String::new();
            let tail: Vec<u32> = ring
                .iter()
                .rev()
                .take(150)
                .map(|e| e.0)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let mut i = 0;
            while i < tail.len() {
                let mut j = i;
                while j + 1 < tail.len() && tail[j + 1] > tail[j] && tail[j + 1] - tail[j] <= 8 {
                    j += 1;
                }
                path.push_str(&format!("{:X}-{:X} ", tail[i], tail[j]));
                i = j + 1;
            }
            println!("path: {path}");
            return;
        }
    }
    println!("never reached");
}

/// Linear disassembly of the sub-CPU's BIOS (`RC_BIOS`) printing the lines that contain `RC_GREP` (default `$2(a6)`).
#[test]
#[ignore]
fn grep_the_bios_listing() {
    let Ok(path) = std::env::var("RC_BIOS") else {
        return;
    };
    let mut bios = std::fs::read(&path).unwrap();
    if let Ok(disc) = std::env::var("RC_LIVE_DISC") {
        // the sub-CPU's code as it is in program RAM after RC_FRAMES frames
        let frames: u64 = std::env::var("RC_FRAMES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(900);
        let mut config = SegaCdConfig::new(bios, Region::Usa);
        config.disc = Some(disc.into());
        let mut m = new_machine(config).unwrap();
        let mut spent = 0u64;
        let budget = frames * (m.clock_hz() / m.frame_hz());
        while spent < budget {
            spent += u64::from(m.step());
        }
        let e = m
            .bus()
            .expansion()
            .unwrap()
            .as_any()
            .downcast_ref::<SegaCdExp>()
            .unwrap();
        bios = e.prg_ram()[..0x2_0000].to_vec();
    }
    let pattern = std::env::var("RC_GREP").unwrap_or_else(|_| "$2(a6)".into());
    let mut at = 0usize;
    while at + 10 < bios.len() {
        let (text, len) = disassemble(&bios[at..at + 10], at as u32);
        let (from, to) = std::env::var("RC_RANGE")
            .ok()
            .and_then(|v| {
                let (a, b) = v.split_once('-')?;
                Some((
                    usize::from_str_radix(a, 16).ok()?,
                    usize::from_str_radix(b, 16).ok()?,
                ))
            })
            .unwrap_or((0, usize::MAX));
        if (from..=to).contains(&at) && text.contains(&pattern) {
            println!("  {at:06X}  {text}");
        }
        at += len.clamp(2, 10);
    }
}

/// The distinct sub-CPU addresses the last 20 000 instructions touched (`RC_STOP` stops that never hit keep the ring running):
/// shows which routines are alive in a steady state, in the order of their first appearance, with interrupt entries marked.
#[test]
#[ignore]
fn which_routines_are_alive() {
    let (Ok(bios), Ok(disc)) = (std::env::var("RC_BIOS"), std::env::var("RC_DISC")) else {
        return;
    };
    let frames: u64 = std::env::var("RC_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(900);
    let mut config = SegaCdConfig::new(std::fs::read(bios).unwrap(), Region::Usa);
    config.disc = Some(disc.into());
    let mut m = new_machine(config).unwrap();
    m.bus_mut()
        .expansion_mut()
        .unwrap()
        .as_any_mut()
        .downcast_mut::<SegaCdExp>()
        .unwrap()
        .trace_sub(vec![0xFFFF_FFF0]);
    let mut spent = 0u64;
    let budget = frames * (m.clock_hz() / m.frame_hz());
    while spent < budget {
        spent += u64::from(m.step());
    }
    let e = m
        .bus()
        .expansion()
        .unwrap()
        .as_any()
        .downcast_ref::<SegaCdExp>()
        .unwrap();
    let (_, ring) = e.sub_trace().unwrap();
    println!("{} instructions in the ring", ring.len());
    let mut seen = std::collections::BTreeSet::new();
    let mut out = String::new();
    let mut prev_sr = 0u16;
    for (pc, _, _, sr) in &ring {
        if (sr & 0x0700) != (prev_sr & 0x0700) {
            out.push_str(&format!(
                "
[sr {sr:04X}] "
            ));
        }
        prev_sr = *sr;
        if seen.insert(*pc) {
            out.push_str(&format!("{pc:X} "));
        }
    }
    println!("{out}");
}

/// The main CPU's view at a stall: its vectors (work RAM `$FFFF0060..`), status register and the gate array registers it can read.
#[test]
#[ignore]
fn main_side_at_the_stall() {
    let (Ok(bios), Ok(disc)) = (std::env::var("RC_BIOS"), std::env::var("RC_DISC")) else {
        return;
    };
    let frames: u64 = std::env::var("RC_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(900);
    let mut config = SegaCdConfig::new(std::fs::read(bios).unwrap(), Region::Usa);
    config.disc = Some(disc.into());
    let mut m = new_machine(config).unwrap();
    let mut spent = 0u64;
    let budget = frames * (m.clock_hz() / m.frame_hz());
    while spent < budget {
        spent += u64::from(m.step());
    }
    let word = |m: &genesis::Genesis, a: u32| {
        (u32::from(m.read_memory(u64::from(a))) << 8) | u32::from(m.read_memory(u64::from(a + 1)))
    };
    let long = |m: &genesis::Genesis, a: u32| (word(m, a) << 16) | word(m, a + 2);
    println!("main sr {:04X}", m.cpu().sr());
    for (name, at) in [
        ("lvl1", 0x64u32),
        ("lvl2", 0x68),
        ("lvl3", 0x6C),
        ("lvl4", 0x70),
        ("lvl6", 0x78),
    ] {
        println!("vector {name} at {at:X}: {:08X}", long(&m, at));
    }
    let e = m
        .bus()
        .expansion()
        .unwrap()
        .as_any()
        .downcast_ref::<SegaCdExp>()
        .unwrap();
    let mut gate = e.gate_array().clone();
    for off in (0u32..0x40).step_by(2) {
        print!(
            "{:02X}:{:04X} ",
            off,
            gate.main_read_word(off).unwrap_or(0xDEAD)
        );
    }
    println!();
    let dbg = format!("{:?}", e.gate_array());
    println!("gate: {}", dbg.split("cdc:").next().unwrap_or(&dbg));
    println!(
        "8e65: {:02X}, 8e66: {:02X}",
        m.read_memory(0xFF8E65u64),
        m.read_memory(0xFF8E66u64)
    );
}
