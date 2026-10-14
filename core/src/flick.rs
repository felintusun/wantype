//! Flick input (フリック入力) on a phone's 12-key kana pad (ケータイ配列),
//! the way most phones in Japan are typed: each key holds a row of kana, a
//! tap types the first and a flick left, up, right or down the others
//! (あ, い, う, え, お). The ゛゜小 key turns the kana just typed:
//! は → ば → ぱ, つ → っ → づ.
//!
//! The matcher takes kana as a JIS kana keyboard sends them (the base kana,
//! then a spacing ゛ or ゜; small kana as themselves) and checks each as it
//! comes, while a flick keyboard changes the last kana afterwards. So the
//! [`Composer`] holds a kana back while ゛゜小 can still turn it into the
//! one at the cursor (や for ゃ), and passes everything else on at once.

use wana_kana::utils::katakana_to_hiragana;

use crate::chars::{fold, single};
use crate::compat::VOICED;
use crate::matcher::Matcher;
use crate::rules::kana_keys;

/// A key of the pad, in reading order: rows of three ([`PAD`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pad {
    A,
    Ka,
    Sa,
    Ta,
    Na,
    Ha,
    Ma,
    Ya,
    Ra,
    /// ゛゜小: turns the kana just typed.
    Turn,
    Wa,
    /// 、。？！・
    Marks,
}

/// The pad's keys, row by row.
pub const PAD: [Pad; 12] = [
    Pad::A,
    Pad::Ka,
    Pad::Sa,
    Pad::Ta,
    Pad::Na,
    Pad::Ha,
    Pad::Ma,
    Pad::Ya,
    Pad::Ra,
    Pad::Turn,
    Pad::Wa,
    Pad::Marks,
];

/// How a touch leaves its key: a tap, or a flick toward one side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flick {
    Tap,
    Left,
    Up,
    Right,
    Down,
}

impl Flick {
    /// How far a touch travels to flick rather than tap, in CSS pixels.
    pub const REACH: f64 = 24.0;

    /// The flick of a touch that moved `dx` right and `dy` down from
    /// where it began: a tap within [`Flick::REACH`], else the way it went
    /// most.
    pub fn of_motion(dx: f64, dy: f64) -> Flick {
        if dx.hypot(dy) < Flick::REACH {
            Flick::Tap
        } else if dx.abs() > dy.abs() {
            if dx < 0.0 {
                Flick::Left
            } else {
                Flick::Right
            }
        } else if dy < 0.0 {
            Flick::Up
        } else {
            Flick::Down
        }
    }
}

/// The flicks, in the order of [`Pad::chars`].
pub const FLICKS: [Flick; 5] = [
    Flick::Tap,
    Flick::Left,
    Flick::Up,
    Flick::Right,
    Flick::Down,
];

impl Pad {
    /// What each flick types, in [`FLICKS`] order; `None` where the key
    /// has nothing that way. ゛゜小 types nothing of its own: it turns.
    pub fn chars(self) -> [Option<char>; 5] {
        let row = |chars: [char; 5]| chars.map(Some);
        match self {
            Pad::A => row(['あ', 'い', 'う', 'え', 'お']),
            Pad::Ka => row(['か', 'き', 'く', 'け', 'こ']),
            Pad::Sa => row(['さ', 'し', 'す', 'せ', 'そ']),
            Pad::Ta => row(['た', 'ち', 'つ', 'て', 'と']),
            Pad::Na => row(['な', 'に', 'ぬ', 'ね', 'の']),
            Pad::Ha => row(['は', 'ひ', 'ふ', 'へ', 'ほ']),
            Pad::Ma => row(['ま', 'み', 'む', 'め', 'も']),
            Pad::Ya => row(['や', '「', 'ゆ', '」', 'よ']),
            Pad::Ra => row(['ら', 'り', 'る', 'れ', 'ろ']),
            Pad::Turn => [None; 5],
            Pad::Wa => [Some('わ'), Some('を'), Some('ん'), Some('ー'), None],
            Pad::Marks => row(['、', '。', '？', '！', '・']),
        }
    }

    /// What `flick` types on this key ([`Flick`] is declared in [`FLICKS`]
    /// order).
    pub fn char(self, flick: Flick) -> Option<char> {
        self.chars().get(flick as usize).copied().flatten()
    }
}

/// The small kana ゛゜小 turns a kana into first.
fn small(kana: char) -> Option<char> {
    let small = match kana {
        'あ' => 'ぁ',
        'い' => 'ぃ',
        'う' => 'ぅ',
        'え' => 'ぇ',
        'お' => 'ぉ',
        'つ' => 'っ',
        'や' => 'ゃ',
        'ゆ' => 'ゅ',
        'よ' => 'ょ',
        'わ' => 'ゎ',
        _ => return None,
    };
    Some(small)
}

/// What ゛゜小 turns a kana into, in turn: the kana itself, its small form,
/// then with ゛ and ゜ ([`VOICED`]). は → ば → ぱ, つ → っ → づ.
fn turns(kana: char) -> impl Iterator<Item = char> + Clone {
    let voiced = VOICED
        .iter()
        .filter(move |&&(_, base, _)| base == kana)
        .map(|&(voiced, ..)| voiced);
    std::iter::once(kana).chain(small(kana)).chain(voiced)
}

/// Whether ゛゜小 turns `kana` into `to` (or it is `to` already).
fn turns_into(kana: char, to: char) -> bool {
    turns(kana).any(|k| k == to)
}

/// `kana` turned `n` times.
fn turned(kana: char, n: usize) -> char {
    turns(kana).cycle().nth(n).unwrap_or(kana)
}

/// `c` as compared: folded as the matcher folds, katakana as hiragana
/// (the pad has hiragana; katakana are typed with them).
fn plain(c: char) -> char {
    let c = fold(c);
    single(katakana_to_hiragana(&c.to_string()).chars()).unwrap_or(c)
}

/// The matcher's keys for `kana` ([`kana_keys`]); a mark as itself.
fn keys(kana: char) -> String {
    kana_keys(kana).unwrap_or_else(|| kana.to_string())
}

/// A turn with nothing to turn, which the matcher refuses: a wrong key.
const STRAY: char = '゛';

/// Turns gestures on the pad into the matcher's keys ([`Composer::input`]),
/// holding back a kana that ゛゜小 may still have to turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Composer {
    /// The kana held back, and how many times it was turned.
    held: Option<(char, usize)>,
}

impl Composer {
    /// The kana held back, to show as being typed.
    pub fn held(&self) -> Option<char> {
        self.held.map(|(kana, n)| turned(kana, n))
    }

    /// Forget what is held: a new sentence, or a retry.
    pub fn clear(&mut self) {
        self.held = None;
    }

    /// The keys `flick` on `pad` types now, `target` being the kana at the
    /// cursor ([`Event::Flick`](crate::Event::Flick)). Empty
    /// while a kana is held back, or for a flick toward nothing.
    pub fn input(&mut self, pad: Pad, flick: Flick, target: Option<char>) -> String {
        let target = target.map(plain);
        if pad == Pad::Turn {
            let Some((kana, n)) = self.held.take() else {
                return STRAY.to_string();
            };
            let now = turned(kana, n + 1);
            if Some(now) == target {
                return keys(now);
            }
            self.held = Some((kana, n + 1));
            return String::new();
        }
        let Some(c) = pad.char(flick) else {
            return String::new();
        };
        let mut typed = self
            .held
            .take()
            .map(|(kana, n)| keys(turned(kana, n)))
            .unwrap_or_default();
        let c = fold(c);
        match target {
            Some(t) if t != c && turns_into(c, t) => self.held = Some((c, 0)),
            _ => typed.push_str(&keys(c)),
        }
        typed
    }

    /// The gesture that brings `target`, the kana at the cursor, closer:
    /// ゛゜小 for the kana held back, else the key and flick of its row.
    pub fn hint(&self, target: Option<char>) -> Option<(Pad, Flick)> {
        let t = plain(target?);
        if self.held.is_some_and(|(kana, _)| turns_into(kana, t)) {
            return Some((Pad::Turn, Flick::Tap));
        }
        PAD.iter()
            .flat_map(|&pad| FLICKS.map(|flick| (pad, flick)))
            .find(|&(pad, flick)| pad.char(flick).map(fold).is_some_and(|c| turns_into(c, t)))
    }
}

/// Why following the hint does not flick a text to its end.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FlickError {
    #[error("{0:?} is on no key of the pad")]
    NoKey(char),
    #[error("stuck at {0:?}")]
    Stuck(char),
}

/// The gestures that type `text` (kana) by following the hint, or why it
/// cannot be flicked.
pub fn gestures(text: &str) -> Result<Vec<(Pad, Flick)>, FlickError> {
    let mut matcher = Matcher::new(text);
    let mut composer = Composer::default();
    let mut done = Vec::new();
    while let Some(at) = matcher.current_kana() {
        let (pad, flick) = composer.hint(Some(at)).ok_or(FlickError::NoKey(at))?;
        done.push((pad, flick));
        let typed = composer.input(pad, flick, Some(at));
        // Every kana takes at most three gestures (は, ゛, ゜).
        if !typed.chars().all(|key| matcher.input(key)) || done.len() > 3 * matcher.len() {
            return Err(FlickError::Stuck(at));
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use crate::testing::TestResult;

    #[test]
    fn every_kana_and_mark_is_on_the_pad() {
        let all = "あいうえおかきくけこがぎぐげごさしすせそざじずぜぞたちつてとだぢづでど\
                   なにぬねのはひふへほばびぶべぼぱぴぷぺぽまみむめもやゆよらりるれろわをん\
                   ぁぃぅぇぉっゃゅょゎゔ\
                   アイウエオガパッャヴー、。？！「」・";
        for c in all.chars() {
            assert!(Composer::default().hint(Some(c)).is_some(), "{c}");
        }
        assert_eq!(Composer::default().hint(Some('〜')), None);
    }

    #[test]
    fn turns_follow_the_hint() -> TestResult {
        use Flick::*;
        use Pad::*;
        assert_eq!(
            gestures("ぱっちゃ")?,
            [
                (Ha, Tap),
                (Turn, Tap),
                (Turn, Tap),
                (Ta, Up),
                (Turn, Tap),
                (Ta, Left),
                (Ya, Tap),
                (Turn, Tap)
            ]
        );
        // Katakana with the same keys; ー is on わ.
        assert_eq!(gestures("ガー")?, [(Ka, Tap), (Turn, Tap), (Wa, Right)]);
        Ok(())
    }

    #[test]
    fn a_kana_is_held_only_while_it_can_still_turn_into_the_target() {
        let mut c = Composer::default();
        // や for ゃ: held, shown, then turned and passed on.
        assert_eq!(c.input(Pad::Ya, Flick::Tap, Some('ゃ')), "");
        assert_eq!(c.held(), Some('や'));
        assert_eq!(c.input(Pad::Turn, Flick::Tap, Some('ゃ')), "ゃ");
        assert_eq!(c.held(), None);
        // は for ぱ: ば on the way is held too.
        assert_eq!(c.input(Pad::Ha, Flick::Tap, Some('ぱ')), "");
        assert_eq!(c.input(Pad::Turn, Flick::Tap, Some('ぱ')), "");
        assert_eq!(c.held(), Some('ば'));
        assert_eq!(c.input(Pad::Turn, Flick::Tap, Some('ぱ')), "は゜");
        // A right kana goes at once, a wrong one too.
        assert_eq!(c.input(Pad::Sa, Flick::Tap, Some('さ')), "さ");
        assert_eq!(c.input(Pad::Ka, Flick::Tap, Some('さ')), "か");
        // Another key passes on what was held, as it stands.
        assert_eq!(c.input(Pad::Ka, Flick::Tap, Some('が')), "");
        assert_eq!(c.input(Pad::Sa, Flick::Tap, Some('が')), "かさ");
        // A turn with nothing held is a wrong key; a flick toward nothing
        // types nothing.
        assert_eq!(c.input(Pad::Turn, Flick::Tap, Some('か')), "゛");
        assert_eq!(c.input(Pad::Wa, Flick::Down, Some('か')), "");
    }

    #[test]
    fn a_touch_flicks_the_way_it_went_most() {
        assert_eq!(Flick::of_motion(3.0, -5.0), Flick::Tap);
        assert_eq!(Flick::of_motion(-40.0, 10.0), Flick::Left);
        assert_eq!(Flick::of_motion(5.0, -30.0), Flick::Up);
        assert_eq!(Flick::of_motion(30.0, 29.0), Flick::Right);
        assert_eq!(Flick::of_motion(0.0, 24.0), Flick::Down);
    }

    #[test]
    fn the_fixtures_can_be_flicked() -> TestResult {
        for s in fixtures::sentences()? {
            gestures(&s.kana())?;
        }
        Ok(())
    }
}
