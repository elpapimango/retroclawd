# Machines are shared libraries behind a C-ABI

The Host links no Machine code. Each Family builds Plugins as shared libraries that the Host discovers and loads at run time through the versioned C-ABI in `plugin-api`, so a Machine can be built, shipped and replaced without rebuilding the Host. The cost is a serialised protocol for most calls, with a raw `step`/`halted`/`pc` fast path because per-instruction encoding was too slow. Compatibility rules are in `docs/plugin-abi.md`.
