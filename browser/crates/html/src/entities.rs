//! Character references (WHATWG 13.2.5.2, plan/05 §5.2).
//!
//! Full numeric references (decimal/hex, with the C1 Windows-1252 fixups
//! and U+FFFD fallbacks) plus the common named entities. The named table
//! covers everyday web text; completing all 2000+ rare entries is tracked
//! as Phase 11 compatibility work (the table is a single match to extend).

/// How broken a consumed reference was (for parse-error reporting).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceBadness {
    /// Clean `&name;` / `&#123;`.
    None,
    /// Missing trailing semicolon (still resolved per spec).
    MissingSemicolon,
    /// Null, surrogate, out-of-range or control value (resolved to U+FFFD
    /// or the C1 fixup, per spec).
    Invalid,
}

/// Try to consume a character reference at the start of `input`, where
/// `input` begins right after the `&`.
///
/// Returns the replacement text, the number of **bytes** consumed from
/// `input` (excluding the `&` itself), and how broken it was. Returns
/// `None` when no reference starts here (the `&` stays literal).
///
/// `in_attribute` selects the attribute rule: a semicolon-less match
/// followed by ASCII alphanumeric or `=` is left literal.
pub fn consume_reference(
    input: &str,
    in_attribute: bool,
) -> Option<(String, usize, ReferenceBadness)> {
    let bytes = input.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    if bytes[0] == b'#' {
        return consume_numeric(&input[1..]);
    }
    // Longest table match (entries carry their semicolon, except legacy).
    // The scan window is capped for pathological inputs, on a char boundary.
    let mut haystack_len = input.len().min(33);
    while haystack_len > 0 && !input.is_char_boundary(haystack_len) {
        haystack_len -= 1;
    }
    let haystack = &input[..haystack_len];
    let mut best: Option<(&str, char)> = None;
    let mut best_len = 0usize;
    let mut end = 0usize;
    for (index, ch) in haystack.char_indices() {
        if !(ch.is_ascii_alphanumeric() || ch == '#' || ch == ';') {
            break;
        }
        end = index + ch.len_utf8();
        if ch == ';' {
            if let Some(replacement) = lookup_named(&haystack[..end]) {
                best = Some((&haystack[..end], replacement));
                best_len = end;
                break;
            }
        }
    }
    // Semicolon-less match: longest table name that fits the prefix
    // (spec longest-match over all named references). In attributes, a
    // match followed by ASCII alphanumeric or `=` stays literal.
    if best.is_none() {
        let max_name = end.min(32);
        let mut len = max_name.min(haystack.len());
        // Only name characters can extend the match.
        while len > 0 && !haystack.is_char_boundary(len) {
            len -= 1;
        }
        let mut probe = len;
        while probe > 0 {
            while probe > 0 && !haystack.is_char_boundary(probe) {
                probe -= 1;
            }
            if probe == 0 {
                break;
            }
            let candidate = &haystack[..probe];
            if candidate.chars().all(|ch| ch.is_ascii_alphanumeric()) {
                let mut with_semi = candidate.to_owned();
                with_semi.push(';');
                if lookup_named(&with_semi).is_some() {
                    let next = haystack[probe..].chars().next();
                    if in_attribute
                        && matches!(
                            next,
                            Some('=') | Some('0'..='9') | Some('a'..='z') | Some('A'..='Z')
                        )
                    {
                        return None;
                    }
                    let replacement = lookup_named(&with_semi).expect("checked");
                    return Some((
                        replacement.to_string(),
                        probe,
                        ReferenceBadness::MissingSemicolon,
                    ));
                }
            }
            probe -= 1;
        }
    }
    match (best, best_len) {
        (Some((_, replacement)), len) => {
            let bad = if haystack[..len].ends_with(';') {
                ReferenceBadness::None
            } else {
                ReferenceBadness::MissingSemicolon
            };
            Some((replacement.to_string(), len, bad))
        }
        _ => None,
    }
}

/// Numeric `&#123;` / `&#x1F;` reference (input starts after `#`).
fn consume_numeric(input: &str) -> Option<(String, usize, ReferenceBadness)> {
    let bytes = input.as_bytes();
    let (digits, value, hex) = if !bytes.is_empty() && (bytes[0] == b'x' || bytes[0] == b'X') {
        let len: usize = input[1..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .map(char::len_utf8)
            .sum();
        if len == 0 {
            return None;
        }
        let value = u32::from_str_radix(&input[1..1 + len], 16).ok()?;
        (1 + len, value, true)
    } else {
        let len: usize = input
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .map(char::len_utf8)
            .sum();
        if len == 0 {
            return None;
        }
        let value = input[..len].parse::<u32>().ok()?;
        (len, value, false)
    };
    let _ = hex;
    let (consumed, bad) = match input[digits..].chars().next() {
        // +1 for the leading `#` (input starts after `&`).
        Some(';') => (1 + digits + 1, ReferenceBadness::None),
        _ => (1 + digits, ReferenceBadness::MissingSemicolon),
    };
    let (ch, bad) = map_numeric(value, bad);
    Some((ch.to_string(), consumed, bad))
}

/// Spec value mapping for numeric references.
fn map_numeric(value: u32, bad: ReferenceBadness) -> (char, ReferenceBadness) {
    // C1 controls map to Windows-1252 (spec table). The five bytes
    // undefined in Windows-1252 (81 8D 8F 90 9D) map to themselves.
    const C1: [char; 32] = [
        '€', '\u{0081}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{008D}', 'Ž',
        '\u{008F}', '\u{0090}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ',
        '\u{009D}', 'ž', 'Ÿ',
    ];
    if value == 0 || (0xD800..=0xDFFF).contains(&value) || value > 0x10FFFF {
        return ('\u{FFFD}', ReferenceBadness::Invalid);
    }
    if (0x80..=0x9F).contains(&value) {
        return (C1[(value - 0x80) as usize], ReferenceBadness::Invalid);
    }
    match char::from_u32(value) {
        Some(ch) => (ch, bad),
        None => ('\u{FFFD}', ReferenceBadness::Invalid),
    }
}

/// Common named references, semicolon included. Single-char values only.
fn lookup_named(name: &str) -> Option<char> {
    Some(match name {
        "amp;" => '&',
        "AMP;" => '&',
        "lt;" => '<',
        "LT;" => '<',
        "gt;" => '>',
        "GT;" => '>',
        "quot;" => '"',
        "QUOT;" => '"',
        "apos;" => '\'',
        "nbsp;" => ' ',
        "iexcl;" => '¡',
        "cent;" => '¢',
        "pound;" => '£',
        "curren;" => '¤',
        "yen;" => '¥',
        "brvbar;" => '¦',
        "sect;" => '§',
        "uml;" => '¨',
        "copy;" => '©',
        "COPY;" => '©',
        "ordf;" => 'ª',
        "laquo;" => '«',
        "not;" => '¬',
        "shy;" => '\u{AD}',
        "reg;" => '®',
        "REG;" => '®',
        "macr;" => '¯',
        "deg;" => '°',
        "plusmn;" => '±',
        "sup2;" => '²',
        "sup3;" => '³',
        "acute;" => '´',
        "micro;" => 'µ',
        "para;" => '¶',
        "middot;" => '·',
        "cedil;" => '¸',
        "sup1;" => '¹',
        "ordm;" => 'º',
        "raquo;" => '»',
        "frac14;" => '¼',
        "frac12;" => '½',
        "frac34;" => '¾',
        "iquest;" => '¿',
        "Agrave;" => 'À',
        "Aacute;" => 'Á',
        "Acirc;" => 'Â',
        "Atilde;" => 'Ã',
        "Auml;" => 'Ä',
        "Aring;" => 'Å',
        "AElig;" => 'Æ',
        "Ccedil;" => 'Ç',
        "Egrave;" => 'È',
        "Eacute;" => 'É',
        "Ecirc;" => 'Ê',
        "Euml;" => 'Ë',
        "Igrave;" => 'Ì',
        "Iacute;" => 'Í',
        "Icirc;" => 'Î',
        "Iuml;" => 'Ï',
        "ETH;" => 'Ð',
        "Ntilde;" => 'Ñ',
        "Ograve;" => 'Ò',
        "Oacute;" => 'Ó',
        "Ocirc;" => 'Ô',
        "Otilde;" => 'Õ',
        "Ouml;" => 'Ö',
        "times;" => '×',
        "Oslash;" => 'Ø',
        "Ugrave;" => 'Ù',
        "Uacute;" => 'Ú',
        "Ucirc;" => 'Û',
        "Uuml;" => 'Ü',
        "Yacute;" => 'Ý',
        "THORN;" => 'Þ',
        "szlig;" => 'ß',
        "agrave;" => 'à',
        "aacute;" => 'á',
        "acirc;" => 'â',
        "atilde;" => 'ã',
        "auml;" => 'ä',
        "aring;" => 'å',
        "aelig;" => 'æ',
        "ccedil;" => 'ç',
        "egrave;" => 'è',
        "eacute;" => 'é',
        "ecirc;" => 'ê',
        "euml;" => 'ë',
        "igrave;" => 'ì',
        "iacute;" => 'í',
        "icirc;" => 'î',
        "iuml;" => 'ï',
        "eth;" => 'ð',
        "ntilde;" => 'ñ',
        "ograve;" => 'ò',
        "oacute;" => 'ó',
        "ocirc;" => 'ô',
        "otilde;" => 'õ',
        "ouml;" => 'ö',
        "divide;" => '÷',
        "oslash;" => 'ø',
        "ugrave;" => 'ù',
        "uacute;" => 'ú',
        "ucirc;" => 'û',
        "uuml;" => 'ü',
        "yacute;" => 'ý',
        "thorn;" => 'þ',
        "yuml;" => 'ÿ',
        "OElig;" => 'Œ',
        "oelig;" => 'œ',
        "Scaron;" => 'Š',
        "scaron;" => 'š',
        "Yuml;" => 'Ÿ',
        "fnof;" => 'ƒ',
        "circ;" => 'ˆ',
        "tilde;" => '˜',
        "Alpha;" => 'Α',
        "Beta;" => 'Β',
        "Gamma;" => 'Γ',
        "Delta;" => 'Δ',
        "Epsilon;" => 'Ε',
        "Zeta;" => 'Ζ',
        "Eta;" => 'Η',
        "Theta;" => 'Θ',
        "Iota;" => 'Ι',
        "Kappa;" => 'Κ',
        "Lambda;" => 'Λ',
        "Mu;" => 'Μ',
        "Nu;" => 'Ν',
        "Xi;" => 'Ξ',
        "Omicron;" => 'Ο',
        "Pi;" => 'Π',
        "Rho;" => 'Ρ',
        "Sigma;" => 'Σ',
        "Tau;" => 'Τ',
        "Upsilon;" => 'Υ',
        "Phi;" => 'Φ',
        "Chi;" => 'Χ',
        "Psi;" => 'Ψ',
        "Omega;" => 'Ω',
        "alpha;" => 'α',
        "beta;" => 'β',
        "gamma;" => 'γ',
        "delta;" => 'δ',
        "epsilon;" => 'ε',
        "zeta;" => 'ζ',
        "eta;" => 'η',
        "theta;" => 'θ',
        "iota;" => 'ι',
        "kappa;" => 'κ',
        "lambda;" => 'λ',
        "mu;" => 'μ',
        "nu;" => 'ν',
        "xi;" => 'ξ',
        "omicron;" => 'ο',
        "pi;" => 'π',
        "rho;" => 'ρ',
        "sigmaf;" => 'ς',
        "sigma;" => 'σ',
        "tau;" => 'τ',
        "upsilon;" => 'υ',
        "phi;" => 'φ',
        "chi;" => 'χ',
        "psi;" => 'ψ',
        "omega;" => 'ω',
        "thetasym;" => 'ϑ',
        "upsih;" => 'ϒ',
        "piv;" => 'ϖ',
        "ensp;" => ' ',
        "emsp;" => ' ',
        "thinsp;" => ' ',
        "zwnj;" => '‌',
        "zwj;" => '‍',
        "lrm;" => '‎',
        "rlm;" => '‏',
        "ndash;" => '–',
        "mdash;" => '—',
        "lsquo;" => '‘',
        "rsquo;" => '’',
        "sbquo;" => '‚',
        "ldquo;" => '“',
        "rdquo;" => '”',
        "bdquo;" => '„',
        "dagger;" => '†',
        "Dagger;" => '‡',
        "bull;" => '•',
        "hellip;" => '…',
        "permil;" => '‰',
        "prime;" => '′',
        "Prime;" => '″',
        "lsaquo;" => '‹',
        "rsaquo;" => '›',
        "oline;" => '‾',
        "frasl;" => '⁄',
        "euro;" => '€',
        "image;" => 'ℑ',
        "weierp;" => '℘',
        "real;" => 'ℜ',
        "trade;" => '™',
        "TRADE;" => '™',
        "alefsym;" => 'ℵ',
        "larr;" => '←',
        "uarr;" => '↑',
        "rarr;" => '→',
        "darr;" => '↓',
        "harr;" => '↔',
        "crarr;" => '↵',
        "lArr;" => '⇐',
        "uArr;" => '⇑',
        "rArr;" => '⇒',
        "dArr;" => '⇓',
        "hArr;" => '⇔',
        "forall;" => '∀',
        "part;" => '∂',
        "exist;" => '∃',
        "empty;" => '∅',
        "nabla;" => '∇',
        "isin;" => '∈',
        "notin;" => '∉',
        "ni;" => '∋',
        "prod;" => '∏',
        "sum;" => '∑',
        "minus;" => '−',
        "lowast;" => '∗',
        "radic;" => '√',
        "prop;" => '∝',
        "infin;" => '∞',
        "ang;" => '∠',
        "and;" => '∧',
        "or;" => '∨',
        "cap;" => '∩',
        "cup;" => '∪',
        "int;" => '∫',
        "there4;" => '∴',
        "sim;" => '∼',
        "cong;" => '≅',
        "asymp;" => '≈',
        "ne;" => '≠',
        "equiv;" => '≡',
        "le;" => '≤',
        "ge;" => '≥',
        "sub;" => '⊂',
        "sup;" => '⊃',
        "nsub;" => '⊄',
        "sube;" => '⊆',
        "supe;" => '⊇',
        "oplus;" => '⊕',
        "otimes;" => '⊗',
        "perp;" => '⊥',
        "sdot;" => '⋅',
        "lceil;" => '⌈',
        "rceil;" => '⌉',
        "lfloor;" => '⌊',
        "rfloor;" => '⌋',
        "lang;" => '⟨',
        "rang;" => '⟩',
        "loz;" => '◊',
        "spades;" => '♠',
        "clubs;" => '♣',
        "hearts;" => '♥',
        "diams;" => '♦',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_named() {
        assert_eq!(
            consume_reference("amp;rest", false),
            Some(("&".to_owned(), 4, ReferenceBadness::None))
        );
        assert_eq!(
            consume_reference("lt;", false),
            Some(("<".to_owned(), 3, ReferenceBadness::None))
        );
    }

    #[test]
    fn legacy_without_semicolon_in_text() {
        assert_eq!(
            consume_reference("amp ", false),
            Some(("&".to_owned(), 3, ReferenceBadness::MissingSemicolon))
        );
    }

    #[test]
    fn legacy_blocked_in_attribute_before_equals() {
        assert_eq!(consume_reference("amp=x", true), None);
    }

    #[test]
    fn longest_match_wins_over_unknown_tails() {
        // `not` is a table entry: `&notanentity;` resolves `&not` and
        // leaves `anentity;` as literal text (spec longest-match rule,
        // same as WPT `&noti;` → `¬i;`).
        assert_eq!(
            consume_reference("notanentity;", false),
            Some(("¬".to_owned(), 3, ReferenceBadness::MissingSemicolon))
        );
    }

    #[test]
    fn numeric_decimal_and_hex() {
        assert_eq!(
            consume_reference("#65;", false),
            Some(("A".to_owned(), 4, ReferenceBadness::None))
        );
        assert_eq!(
            consume_reference("#x41;", false),
            Some(("A".to_owned(), 5, ReferenceBadness::None))
        );
    }

    #[test]
    fn numeric_edge_cases() {
        assert_eq!(
            consume_reference("#0;", false),
            Some(("\u{FFFD}".to_owned(), 3, ReferenceBadness::Invalid))
        );
        // C1 fixup: 0x80..0x9F map to Windows-1252 (0x80 -> €, 0x82 -> ‚;
        // the bytes undefined in Windows-1252 stay themselves).
        assert_eq!(
            consume_reference("#x80;", false),
            Some(("€".to_owned(), 5, ReferenceBadness::Invalid))
        );
        assert_eq!(
            consume_reference("#x82;", false).map(|t| t.0),
            Some("‚".to_owned())
        );
        assert_eq!(
            consume_reference("#x81;", false).map(|t| t.0),
            Some("".to_owned())
        );
        // Surrogates and overflow become U+FFFD.
        assert_eq!(
            consume_reference("#xD800;", false).map(|t| t.0),
            Some("\u{FFFD}".to_owned())
        );
    }
}
