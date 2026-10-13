//! Almost-correct keys: the first key of a spelling that follows the sound
//! instead of the writing, which no IME accepts (`wa` for the particle は,
//! `geemu` for ゲーム, `nee` for ねぇ). The key still counts as a miss, so the
//! IME habit is what the player learns; the frontend explains the spelling.

use crate::rules::{is_small, steps, Mode};

/// Which spelling rule the key missed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlipKind {
    /// は, へ, を: typed as written, whether or not they are the particle
    /// said わ, え, お. Nothing in the text tells the particle from the は in
    /// はな, so the explanation has to hold for both.
    Particle,
    /// ー: typed with its own key, not the vowel it lengthens again.
    LongVowel,
    /// A small kana (ぇ in ねぇ): typed as such, not as its full-size one.
    Small,
}

/// A key that spelled `kana` by its sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slip {
    pub kind: SlipKind,
    pub kana: char,
    /// How `kana` is typed: the spelling the hint shows.
    pub typed: String,
    /// How the kana it sounds like is typed (`wa` for は).
    pub said: String,
}

/// The slip `key` makes at `text[pos]`, where no key of that kana has been
/// typed yet and `typed` is its hinted spelling.
pub(crate) fn detect(text: &[char], pos: usize, key: char, typed: &str) -> Option<Slip> {
    let kana = *text.get(pos)?;
    let (kind, alike) = match kana {
        'は' => (SlipKind::Particle, 'わ'),
        'へ' => (SlipKind::Particle, 'え'),
        'を' => (SlipKind::Particle, 'お'),
        'ー' => {
            let vowel = "aiueo".find(vowel(&text[..pos])?)?;
            (SlipKind::LongVowel, "あいうえお".chars().nth(vowel)?)
        }
        // Each small kana sits just before its full-size one in Unicode.
        _ if is_small(kana) => (SlipKind::Small, char::from_u32(kana as u32 + 1)?),
        _ => return None,
    };
    let said = spelling(alike)?;
    said.starts_with(key).then(|| Slip {
        kind,
        kana,
        typed: typed.into(),
        said,
    })
}

/// The preferred romaji for `kana` on its own.
fn spelling(kana: char) -> Option<String> {
    steps(&[kana], Mode::Free)
        .into_iter()
        .map(|step| step.keys)
        .find(|keys| keys.is_ascii())
}

/// The vowel a ー after `before` lengthens: that of the last kana, looking
/// through earlier ー. `None` after ん, っ or anything not kana.
fn vowel(before: &[char]) -> Option<char> {
    let &last = before.iter().rev().find(|&&c| c != 'ー')?;
    if matches!(last, 'っ' | 'ッ') {
        return None;
    }
    spelling(last)?
        .chars()
        .last()
        .filter(|c| "aiueo".contains(*c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, pos: usize, key: char) -> Option<(SlipKind, String)> {
        let chars: Vec<char> = text.chars().collect();
        detect(&chars, pos, key, "").map(|slip| (slip.kind, slip.said))
    }

    fn slip(kind: SlipKind, said: &str) -> Option<(SlipKind, String)> {
        Some((kind, said.into()))
    }

    #[test]
    fn particles_typed_as_said() {
        assert_eq!(at("わたしは", 3, 'w'), slip(SlipKind::Particle, "wa"));
        assert_eq!(at("どこへ", 2, 'e'), slip(SlipKind::Particle, "e"));
        assert_eq!(at("ほんを", 2, 'o'), slip(SlipKind::Particle, "o"));
        assert_eq!(at("わたしは", 3, 'x'), None);
        // Katakana ハ is never the particle.
        assert_eq!(at("ハ", 0, 'w'), None);
    }

    #[test]
    fn long_vowels_typed_as_the_vowel_again() {
        assert_eq!(at("ゲーム", 1, 'e'), slip(SlipKind::LongVowel, "e"));
        assert_eq!(at("すごーい", 2, 'o'), slip(SlipKind::LongVowel, "o"));
        assert_eq!(at("シャー", 2, 'a'), slip(SlipKind::LongVowel, "a"));
        assert_eq!(at("ゲーー", 2, 'e'), slip(SlipKind::LongVowel, "e"));
        assert_eq!(at("ゲーム", 1, 'a'), None);
        // No vowel to repeat.
        assert_eq!(at("ンー", 1, 'n'), None);
        assert_eq!(at("ッー", 1, 'u'), None);
        assert_eq!(at("ー", 0, 'a'), None);
    }

    #[test]
    fn small_kana_typed_full_size() {
        assert_eq!(at("ねぇ", 1, 'e'), slip(SlipKind::Small, "e"));
        assert_eq!(at("キャ", 1, 'y'), slip(SlipKind::Small, "ya"));
        assert_eq!(at("ねぇ", 1, 'n'), None);
    }
}
