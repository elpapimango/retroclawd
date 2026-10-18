---
status: accepted
---

# Plugin ABI is widened once before 16-bit machines

The ABI and `TODO.md` guardrails assume 16-bit CPU addresses and one CPU per Machine (the fast-path `pc` returns `u16`; "CPU-visible addresses remain `u16`"). The 68000 has a 24-bit bus and the Atari ST, Amiga and Genesis add several CPUs, controllers and cartridge-only media. The decision is to audit `plugin-api` for these needs and bump it once, before the first such Machine, so that no machine-adding run has to stop on an ABI change. The audit and the concrete design are in `docs/superpowers/specs/2026-10-18-abi-16bit-readiness-design.md`: `u64` addresses at the Machine/protocol boundary with per-Core buses inside, a per-CPU debug surface, stereo audio, rational frame rate, pixel aspect and analogue axes. Implemented 2026-10-18 (protocol 11, ABI minor 2, profile contract 2); what the Host does not yet consume is listed in the spec's Status section.
