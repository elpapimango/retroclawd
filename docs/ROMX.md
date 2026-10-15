Act as a Principal Systems Engineer and Rust Architect. We are continuing work on the `romx` single-file container ecosystem for retro gaming ROMs, tapes, disks, media, and metadata. 

### CORE DESIGN & ARCHITECTURE RECAP

1. **Workspace Layout:**
   - `romx` (Core Library): Zero-copy `mmap` reader (`memmap2`), 4KB sector alignment, SQLite metadata engine (`rusqlite`), and format conversion traits.
   - `romx-pack` (CLI Application): Uses `clap` (derive API) with `build`, `convert`, `inspect`, `verify`, and `extract` subcommands.

2. **Binary Layout (`.romx`):**
   - **Header Block:** Fixed 32 bytes (`repr(C, packed)`): Magic `"ROMX"`, version (`u16`), flags (`u16`), SQLite offset (`u64`), SQLite size (`u64`), payload offset (`u64`, 4KB-aligned).
   - **SQLite DB:** Read-only (`PRAGMA page_size = 4096`), padded to 4KB sector boundary. Stores `manifest`, `assets`, `game_metadata`, `cheats`, and `tape_blocks`.
   - **Binary Payload:** Uncompressed ROM/disk streams aligned to 4KB offsets for zero-copy memory mapping. Compressed UI media (zstd/zlib).

3. **Format Converters (`romx::convert` Module):**
   - Implements `FormatConverter` trait (`detect` and `convert`).
   - `ZipConverter`: Extracts and stages `.zip` archive streams into `RomxBuilder`.
   - `ChdConverter`: Embeds raw MAME CHD streams with `compression_algo = 0` to preserve internal chunking without double compression.
   - `TapeConverter`: Parses Spectrum `.tap` and `.tzx` tape files. For `.tzx`, embeds raw pulse streams and indexes tape blocks into the `tape_blocks` table for UI seeking.

4. **TorrentZip-Inspired Determinism Engine:**
   - **Path Normalization:** Force all path separators to POSIX slashes (`/`).
   - **Canonical Sorting:** Sort `virtual_path` entries using lowercase ASCII order.
   - **Metadata Stripping:** Exclude host OS file attributes, `mtime`, and permissions from SQLite records.
   - **Fixed Encoders:** Enforce fixed compression levels for zstd/zlib encoders.
   - **Verification:** Expose `romx::verify_determinism(&Path)` asserting two containers built from identical sources yield identical SHA-256 hashes.

5. **CLI Syntax (`romx-pack`):**
   - `romx-pack build --mode <non-merged|merged|split> --dat <file.xml> --rom-path <dir> --out <file.romx>`
   - `romx-pack convert --input <file.zip|chd|tzx|tap> [--chd <file.chd>] [--media <dir>] --out <file.romx>`
   - `romx-pack inspect <file.romx>` (JSON offset/metadata breakdown)
   - `romx-pack verify <file.romx>` (SHA-256 payload verification)
   - `romx-pack extract <file.romx> --out-dir <dir>`

---

Please acknowledge that you have fully ingested this context, and let me know you are ready to continue developing the `romx` ecosystem.