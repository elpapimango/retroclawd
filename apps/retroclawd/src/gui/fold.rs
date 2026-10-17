//! Accented Latin letters to the plain ASCII letters the 8-bit machines have: a character the machine has no key for (`é`,
//! `Ü`, `ß`) is typed as its base letter (`e`, `U`, `ss`) instead of being dropped or, worse, falling back to whichever physical
//! key the host keyboard happened to type it on.

/// Each group of look-alike characters and the ASCII letters they fold to.
const FOLDS: &[(&str, &str)] = &[
    ("ÀÁÂÃÄÅĀĂĄ", "A"),
    ("àáâãäåāăą", "a"),
    ("Æ", "AE"),
    ("æ", "ae"),
    ("ĆĈĊČÇ", "C"),
    ("ćĉċčç", "c"),
    ("ĎĐ", "D"),
    ("ďđ", "d"),
    ("ÈÉÊËĒĔĖĘĚ", "E"),
    ("èéêëēĕėęě", "e"),
    ("ĜĞĠĢ", "G"),
    ("ĝğġģ", "g"),
    ("ĤĦ", "H"),
    ("ĥħ", "h"),
    ("ÌÍÎÏĨĪĬĮİ", "I"),
    ("ìíîïĩīĭįı", "i"),
    ("Ĵ", "J"),
    ("ĵ", "j"),
    ("Ķ", "K"),
    ("ķ", "k"),
    ("ĹĻĽĿŁ", "L"),
    ("ĺļľŀł", "l"),
    ("ÑŃŅŇ", "N"),
    ("ñńņň", "n"),
    ("ÒÓÔÕÖØŌŎŐ", "O"),
    ("òóôõöøōŏő", "o"),
    ("Œ", "OE"),
    ("œ", "oe"),
    ("ŔŖŘ", "R"),
    ("ŕŗř", "r"),
    ("ŚŜŞŠ", "S"),
    ("śŝşš", "s"),
    ("ß", "ss"),
    ("ŢŤŦ", "T"),
    ("ţťŧ", "t"),
    ("ÙÚÛÜŨŪŬŮŰŲ", "U"),
    ("ùúûüũūŭůűų", "u"),
    ("Ŵ", "W"),
    ("ŵ", "w"),
    ("ÝŸŶ", "Y"),
    ("ýÿŷ", "y"),
    ("ŹŻŽ", "Z"),
    ("źżž", "z"),
];

/// The ASCII letters `c` folds to (`é` -> `e`, `Æ` -> `AE`), `None` for anything that is not an accented Latin letter.
pub(crate) fn ascii_fold(c: char) -> Option<&'static str> {
    if c.is_ascii() {
        return None;
    }
    FOLDS
        .iter()
        .find(|(group, _)| group.contains(c))
        .map(|(_, plain)| *plain)
}

/// The single ASCII letter `c` folds to, for a key that can only type one character (`ß` -> `ss` has no single letter).
pub(crate) fn ascii_fold_char(c: char) -> Option<char> {
    let plain = ascii_fold(c)?;
    let mut chars = plain.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), None) => Some(letter),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accented_letters_fold_to_their_base_letters() {
        assert_eq!(ascii_fold('é'), Some("e"));
        assert_eq!(ascii_fold('Ü'), Some("U"));
        assert_eq!(ascii_fold('ñ'), Some("n"));
        assert_eq!(ascii_fold('ß'), Some("ss"));
        assert_eq!(ascii_fold('Æ'), Some("AE"));
        assert_eq!(ascii_fold('ł'), Some("l"));
        assert_eq!(ascii_fold_char('é'), Some('e'));
        assert_eq!(ascii_fold_char('ß'), None, "two letters do not fit one key");
        // ASCII and everything that is not a Latin letter stay as they are.
        assert_eq!(ascii_fold('e'), None);
        assert_eq!(ascii_fold('£'), None);
        assert_eq!(ascii_fold('中'), None);
    }
}
