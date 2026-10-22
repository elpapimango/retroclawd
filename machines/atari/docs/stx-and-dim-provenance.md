# `.stx` (Pasti) and `.dim`: provenance and recommendation

Phase 2 reads `.st` and `.msa` floppy images and writes neither of these. This note records what was found and what is
recommended, so the owner can decide (decisions 2 and 3 of the Phase 2 plan, both taken as "no code").

## `.stx` (Pasti)

- **What it is:** a track-level disk image format of the Pasti project, used to keep copy-protected floppies: sector
  and track data with timing information and "fuzzy" bits.
- **What was found:** nothing here. No document of the format was fetched in this work, so no layout is recorded. The
  design (spec section 13 table and section 24 item 4) says it is "documented by reverse engineering only; licence of
  the format docs to check": that check has not been made.
- **What it would take:** a licence-clean specification (or a written permission) and sample files whose redistribution is
  allowed; the WD1772 model in `retroclawd-crates` already has the read-address/read-track/write-track behaviour a track
  image needs, so the emulator side is not the obstacle.
- **Recommendation: no-go for Phase 2.** Revisit when a specification with a usable licence is in hand. Nothing in the
  tree names or parses `.stx`.

## `.dim` (FastCopy / Pro DIM)

- **What it is:** a sector-image container of the FastCopy family: a small header in front of a raw `.st`-like image.
- **What was found:** nothing here; the header layout was not researched (the plan said to skip until someone has a
  file to test with).
- **Recommendation: no-go** until a sample file and a source for its header exist. A `.dim` can be converted to `.st` by
  removing its header outside this project in the meantime.

Neither format is bundled, downloaded or required by any test.
