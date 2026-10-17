//! Symbol names shown in the debugger's disassembly: a built-in table per machine (ROM entry points,
//! hardware registers) plus the user's own symbol files (`--symbols`). The annotating itself is
//! `machine::SymbolTable::annotate`, which works on the disassembler's text and so serves every CPU.

use std::sync::{Arc, Mutex, OnceLock};

use machine::SymbolTable;

static USER: OnceLock<SymbolTable> = OnceLock::new();
static CACHE: Mutex<Option<(String, Arc<SymbolTable>)>> = Mutex::new(None);

/// Reads the `--symbols` files (paths, URLs or `archive.zip#entry`, later files replace earlier names)
/// and keeps them for the rest of the run.
pub(crate) fn load_user_files(sources: &[String]) -> Result<(), String> {
    let mut table = SymbolTable::new();
    for source in sources {
        let loaded = crate::inputs::load(source)?;
        let text = String::from_utf8_lossy(&loaded.bytes);
        let parsed = SymbolTable::parse(&text).map_err(|e| format!("{source}: {e}"))?;
        table.merge(&parsed);
    }
    // A second call (tests) keeps the first table; the app calls this once at start-up.
    let _ = USER.set(table);
    Ok(())
}

/// The table for `machine_id`: the built-ins, overridden by the user's files. Built once per machine
/// (the debugger asks for it on every tick).
pub(crate) fn table_for(machine_id: &str) -> Arc<SymbolTable> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((id, table)) = cache.as_ref()
        && id == machine_id
    {
        return Arc::clone(table);
    }
    let mut table = builtin(machine_id);
    if let Some(user) = USER.get() {
        table.merge(user);
    }
    let table = Arc::new(table);
    *cache = Some((machine_id.to_string(), Arc::clone(&table)));
    table
}

fn builtin(machine_id: &str) -> SymbolTable {
    if machine_id == "c64" || machine_id == "c128" {
        let mut table = SymbolTable::from_entries(C64_KERNAL);
        table.merge(&SymbolTable::from_entries(C64_HARDWARE));
        table
    } else if machine_id.starts_with("zx-spectrum") {
        SymbolTable::from_entries(SPECTRUM_ROM)
    } else {
        SymbolTable::new()
    }
}

/// The KERNAL's documented jump table (`$FF81`-`$FFF3`), its three hardware vectors and the
/// most-used RAM vectors. The same addresses on the C128 (its jump table is compatible).
const C64_KERNAL: &[(u16, &str)] = &[
    (0xFF81, "CINT"),
    (0xFF84, "IOINIT"),
    (0xFF87, "RAMTAS"),
    (0xFF8A, "RESTOR"),
    (0xFF8D, "VECTOR"),
    (0xFF90, "SETMSG"),
    (0xFF93, "SECOND"),
    (0xFF96, "TKSA"),
    (0xFF99, "MEMTOP"),
    (0xFF9C, "MEMBOT"),
    (0xFF9F, "SCNKEY"),
    (0xFFA2, "SETTMO"),
    (0xFFA5, "ACPTR"),
    (0xFFA8, "CIOUT"),
    (0xFFAB, "UNTLK"),
    (0xFFAE, "UNLSN"),
    (0xFFB1, "LISTEN"),
    (0xFFB4, "TALK"),
    (0xFFB7, "READST"),
    (0xFFBA, "SETLFS"),
    (0xFFBD, "SETNAM"),
    (0xFFC0, "OPEN"),
    (0xFFC3, "CLOSE"),
    (0xFFC6, "CHKIN"),
    (0xFFC9, "CHKOUT"),
    (0xFFCC, "CLRCHN"),
    (0xFFCF, "CHRIN"),
    (0xFFD2, "CHROUT"),
    (0xFFD5, "LOAD"),
    (0xFFD8, "SAVE"),
    (0xFFDB, "SETTIM"),
    (0xFFDE, "RDTIM"),
    (0xFFE1, "STOP"),
    (0xFFE4, "GETIN"),
    (0xFFE7, "CLALL"),
    (0xFFEA, "UDTIM"),
    (0xFFED, "SCREEN"),
    (0xFFF0, "PLOT"),
    (0xFFF3, "IOBASE"),
    (0xFFFA, "NMI_VECTOR"),
    (0xFFFC, "RESET_VECTOR"),
    (0xFFFE, "IRQ_VECTOR"),
    (0x0000, "DDR6510"),
    (0x0001, "PORT6510"),
    (0x0314, "CINV"),
    (0x0316, "CBINV"),
    (0x0318, "NMINV"),
];

/// VIC-II, SID and CIA register addresses (the base of each block plus the registers programs touch
/// most), the colour RAM and the default screen.
const C64_HARDWARE: &[(u16, &str)] = &[
    (0xD000, "SPRITE0_X"),
    (0xD001, "SPRITE0_Y"),
    (0xD010, "SPRITES_X_MSB"),
    (0xD011, "VIC_CTRL1"),
    (0xD012, "VIC_RASTER"),
    (0xD015, "SPRITE_ENABLE"),
    (0xD016, "VIC_CTRL2"),
    (0xD018, "VIC_MEMORY"),
    (0xD019, "VIC_IRQ_STATUS"),
    (0xD01A, "VIC_IRQ_ENABLE"),
    (0xD020, "BORDER"),
    (0xD021, "BACKGROUND"),
    (0xD400, "SID_V1_FREQ_LO"),
    (0xD404, "SID_V1_CTRL"),
    (0xD40B, "SID_V2_CTRL"),
    (0xD412, "SID_V3_CTRL"),
    (0xD415, "SID_FILTER_LO"),
    (0xD418, "SID_VOLUME"),
    (0xD800, "COLOUR_RAM"),
    (0xDC00, "CIA1_PORT_A"),
    (0xDC01, "CIA1_PORT_B"),
    (0xDC02, "CIA1_DDR_A"),
    (0xDC03, "CIA1_DDR_B"),
    (0xDC04, "CIA1_TIMER_A"),
    (0xDC0D, "CIA1_ICR"),
    (0xDC0E, "CIA1_CRA"),
    (0xDD00, "CIA2_PORT_A"),
    (0xDD01, "CIA2_PORT_B"),
    (0xDD02, "CIA2_DDR_A"),
    (0xDD0D, "CIA2_ICR"),
    (0xDD0E, "CIA2_CRA"),
    (0x0400, "SCREEN"),
];

/// The 48K ROM's documented entry points and the system variables BASIC programs and loaders use.
/// (On the 128K models these are the 48K-compatible ROM's addresses.)
const SPECTRUM_ROM: &[(u16, &str)] = &[
    (0x0038, "MASK_INT"),
    (0x0066, "NMI"),
    (0x028E, "KEY_SCAN"),
    (0x04C2, "SA_BYTES"),
    (0x0556, "LD_BYTES"),
    (0x0D6B, "CLS"),
    (0x0DAF, "CL_ALL"),
    (0x15F2, "PRINT_A_2"),
    (0x1601, "CHAN_OPEN"),
    (0x5C00, "KSTATE"),
    (0x5C08, "LAST_K"),
    (0x5C3A, "ERR_NR"),
    (0x5C3B, "FLAGS"),
    (0x5C3D, "ERR_SP"),
    (0x5C48, "BORDCR"),
    (0x5C4B, "VARS"),
    (0x5C53, "PROG"),
    (0x5C59, "E_LINE"),
    (0x5C5D, "CH_ADD"),
    (0x5C78, "FRAMES"),
    (0x5C7B, "UDG"),
    (0x5C8D, "ATTR_P"),
    (0x5C8F, "ATTR_T"),
    (0x5CB2, "RAMTOP"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_tables_follow_the_machine() {
        assert_eq!(builtin("c64").annotate("JSR $FFD2"), "JSR $FFD2  ; CHROUT");
        assert_eq!(builtin("c128").annotate("STA $D020"), "STA $D020  ; BORDER");
        assert_eq!(
            builtin("zx-spectrum-48").annotate("CALL 0x0556"),
            "CALL 0x0556  ; LD_BYTES"
        );
        assert!(builtin("zx81").is_empty());
        // The Spectrum's table must not leak onto a 6502 machine and vice versa.
        assert_eq!(builtin("c64").annotate("CALL 0x0556"), "CALL 0x0556");
    }
}
