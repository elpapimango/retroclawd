# Prompt: build `retroclawd-tools`

> **Status (2026-10-13):** all six tools are built and pushed (<https://github.com/elpapimango/retroclawd-tools>): `rcasm`,
> `rcdis`, `rcdisk` (CBM and CP/M images), `rctape` (TZX/TAP, `.t64`), `rcsnap` (snapshots, cartridges) and `rcld`
> (linker / packager).
> Two decisions below changed: the repository is **standalone** (not a fifth submodule of the
> application; it is not part of `build_all`, `smoke_test`, `bump_pin` or this repository's CI), and the order is
> `rcasm`/`rcdis` first, then `rcdisk`/`rctape`, `rcsnap`, `rcld`. CI secret: `FAMILY_REPOS_TOKEN` in the tools
> repository. The notes below are the original brief.

Use this as the opening brief for a session that builds the tools repository. It is self-contained; read
`CLAUDE.md` and `README.md` of this repository first, then the `CLAUDE.md` of `shared/` and the machine
repositories for the details of the crates you will use.

## Goal

Create **`elpapimango/retroclawd-tools`** (private, same account; the owner creates the empty repository, the
session attaches it with `add_repo`): command-line tools built on the retroclawd crates to **open, create, read,
write and convert tape and disk images**, and to **disassemble, assemble and link programs that run on our
machines** (ZX81, ZX Spectrum, Amstrad CPC, C64, C128). Rust, cross-platform (Linux, macOS, Windows), no machine
emulation inside: these are file/format and code tools. The emulator application stays as it is.

## Decisions already made (do not re-ask)

- **Repository model:** one tools repository that depends on **all four existing repositories by git revision**,
  crates only (never the machine plugin libraries): `retroclawd-crates` (shared: `machine`, `plugin-api`, `z80`,
  `mos6502`, `ay-3-8910`, `nec765`, `dsk`, `zx-tape`), `retroclawd-sinclair` (`z80-snapshot`, `sna-snapshot`,
  `szx-snapshot`, `zx81-snapshot`), `retroclawd-amstrad` (`cpc-cartridge`, `cpc-snapshot`) and
  `retroclawd-commodore` (`d64`, `d71`, `d81`, `g64`, `cbm-dos`, `c64-tape`, `c64-cartridge`). (Original plan: a fifth submodule of the
  application repository wired into `bump_pin`/`build_all`/`smoke_test`; **changed**: it builds by itself and the
  application repository does not know it.)
  If a format crate lacks something a tool needs, **add it to that crate in its own repository** (and bump the pin),
  not as a private copy in the tools repository.
- **Binaries, `rc` prefix, separate executables:** `rcdisk` (disk images), `rctape` (tape images), `rcsnap` (snapshots
  and cartridges: inspect/convert; add it if it does not fit `rcdisk`/`rctape`), `rcasm` (assembler), `rcdis`
  (disassembler), `rcld` (linker/packager). Each is its own binary crate; common code (argument conventions, error
  reporting, format sniffing, file I/O incl. the zip/URL conventions of the app's `file-loader` where useful) lives in
  a small library crate. Also expose the functionality as library crates so other programs can reuse it.
- **Order of work:** `rcasm`/`rcdis` first (done), then the media tools (`rcdisk`, `rctape`, `rcsnap`), then `rcld`.
- **Media priorities for the first release (all four):** (1) list/inspect/validate, (2) extract and add files
  (also delete/rename), (3) create blank/formatted images, (4) convert between formats.
- **Assembler:** a real **two-pass assembler without object files**, for **6502 and Z80 from the start**, one shared
  syntax: `label:` and local labels, `.org .byte .word .text .fill .incbin .include .equ .macro/.endm .if/.else/.endif`,
  expressions with symbols, `*`/`$` for the current address, `;` comments, standard mnemonics per CPU (6502 incl.
  the undocumented opcodes behind a flag, using the `*`-free spelling plus a `--illegal` option; Z80 incl. IXH/IXL/IYH/IYL).
  It writes a flat binary per segment, a **symbol/map file**, and an optional listing.
- **Linker (`rcld`):** no relocatable objects. It is a **packager/linker for images**: takes the assembler's
  segments (each with a load address) and symbol files plus a small layout/memory-map file, places and checks them
  (overlaps, region limits), resolves cross-segment addresses via the symbol files, and emits a runnable result:
  `.prg`, `.tap`/`.tzx`/`.cdt`, `.d64`/`.d81`/`.dsk`, `.sna`/`.z80`, `.crt`/`.cpr`, optionally with a BASIC loader stub
  and the machine's standard loader header.
- **Disassembler (`rcdis`):** uses `z80::disassemble` / `mos6502::disassemble` (extend them, they are the single
  source of truth the debugger also uses); reads raw binaries, `.prg`, tape/disk files, snapshots and memory dumps;
  linear and entry-point-following modes, symbol files in, assembler-compatible source out so a round trip
  (disassemble -> assemble) reproduces the bytes.
- Interface conventions: subcommands per tool (`rcdisk list|info|validate|extract|add|rm|rename|create|convert ...`),
  `--help` everywhere, machine-readable output option (`--json`) for list/info, non-zero exit on error, never
  overwrite an existing file without `--force`, all file arguments accept the zip/URL forms of the app where it is cheap.

## Reference tool: tzxtools

[`tzxtools`](https://pypi.org/project/tzxtools/) (source: <https://github.com/shred/tzxtools>, Python, **GPL-3.0**)
is the popular ZX Spectrum `.tzx` toolbox; use it to see how a mature tool is organised and which operations
people actually use. Its commands: `tzxls` (show contents), `tzxcat` (extract data; hexdump, disassemble, export
screens as PNG), `tzxcleanup` (strip filler blocks, tidy), `tzxcut` (remove selected blocks), `tzxmerge` (join
files), `tzxsplit` (one file per program), `tzxtap` (TZX -> TAP), `tzxwav` (WAV recording -> TZX) and `tzxplay`
(play to a real machine). Map them onto `rctape` as subcommands (`ls`, `cat`/`extract` with `--hex`/`--disasm`/`--png`,
`cleanup`, `cut`, `merge`, `split`, `convert`, `import-wav`/`export-wav`, and optionally `play` to a sound device)
and generalise them across our machines (CPC `.cdt`, C64 `.tap`) -- tzxtools is ZX-only. Compare behaviours and output
conventions, and test our results against its on the same tapes. **Do not copy its code** (GPL-3.0): implement from the
format specifications, cite tzxtools in `CREDITS.md` as a behavioural reference, and keep it a reference rather than a
dependency. Other tools worth surveying the same way: for disks, VICE's `c1541`; for assembly, `ca65`/`acme`/`64tass`
(6502) and `sjasmplus`/`pasmo` (Z80).

## What to do first (survey, then plan)

1. Survey the existing format crates' APIs and write down, per format, what exists (parse, write, create blank,
   mutate) and what is missing for the four media priorities. Report the gaps before writing tools.
2. Propose the crate layout of the tools repository, the shared command-line conventions, the assembler's
   internal design (lexer, expression evaluator, two-pass symbol resolution, per-CPU instruction tables built from
   the existing disassembler tables so there is one source of truth) and the layout-file format for `rcld`.
3. Agree the plan, then build in the order above, each tool with tests (round trips on real images from the
   family repositories' `software/` folders, malformed-input tests) and at least one end-to-end proof that a
   produced image **boots/loads in the emulator** (use `tools/smoke_test.sh`-style headless runs).

## Questions to ask the owner before building (not decided yet)

- Exact set of file formats per machine for the first release, and which conversions are mandatory (e.g. `.tap`
  <-> WAV, `.d64` <-> `.g64`, `.dsk` <-> `.edsk`, `.prg` <-> `.t64` <-> `.tap`).
- Whether tapes should be created in real ROM format only or also with the common turbo/fast loaders.
- Naming of the layout file and symbol/map file formats (propose first); whether to be compatible with existing
  ones (VICE monitor symbol files, `ld65` map, `sjasmplus` labels) for import/export.
- Whether the assembler's CPU tables should also feed the debugger's `assemble_one` (replacing it) so the debug UI
  gets labels and full syntax.
- Release/packaging of the six binaries (archives per platform, the same `plugins/`-style layout, CI artifacts).

## Constraints

- Follow the repository conventions: `CLAUDE.md` per repository, `CREDITS.md` for third-party references (assembler
  syntax conventions, format specifications), tests for everything, `cargo fmt`, `clippy -D warnings`.
- Commit messages end with the standard attribution lines; never push to master unless told to; GitHub tag/secret
  APIs are not available from the sandbox (pins are `rev = ...`; secrets are added by the owner).
- Keep Windows working: PowerShell entry points for any new script (they just call the bash scripts through Git
  for Windows), LF line endings for shell scripts and baselines (`.gitattributes`).
- Do not weaken accuracy anywhere to make a tool easier; do not duplicate format logic that already lives in a
  crate.

## Definition of done for the first milestone (media tools)

`rcdisk` and `rctape` (and `rcsnap` if split) pass: list/info/validate on every image in the `software/` folders,
extract-then-add round trips that are byte-identical where the format allows, blank-image creation that the
emulator boots/reads (verified headless), and the agreed conversions with tests; wired into `build_all`,
`smoke_test`, `bump_pin` and CI; documented in the README and `CLAUDE.md` of the new repository.
