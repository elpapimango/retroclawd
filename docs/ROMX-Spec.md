Act as a Principal Systems Architect and Binary File Format Specialist. Your task is to generate a complete, language-agnostic, exhaustive technical specification for the `.romx` media container format (Version 2.0). 

This document must contain ALL mathematical, structural, structural-alignment, bitwise, and architectural specifications required for a software engineer to write a native binary parser/writer from scratch in C, C++, Rust, TypeScript, or Python without needing any external context.

Generate the specification covering the following mandatory sections:

---

### SECTION 1: FORMAT OVERVIEW & DESIGN PHILOSOPHY
- **Core Purpose:** Single-file, read-only/immutable container for retro ROMs, disk/tape images, assets, preview media, cheats, and historical metadata.
- **Key Features:** Zero-copy memory mapping (`mmap`), 4KB sector alignment for direct DMA/kernel cache mapping, deterministic byte hashing (TorrentZip inspired), and cross-language serialization compatibility.

---

### SECTION 2: BINARY FILE LAYOUT & MEMORY MAPPING
Specify the exact byte layout, field offsets, types, and alignment padding rules:

1. **Fixed Binary Header (32 Bytes, `repr(C, packed)`):**
   - Byte 0-3: `magic` (`[0x52, 0x4F, 0x4D, 0x58]` / ASCII `"ROMX"`)
   - Byte 4-5: `version` (`u16`, Little-Endian, Current = `0x0001`)
   - Byte 6-7: `flags` (`u16`, Little-Endian bitmask: Bit 0 = Compressed Assets Present, Bit 1 = Read Only)
   - Byte 8-15: `sqlite_offset` (`u64`, Little-Endian byte offset to embedded database, default = `32`)
   - Byte 16-23: `sqlite_size` (`u64`, Little-Endian length in bytes of the SQLite payload)
   - Byte 24-31: `payload_offset` (`u64`, Little-Endian byte offset to binary payload, MUST be aligned to a 4096-byte boundary)

2. **Embedded Metadata Database Segment:**
   - Standard SQLite database file starting at `sqlite_offset`.
   - SQLite `PRAGMA page_size = 4096`.
   - Must be padded at the end with zero bytes (`0x00`) so that `sqlite_offset + sqlite_size + padding` aligns exactly to the next 4096-byte boundary (`payload_offset`).

3. **Contiguous Payload Segment:**
   - Sequentially stores uncompressed or compressed raw data streams (ROMs, CHDs, tapes, box art, videos).
   - Uncompressed ROM streams MUST begin at offsets aligned to 4096-byte boundaries relative to the start of the `.romx` file to enable zero-copy `mmap` slice mapping (`&[u8]`).

---

### SECTION 3: EMBEDDED SQLITE SCHEMA & TABLES
Provide full ANSI SQL DDL scripts and specify field constraints:

- `manifest` table: Key-value store for global build properties (e.g., `creator`, `created_timestamp`, `romx_spec_version`).
- `assets` table: 
  - `id`: INTEGER PRIMARY KEY AUTOINCREMENT
  - `virtual_path`: TEXT UNIQUE NOT NULL (POSIX slashes `/`, lowercase canonical)
  - `hash_sha256`: TEXT NOT NULL (Hexadecimal SHA-256 string of uncompressed stream)
  - `storage_type`: INTEGER NOT NULL (`0` = Embedded Payload, `1` = External Reference)
  - `compression_algo`: INTEGER DEFAULT 0 (`0` = Uncompressed/mmap-ready, `1` = zlib, `2` = zstd)
  - `uncompressed_size`: INTEGER NOT NULL
  - `compressed_size`: INTEGER NOT NULL
  - `payload_offset`: INTEGER (Absolute byte offset in the `.romx` file, NULL if external)
  - `alignment_bytes`: INTEGER DEFAULT 0 (Padding prior to stream offset)
- `game_metadata` table: Key-value metadata (MAME DAT info, parent/clone relationships, genre, release year).
- `cheats` table: Game Genie / Action Replay cheat strings (`title`, `code`, `enabled_by_default`).
- `tape_blocks` table: Indexing table for retro tape structures (Spectrum TAP/TZX) mapping block markers (`block_index`, `block_type`, `description`, `payload_offset`) for fast UI seeking.

---

### SECTION 4: CONVERSION PIPELINE & INGESTION RULES
Detail how external media formats map into `.romx` entries:
- **ZIP Archives:** Extracted entry by entry; files mapped to POSIX normalized paths and SHA-256 hashed.
- **MAME CHDs:** Embedded as continuous raw stream with `compression_algo = 0` to preserve CHD internal chunk compression without double-compression penalties.
- **Spectrum TAP/TZX Tapes:** `.tap` sequences mapped to virtual paths. `.tzx` pulse streams embedded raw, while pulse headers are parsed to create structural index rows in `tape_blocks`.

---

### SECTION 5: TORRENTZIP DETERMINISTIC COMPILATION RULES
Detail the required rules to guarantee identical SHA-256 output hashes across compilers and platforms:
1. **Path Normalization:** Replace all backslashes (`\`) with forward slashes (`/`). Strip leading/trailing slashes.
2. **Canonical Sorting:** Sort all asset `virtual_path` entries using lowercase ASCII byte order before staging or DB insertion.
3. **Metadata Stripping:** Exclude host OS file system timestamps (`mtime`, `ctime`), host attributes, or platform file permissions.
4. **Encoder Determinism:** Enforce fixed compression parameters and dictionary sizes for zstd/zlib encoders.

---

### SECTION 6: CROSS-LANGUAGE IMPLEMENTATION GUIDELINES
Provide implementation hints for:
- **C/C++:** `mmap()`/`MapViewOfFile()` offset calculations; memory cast of byte 0..31 into `struct RomxHeader`.
- **Rust:** `memmap2` slice mapping, `rusqlite` bundled operations, safety rules for alignment assertion.
- **TypeScript / Node.js:** `fs.readSync`, `BigInt64Array` byte parsing, and `better-sqlite3` buffer loading.
- **Python:** `mmap.mmap`, `struct.unpack("<4sHHQQQ")`, and `sqlite3.connect('/dev/shm')` or tempfile DB loading.

---

Acknowledge this prompt by generating the complete, readable specification document based on these rules.