---
name: add-machine
description: Use when asked to add a new emulated machine, computer or console to retroclawd (for example "add machine msx", "add the Atari ST"). Runs the new-machine playbook start to finish without asking questions.
---

# Add a machine

Read `docs/new-machine-playbook.md` and `GLOSSARY.md` first. Use the glossary's terms in code, docs and the report.

Run phases P0 to P6 in order. Do not ask the user questions during the run:

- Where the playbook gives a default, take it and write the assumption in the final report.
- Where something blocks you (reference looks wrong, file cannot be fetched, output differs for an unclear reason), record a known gap in the Machine's manifest with evidence, skip that scenario and continue.
- Stop only if the Machine cannot boot at all, and say so.

Start by creating `machines/<family>/<id>.intake.toml` from primary sources. If the ABI readiness pass in `docs/adr/0003-address-width-and-multi-cpu.md` is not done and the Machine needs it, stop and say so before starting.

Never weaken a golden or tolerance to pass a scenario. Finish with the report: scenario results, known gaps, assumptions.
