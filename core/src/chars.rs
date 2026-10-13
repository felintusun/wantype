//! Character folding shared by the sentence and the keystrokes, from the
//! compatibility tables in `compat.rs`.

use crate::compat::{FOLD, KANA, VOICED};

/// Fold `c` for matching: compatibility forms to their plain form
/// (fullwidth `Ａ`/`６`/`－` to ASCII, halfwidth `ｶ` to `カ`, the halfwidth
/// and combining (semi-)voiced marks to the spacing ones kana input uses),
/// then lowercase. Always one char to one char (a char whose folded form
/// would be longer, like `㌔` or `İ`, is kept as is), so indices into the
/// folded sentence stay valid for the original one.
pub fn fold(c: char) -> char {
    let c = match c {
        // Fullwidth ASCII: an offset (U+FF01..=U+FF5E to U+21..=U+7E).
        '\u{FF01}'..='\u{FF5E}' => char::from((u32::from(c) - 0xFEE0) as u8),
        _ => lookup(FOLD, c).copied().unwrap_or(c),
    };
    single(c.to_lowercase()).unwrap_or(c)
}

/// The kana a symbol stands for (`㌔` → `キロ`), if it is one.
pub(crate) fn symbol_kana(c: char) -> Option<&'static str> {
    lookup(KANA, c).copied()
}

/// A kana with a (semi-)voiced mark as its base kana and the spacing mark
/// (`が` → `か`, `゛`); `None` for any other char.
pub(crate) fn split_voiced(c: char) -> Option<(char, char)> {
    let i = VOICED.binary_search_by_key(&c, |&(kana, ..)| kana);
    i.into_iter()
        .find_map(|i| VOICED.get(i))
        .map(|&(_, base, mark)| (base, mark))
}

/// The value `c` maps to in a table sorted by char.
fn lookup<T>(table: &'static [(char, T)], c: char) -> Option<&'static T> {
    let i = table.binary_search_by_key(&c, |(from, _)| *from);
    i.into_iter().find_map(|i| table.get(i)).map(|(_, to)| to)
}

fn single(mut chars: impl Iterator<Item = char>) -> Option<char> {
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use unicode_normalization::UnicodeNormalization;

    use super::*;

    /// What `fold` did with the full NFKC tables, before `compat.rs`.
    fn nfkc_fold(c: char) -> char {
        let c = single(c.nfkc()).unwrap_or(c);
        let c = match c {
            '\u{3099}' => '゛',
            '\u{309A}' => '゜',
            c => c,
        };
        single(c.to_lowercase()).unwrap_or(c)
    }

    #[test]
    fn folds_width_and_case() {
        assert_eq!(fold('A'), 'a');
        assert_eq!(fold('Ｔ'), 't');
        assert_eq!(fold('６'), '6');
        assert_eq!(fold('－'), '-');
        assert_eq!(fold('ｶ'), 'カ');
        assert_eq!(fold('ﾞ'), '゛');
        assert_eq!(fold('あ'), 'あ');
        // Longer folded forms are left alone.
        assert_eq!(fold('İ'), 'İ');
        assert_eq!(fold('㌔'), '㌔');
        assert_eq!(fold('…'), '…');
        assert_eq!(fold('゛'), '゛');
    }

    /// In ASCII, punctuation and symbols (U+2000 to U+24FF), the CJK symbols
    /// and kana, and the halfwidth and fullwidth forms, the tables fold
    /// exactly as NFKC did. (Elsewhere, such as mathematical letters or
    /// compatibility kanji, chars stay as they are; the data test checks that
    /// no shipped sentence has one.)
    #[test]
    fn folds_as_nfkc_did_in_the_blocks_the_game_types() {
        let blocks = [
            0x0000..0x0080,
            0x2000..0x2500,
            0x3000..0x3100,
            0xFF00..0xFFF0,
        ];
        for c in blocks.into_iter().flatten().filter_map(char::from_u32) {
            assert_eq!(fold(c), nfkc_fold(c), "U+{:04X}", u32::from(c));
        }
    }

    /// Every char, exhaustively: the symbols NFKC turns into kana are
    /// exactly the table's.
    #[test]
    fn symbol_kana_is_nfkc_for_every_char() {
        let kana = |c: char| ('\u{3041}'..='\u{30FF}').contains(&c);
        for c in (0..=u32::from(char::MAX)).filter_map(char::from_u32) {
            let nfkc: String = c.nfkc().collect();
            let expected = (!kana(c) && nfkc != c.to_string() && nfkc.chars().all(kana))
                .then_some(nfkc.as_str());
            assert_eq!(symbol_kana(c), expected, "U+{:04X}", u32::from(c));
        }
    }

    #[test]
    fn voiced_kana_split_as_nfd() {
        let spacing = |c: char| match c {
            '\u{3099}' => '゛',
            '\u{309A}' => '゜',
            c => c,
        };
        for c in ('\u{3041}'..='\u{30FF}').chain(['a', '漢']) {
            let mut nfd = c.nfd();
            let expected = match (nfd.next(), nfd.next(), nfd.next()) {
                (Some(base), Some(mark), None) => Some((base, spacing(mark))),
                _ => None,
            };
            assert_eq!(split_voiced(c), expected, "{c}");
        }
    }

    /// Every char, exhaustively (1.1 M of them, milliseconds): folding never
    /// panics and is idempotent, so the sentence and the keys agree however
    /// often either side is folded.
    #[test]
    fn fold_is_idempotent_for_every_char() {
        for c in (0..=u32::from(char::MAX)).filter_map(char::from_u32) {
            let once = fold(c);
            assert_eq!(fold(once), once, "{c:?} U+{:04X}", u32::from(c));
        }
    }
}
