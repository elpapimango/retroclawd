//! Bring-up probe: `ROM=game.pce FRAMES=120 OUT=frame.ppm cargo run --release -p pcengine --example probe`.
//! Prints the CPU and the video registers and writes the picture as a PPM; `RC_TRACE=n` also prints the first n
//! instructions.
use pcengine::{Console, FRAME_CLOCKS, PcEngine};

fn main() {
    let rom = std::fs::read(std::env::var("ROM").expect("ROM")).unwrap();
    let frames: u64 = std::env::var("FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let mut e = PcEngine::from_card(rom, Console::PcEngine).unwrap();
    let trace: usize = std::env::var("RC_TRACE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    for i in 0..trace {
        let pc = e.cpu.pc;
        let bytes: Vec<u8> = (0..7).map(|k| e.peek_logical(pc.wrapping_add(k))).collect();
        let (text, _) = huc6280::disassemble::disassemble(&bytes, pc);
        println!(
            "{i:6} {pc:04X} {text:28} A={:02X} X={:02X} Y={:02X} S={:02X} P={:02X}",
            e.cpu.a, e.cpu.x, e.cpu.y, e.cpu.s, e.cpu.p
        );
        e.step();
    }
    let mut clocks = 0u64;
    let mut audio: Vec<f32> = Vec::new();
    // PRESS=frame:Key,frame:Key... holds a key from that frame on (prefix `-` to release)
    let presses: Vec<(u64, String)> = std::env::var("PRESS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|p| {
            p.split_once(':')
                .map(|(f, k)| (f.parse().unwrap_or(0), k.to_string()))
        })
        .collect();
    while clocks < frames * u64::from(FRAME_CLOCKS) {
        for (f, k) in &presses {
            if clocks / u64::from(FRAME_CLOCKS) == *f && clocks % u64::from(FRAME_CLOCKS) < 400 {
                match k.strip_prefix('-') {
                    Some(key) => e.set_key(key, false),
                    None => e.set_key(k, true),
                };
            }
        }
        clocks += u64::from(e.step());
        if clocks % 4096 < 16 {
            audio.extend(e.drain_audio());
        }
    }
    audio.extend(e.drain_audio());
    if let Ok(out) = std::env::var("AUDIOOUT") {
        let mut w = Vec::new();
        let data: Vec<u8> = audio
            .iter()
            .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
            .collect();
        w.extend(b"RIFF");
        w.extend((36 + data.len() as u32).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes());
        w.extend(2u16.to_le_bytes());
        w.extend(44_100u32.to_le_bytes());
        w.extend((44_100u32 * 4).to_le_bytes());
        w.extend(4u16.to_le_bytes());
        w.extend(16u16.to_le_bytes());
        w.extend(b"data");
        w.extend((data.len() as u32).to_le_bytes());
        w.extend(data);
        std::fs::write(out, w).unwrap();
    }
    let tail: usize = std::env::var("RC_TRACE_END")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    for i in 0..tail {
        let pc = e.cpu.pc;
        let bytes: Vec<u8> = (0..7).map(|k| e.peek_logical(pc.wrapping_add(k))).collect();
        let (text, _) = huc6280::disassemble::disassemble(&bytes, pc);
        println!(
            "end{i:4} {pc:04X} {text:28} A={:02X} X={:02X} Y={:02X} P={:02X} line={}",
            e.cpu.a,
            e.cpu.x,
            e.cpu.y,
            e.cpu.p,
            e.line()
        );
        e.step();
    }
    let c = &e.cpu;
    println!(
        "PC={:04X} A={:02X} X={:02X} Y={:02X} S={:02X} P={:02X} fast={} MPR={:02X?}",
        c.pc, c.a, c.x, c.y, c.s, c.p, c.fast, c.mpr
    );
    let v = e.vdc();
    for r in 0..0x14 {
        print!("R{r:02X}={:04X} ", v.register(r));
    }
    println!();
    println!(
        "VCE ctrl dot={} lines={} pal0={:03X} pal1={:03X} pal16={:03X}",
        e.vce().dot_clock(),
        e.vce().lines(),
        e.vce().entry(0),
        e.vce().entry(1),
        e.vce().entry(0x11)
    );
    for i in 0..8 {
        let w = &v.satb()[i * 4..i * 4 + 4];
        println!(
            "SAT{i}: y={:03X} x={:03X} pat={:04X} attr={:04X}",
            w[0], w[1], w[2], w[3]
        );
    }
    println!(
        "timer reload/enabled: {:?}  irq mask/timer-pending: {:?}",
        e.timer(),
        e.irq_state()
    );
    let pal_nz = (0..512u16).filter(|i| e.vce().entry(*i) != 0).count();
    println!(
        "palette nonzero entries: {pal_nz}; first: {:03X?}",
        (0..32u16).map(|i| e.vce().entry(i)).collect::<Vec<_>>()
    );
    if let Ok(a) = std::env::var("VRAMAT") {
        let a = usize::from_str_radix(&a, 16).unwrap();
        for r in 0..8 {
            let w: Vec<String> = (0..8)
                .map(|i| format!("{:04X}", v.vram()[(a + r * 8 + i) & 0x7FFF]))
                .collect();
            println!("VRAM {:04X}: {}", a + r * 8, w.join(" "));
        }
    }
    {
        // runs of non-zero VRAM words, 256-word blocks
        let mut line = String::new();
        for blk in 0..128 {
            let n = v.vram()[blk * 256..blk * 256 + 256]
                .iter()
                .filter(|w| **w != 0)
                .count();
            line.push(if n == 0 {
                '.'
            } else if n < 64 {
                'o'
            } else {
                '#'
            });
        }
        println!("VRAMMAP {line}");
    }
    let nz = v.vram().iter().filter(|w| **w != 0).count();
    println!("VRAM nonzero words: {nz}; frame {}", v.frame);
    if let Ok(out) = std::env::var("RAMOUT") {
        let ram: Vec<u8> = (0..0x2000u32).map(|a| e.peek((0xF8 << 13) | a)).collect();
        std::fs::write(out, ram).unwrap();
    }
    if let (Ok(out), Some(bram)) = (std::env::var("BRAMOUT"), e.backup_ram()) {
        std::fs::write(out, bram).unwrap();
    }
    if let Ok(out) = std::env::var("OUT") {
        let (w, h, rgb) = e.render_rgb();
        let mut f = format!("P6\n{w} {h}\n255\n").into_bytes();
        f.extend(rgb);
        std::fs::write(out, f).unwrap();
    }
}
