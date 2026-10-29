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
        .take(400)
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
    println!("sub 5f30..5f80 {:02X?}", &e.prg_ram()[0x5F30..0x5F80]);
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
