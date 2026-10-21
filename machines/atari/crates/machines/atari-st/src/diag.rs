//! Bounded bring-up diagnostics (spec section 22): the last instructions, the last exceptions, the PC trace and
//! the report written when the machine stops at an unimplemented device. No file I/O here: the plugin decides
//! where the text goes. No memory or ROM contents are written besides one opcode word per history line.

use std::collections::VecDeque;
use std::fmt::Write as _;

use mc68000::{Cpu, ExceptionRecord, FaultInfo, Size};

use crate::bus::{IoStop, StopReason};

const HISTORY: usize = 256;
const EXCEPTIONS: usize = 64;

#[derive(Debug, Default)]
pub struct Diag {
    history: VecDeque<(u32, u16)>,
    exceptions: VecDeque<ExceptionRecord>,
    trace: Vec<u32>,
    trace_limit: u32,
    reported: bool,
}

impl Diag {
    pub fn new(trace_limit: u32) -> Self {
        Self {
            trace_limit,
            ..Self::default()
        }
    }

    /// The instruction about to execute at `pc` (opcode `op`).
    pub fn instruction(&mut self, pc: u32, op: u16) {
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back((pc, op));
        if (self.trace.len() as u32) < self.trace_limit {
            self.trace.push(pc);
        }
    }

    pub fn exception(&mut self, record: ExceptionRecord) {
        if self.exceptions.len() == EXCEPTIONS {
            self.exceptions.pop_front();
        }
        self.exceptions.push_back(record);
    }

    /// The PC of the last instruction recorded.
    pub fn last_pc(&self) -> Option<u32> {
        self.history.back().map(|(pc, _)| *pc)
    }

    pub fn trace(&self) -> &[u32] {
        &self.trace
    }

    pub fn trace_full(&self) -> bool {
        self.trace_limit != 0 && self.trace.len() as u32 >= self.trace_limit
    }

    /// Whether the report has been given already (checked before any of its work is done).
    pub fn reported(&self) -> bool {
        self.reported
    }

    /// The report headed by `headline`, the first time it is asked for.
    pub fn take_report(&mut self, cpu: &Cpu, headline: &str, disassembly: &str) -> Option<String> {
        if self.reported {
            return None;
        }
        self.reported = true;
        Some(self.report(cpu, headline, disassembly))
    }

    /// The first line for an unemulated device.
    pub fn io_headline(stop: &IoStop, clock_hz: u64) -> String {
        let size = if stop.size == Size::Byte {
            "byte"
        } else {
            "word"
        };
        let direction = if stop.write { "write" } else { "read" };
        let ms = stop.at_cycle as f64 * 1000.0 / clock_hz.max(1) as f64;
        let value = if stop.write {
            format!(" = ${:X}", stop.value)
        } else {
            String::new()
        };
        let why = match stop.reason {
            StopReason::NotEmulated(d) => format!("{} is not emulated yet", d.name()),
            StopReason::NoFact => format!(
                "no hardware fact for ${:06X} ({size} {direction})",
                stop.addr
            ),
        };
        format!(
            "stop: {direction} {size} ${:06X}{value} (fc {}): {why}; cycle {} ({ms:.3} ms)",
            stop.addr, stop.fc.0, stop.at_cycle
        )
    }

    /// The first line for a halted CPU (UM 6.3.9.1: a bus or address error while processing one, or while reading
    /// the reset vectors).
    pub fn halt_headline(fault: Option<FaultInfo>, cycle: u64, clock_hz: u64) -> String {
        let ms = cycle as f64 * 1000.0 / clock_hz.max(1) as f64;
        let cause = match fault {
            Some(f) => format!(
                "fault on the {} of ${:08X} (fc {})",
                if f.write { "write" } else { "read" },
                f.addr,
                f.fc.0
            ),
            None => "cause unknown".to_string(),
        };
        format!(
            "cpu halted (double bus fault or a fault reading the reset vectors): {cause}; cycle {cycle} ({ms:.3} ms)"
        )
    }

    fn report(&self, cpu: &Cpu, headline: &str, disassembly: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{headline}");
        let regs = |prefix: char, values: &[u32]| {
            values
                .iter()
                .enumerate()
                .map(|(i, v)| format!("{prefix}{i}={v:08X}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let sr = cpu.sr();
        let flag = |bit: u16, name: char| if sr & bit != 0 { name } else { '-' };
        let _ = writeln!(out, "cpu: {}", regs('D', &cpu.d));
        let _ = writeln!(out, "     {} A7={:08X}", regs('A', &cpu.a), cpu.a7());
        let _ = writeln!(
            out,
            "     PC=${:06X} SR=${sr:04X} ({}{} I={} {}{}{}{}{}) USP={:08X} SSP={:08X} {} mode",
            cpu.pc,
            flag(0x8000, 'T'),
            flag(0x2000, 'S'),
            (sr >> 8) & 7,
            flag(0x10, 'X'),
            flag(0x08, 'N'),
            flag(0x04, 'Z'),
            flag(0x02, 'V'),
            flag(0x01, 'C'),
            cpu.usp,
            cpu.ssp,
            if sr & 0x2000 != 0 {
                "supervisor"
            } else {
                "user"
            },
        );
        let _ = writeln!(out, "instruction: {disassembly}");
        let _ = writeln!(out, "history (last {}, oldest first):", self.history.len());
        for (pc, op) in &self.history {
            let _ = writeln!(out, "  ${pc:06X} {op:04X}");
        }
        let _ = writeln!(out, "exceptions (last {}):", self.exceptions.len());
        for e in &self.exceptions {
            let _ = write!(
                out,
                "  vector {} stacked pc ${:06X} sr ${:04X}",
                e.vector, e.pc, e.sr
            );
            if let Some(f) = e.fault {
                let _ = write!(
                    out,
                    " fault {} ${:08X} fc {}",
                    if f.write { "write" } else { "read" },
                    f.addr,
                    f.fc.0
                );
            }
            let _ = writeln!(out);
        }
        out
    }
}
