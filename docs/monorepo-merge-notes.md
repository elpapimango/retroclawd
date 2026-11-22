# Monorepo merge: what was done and what is left to review (2026-11-22)

The eight repositories (`retroclawd`, `retroclawd-crates`, `-sinclair`, `-amstrad`, `-commodore`, `-atari`, `-snk`,
`-sega`) and `retroclawd-tools` (branch `phase5-tools`) are now one repository with their history kept
(`git filter-repo --to-subdirectory-filter`, then merged with `--allow-unrelated-histories`; `git log --follow` and
`git blame` work through the merges).

## What changed

| Before | After |
|---|---|
| 7 git submodules + `retroclawd-tools` next to the checkout, family workspaces pinned the shared crates by git `rev` | one cargo workspace (`Cargo.toml` at the root), path dependencies, one `Cargo.lock` |
| every machine a `cdylib` loaded from `plugins/` at run time through a C ABI (`RcSlice`, vtables, bincode messages) | every machine linked into the executable; `plugin_api::MachinePlugin` is an ordinary Rust trait, requests and responses are the typed enums of `protocol.rs`, no serialisation |
| `retroclawd.exe` + a `plugins/` folder + `*.firmware.toml` beside it | `rcd.exe` (player) and `rcdd.exe` (developer), nothing else; each family's `firmware.toml` is embedded with `include_str!` |
| debuggers behind `debug-ui`/`debug-zrcp`/`debug-log` (default on) | `rcd` has none; `rcdd` is `--features debug` (those three, plus the machine-side hooks below) |

Removed: `plugin-dlls/` crates, `plugin_api::{abi, codec, compat}`, `libloading`, `RETROCLAWD_MACHINE_PLUGIN*`,
`--plugin machine=<path>`, `tools/bump_pin.*`, the per-family CI workflows, `docs/plugin-abi.md`, the protocol wire
fingerprint test (`tests/protocol_schema.rs`; there is no wire any more), the unit tests of descriptor/vtable skew.
Kept: the `PROTOCOL_VERSION` and `MACHINE_PROFILE_CONTRACT_VERSION` constants (host and machines are now always built
together, the checks are cheap and `--list-plugins` prints them).

## Verified

* `tools/smoke_test.sh`: all seven boot screens (ZX81, Spectrum 48K/128K, CPC464/6128, C64, C128) are bit-identical to
  the recorded hashes from the plugin-DLL build.
* `rcd --list-plugins` lists all ten plugins / sixteen machines from one 60 MB executable.
* `cargo test --workspace --features debug --release --no-fail-fast`: every test passes; `cargo test -p retroclawd` (no debug feature): 285 pass. `rcd --help` has no `--show-overlay`/`--symbols`, `rcdd --help` has them. Not run: the scenario manifests and `rc-suite` (need the goldens folder and reference emulators), the CPC acid suite, the Windows/macOS CI legs.

## Left to review (not done, or done only partly)

1. **Debug compile-out.** Behind `feature = "debug"` (absent from `rcd`, `rcda`, `rcdc`, `rcdh`): `--show-overlay`, `--symbols`,
   `--plugin`, the `debugger` and `debuggers` modules (browser debugger, ZRCP, log sink, the emulation thread's
   `DebuggerLink` with breakpoints and stepping), `debug_symbols`, `machine::PortWatcher::note` (a no-op without it),
   the Atari ST instruction history, exception log and memory watch points, and the machine options `trace_file`,
   `trace_limit`, `io_trace_file`, `watch` and `diag_file` (Atari ST) and `diag_file` (Mega Drive, Neo Geo). Left in
   `rcd` on purpose: the plugins' `MachineCommand::Debug` handlers (register, memory and chip-snapshot reads that the
   host also uses for `--dump-memory`, auto-run and the tape display; they run only when asked, never per
   instruction), the debug overlay GUI code (inert without `--show-overlay`, which is gone) and the cold code behind the
   removed options (unreachable, not conditionally compiled). Unit tests that exercise debug-only options need
   `--features debug` (CI and `cargo test --workspace --features debug` do that).
2. **`rcda` / `rcdc` / `rcdh` exist** (`tools/build_all.sh --variant a|c|h`): arcade = Neo Geo; console = Mega Drive/Mega-CD
   and Neo Geo; home = Sinclair, Amstrad, Commodore, Atari. No debug builds of them, and the release packaging script
   still ships only `rcd` and `rcdd`. A variant built with another kind's machines linked in fails to compile.
3. **Two `[[bin]]` entries share `src/main.rs`**, so cargo prints "file found to be present in multiple build targets"
   on every build. Harmless; the clean fix is moving the program into a library crate with two three-line `main`s.
4. **`rcd` and `rcdd` need different cargo target folders** (`target/release` and `target/release-debug`) because the
   feature set of the machine crates differs; `tools/build_all.sh` does that. Plain `cargo build --features debug`
   rebuilds the shared crates in the same folder.
5. **Plugin unit tests keep a byte-level shim.** The five plugins whose tests drove the old C functions
   (`zx81`, `zx-spectrum`, `cpc`, `c64`, `c128`) now call typed functions through `plugin_api::testkit` (feature
   `testkit`, dev-only, serialises requests with bincode so the old assertions stay as they were). Rewriting those tests
   to call the typed functions directly would remove `testkit` and the `unsafe { }` blocks that the shim turned
   superfluous.
6. **Crate and directory names still say "plugin"** (`*-machine-plugin`, `crates/plugins/`, `plugin-api`,
   `--list-plugins`, `src/plugin_bridge.rs`). Renaming them (`machine-api`, `family`...) touches every manifest.
7. **File and variable names kept on purpose:** the config files (`retroclawd.toml`, `retroclawd.state.toml`), the
   cache folder, the `RETROCLAWD_BIN` variable used by `rc-suite`, the crate `retroclawd` and its window title.
8. **`retroclawd-enterprise`** (Enterprise 64/128) was not merged: it was never a submodule here, its branch depends on
   an unmerged `retroclawd-crates` branch (`feature/enterprise-64-128`), and its local checkout was not found. The
   repository is left as it is (not archived).
9. **Untracked files of the old working copy were not carried over:** `AGENTS.md`, `.clinerules/`, `.cursor/`,
   `.opencode/`, `.windsurf/`, `.github/copilot-instructions.md`, `docs/*.pdf` (Atari hardware manuals), `saves/`.
   They are still in the old folder.
10. Windows/macOS legs of `release-artifacts.yml` remain unrun (as before); the CI file now builds `rcd`, `rcdd` and tests
    with `--features debug`, and drops `cargo clippy -D warnings` because the merged crates were never clippy-clean as one
    workspace.
11. `docs/superpowers/**`, the family `CLAUDE.md`/`README.md` files and the per-family TODOs still describe the repository
    split in places ("the Commodore submodule", `plugin-dlls`, "pin retroclawd-crates"). The root `CLAUDE.md` says so at
    the top; the older text was not rewritten.
