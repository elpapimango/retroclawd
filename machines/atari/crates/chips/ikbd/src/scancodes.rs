//! Appendix A of the protocol document ("GSX Standard Keyboard Mapping"). The names are the host's key names for the
//! same key position (W3C `code` values, as Bevy names them), so a host keyboard maps onto the ST's without a
//! table; the document's keytop is in the comment.

/// Every key with its make code. Codes the document marks `{NOT USED}` (`$37`, `$45`, `$46`, `$49`, `$4C`, `$4F`,
/// `$51`, `$54-$5F`) have no entry.
pub const SCANCODES: &[(&str, u8)] = &[
    ("Escape", 0x01),       // Esc
    ("Digit1", 0x02),       // 1
    ("Digit2", 0x03),       // 2
    ("Digit3", 0x04),       // 3
    ("Digit4", 0x05),       // 4
    ("Digit5", 0x06),       // 5
    ("Digit6", 0x07),       // 6
    ("Digit7", 0x08),       // 7
    ("Digit8", 0x09),       // 8
    ("Digit9", 0x0A),       // 9
    ("Digit0", 0x0B),       // 0
    ("Minus", 0x0C),        // -
    ("Equal", 0x0D),        // ==
    ("Backspace", 0x0E),    // BS
    ("Tab", 0x0F),          // TAB
    ("KeyQ", 0x10),         // Q
    ("KeyW", 0x11),         // W
    ("KeyE", 0x12),         // E
    ("KeyR", 0x13),         // R
    ("KeyT", 0x14),         // T
    ("KeyY", 0x15),         // Y
    ("KeyU", 0x16),         // U
    ("KeyI", 0x17),         // I
    ("KeyO", 0x18),         // O
    ("KeyP", 0x19),         // P
    ("BracketLeft", 0x1A),  // [
    ("BracketRight", 0x1B), // ]
    ("Enter", 0x1C),        // RET
    ("ControlLeft", 0x1D),  // CNTL
    ("KeyA", 0x1E),         // A
    ("KeyS", 0x1F),         // S
    ("KeyD", 0x20),         // D
    ("KeyF", 0x21),         // F
    ("KeyG", 0x22),         // G
    ("KeyH", 0x23),         // H
    ("KeyJ", 0x24),         // J
    ("KeyK", 0x25),         // K
    ("KeyL", 0x26),         // L
    ("Semicolon", 0x27),    // ;
    ("Quote", 0x28),        // '
    ("Backquote", 0x29),    // `
    ("ShiftLeft", 0x2A),    // (LEFT) SHIFT
    ("Backslash", 0x2B),    // \
    ("KeyZ", 0x2C),         // Z
    ("KeyX", 0x2D),         // X
    ("KeyC", 0x2E),         // C
    ("KeyV", 0x2F),         // V
    ("KeyB", 0x30),         // B
    ("KeyN", 0x31),         // N
    ("KeyM", 0x32),         // M
    ("Comma", 0x33),        // ,
    ("Period", 0x34),       // .
    ("Slash", 0x35),        // /
    ("ShiftRight", 0x36),   // (RIGHT) SHIFT
    ("AltLeft", 0x38),      // ALT
    ("Space", 0x39),        // SPACE BAR
    ("CapsLock", 0x3A),     // CAPS LOCK
    ("F1", 0x3B),
    ("F2", 0x3C),
    ("F3", 0x3D),
    ("F4", 0x3E),
    ("F5", 0x3F),
    ("F6", 0x40),
    ("F7", 0x41),
    ("F8", 0x42),
    ("F9", 0x43),
    ("F10", 0x44),
    ("Home", 0x47),             // HOME
    ("ArrowUp", 0x48),          // UP ARROW
    ("NumpadSubtract", 0x4A),   // KEYPAD -
    ("ArrowLeft", 0x4B),        // LEFT ARROW
    ("ArrowRight", 0x4D),       // RIGHT ARROW
    ("NumpadAdd", 0x4E),        // KEYPAD +
    ("ArrowDown", 0x50),        // DOWN ARROW
    ("Insert", 0x52),           // INSERT
    ("Delete", 0x53),           // DEL
    ("IntlBackslash", 0x60),    // ISO KEY
    ("Undo", 0x61),             // UNDO
    ("Help", 0x62),             // HELP
    ("NumpadParenLeft", 0x63),  // KEYPAD (
    ("NumpadParenRight", 0x64), // KEYPAD )
    ("NumpadDivide", 0x65),     // KEYPAD /
    ("NumpadMultiply", 0x66),   // KEYPAD *
    ("Numpad7", 0x67),
    ("Numpad8", 0x68),
    ("Numpad9", 0x69),
    ("Numpad4", 0x6A),
    ("Numpad5", 0x6B),
    ("Numpad6", 0x6C),
    ("Numpad1", 0x6D),
    ("Numpad2", 0x6E),
    ("Numpad3", 0x6F),
    ("Numpad0", 0x70),
    ("NumpadDecimal", 0x71), // KEYPAD .
    ("NumpadEnter", 0x72),   // KEYPAD ENTER
];

/// The make code for a key name, ignoring case.
pub fn scancode(name: &str) -> Option<u8> {
    SCANCODES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|&(_, c)| c)
}
