//! Levels and score (級): the helpers (furigana, translation, key hint)
//! fade one by one as the player types clean sentences, and come back on
//! offer after a rough one. Turning a helper off raises the score's
//! multiplier; keeping a faded one on gives that back. Zen (禅) plays
//! without any of this.

use serde::{Deserialize, Serialize};

/// How the game is played, picked on the first run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Levels and score (級).
    Score,
    /// Just type (禅): no score, no levels, helpers as set.
    Zen,
}

/// The levels, each fading one more helper.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    /// 入門: every helper.
    #[default]
    Nyumon,
    /// 初級: no key hint.
    Shokyu,
    /// 中級: no translation.
    Chukyu,
    /// 上級: no furigana; numbers in kanji.
    Jokyu,
    /// 無心: nothing shown while typing.
    Mushin,
}

impl Tier {
    pub const ALL: [Tier; 5] = [
        Tier::Nyumon,
        Tier::Shokyu,
        Tier::Chukyu,
        Tier::Jokyu,
        Tier::Mushin,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn next(self) -> Option<Tier> {
        Tier::ALL.get(self.index() + 1).copied()
    }

    pub fn japanese(self) -> &'static str {
        ["入門", "初級", "中級", "上級", "無心"][self.index()]
    }

    pub fn english(self) -> &'static str {
        ["Entry", "Beginner", "Intermediate", "Advanced", "No-mind"][self.index()]
    }

    /// The helper this level fades, if one.
    pub fn fades(self) -> Option<Helper> {
        Helper::ALL.into_iter().find(|h| h.fades_at() == self)
    }

    /// Every helper is gone at this level.
    pub fn is_mushin(self) -> bool {
        self == Tier::Mushin
    }

    /// Numbers are written in kanji from this level on (unless set).
    pub fn kanji_numerals(self) -> bool {
        self >= Tier::Jokyu
    }

    /// The helpers this level shows: those not faded yet.
    pub fn helpers(self) -> Helpers {
        Helper::ALL
            .into_iter()
            .fold(Helpers::default(), |h, helper| {
                h.with(helper, !helper.faded(self))
            })
    }
}

/// The scored helpers, in the order they fade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Helper {
    Hint,
    Translation,
    Furigana,
}

impl Helper {
    pub const ALL: [Helper; 3] = [Helper::Hint, Helper::Translation, Helper::Furigana];

    /// The level from which it is off unless brought back.
    pub fn fades_at(self) -> Tier {
        match self {
            Helper::Hint => Tier::Shokyu,
            Helper::Translation => Tier::Chukyu,
            Helper::Furigana => Tier::Jokyu,
        }
    }

    pub fn faded(self, tier: Tier) -> bool {
        tier >= self.fades_at()
    }
}

/// Which helpers are on (shown), or were at some point of a sentence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Helpers {
    pub hint: bool,
    pub translation: bool,
    pub furigana: bool,
}

impl Helpers {
    pub fn get(self, helper: Helper) -> bool {
        match helper {
            Helper::Hint => self.hint,
            Helper::Translation => self.translation,
            Helper::Furigana => self.furigana,
        }
    }

    pub fn with(mut self, helper: Helper, on: bool) -> Helpers {
        match helper {
            Helper::Hint => self.hint = on,
            Helper::Translation => self.translation = on,
            Helper::Furigana => self.furigana = on,
        }
        self
    }

    pub fn union(self, other: Helpers) -> Helpers {
        Helpers {
            hint: self.hint || other.hint,
            translation: self.translation || other.translation,
            furigana: self.furigana || other.furigana,
        }
    }

    pub fn off_count(self) -> usize {
        Helper::ALL.into_iter().filter(|&h| !self.get(h)).count()
    }
}

/// Numbers in the interface: arabic, or positional kanji digits (一二四〇)
/// as vertical typesetting writes them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Numerals {
    /// Kanji from 上級 on.
    #[default]
    Auto,
    Arabic,
    Kanji,
}

/// The player's standing, saved with the settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Ladder {
    pub tier: Tier,
    /// Clean sentences toward the next level, of [`PIPS`].
    pub pips: u8,
    /// Clean sentences in a row.
    pub streak: u32,
    pub score: u64,
    /// The level is chosen in the options and stays: no climbing, no
    /// dropping back.
    pub fixed: bool,
}

/// Clean sentences from one level to the next.
pub const PIPS: u8 = 5;
/// Accuracy of a clean sentence: at most one miss in twenty keys.
pub const CLEAN: f64 = 0.95;
/// Below this a sentence was rough: the streak and the level's progress
/// start over, and the last faded helper is offered back.
pub const ROUGH: f64 = 0.8;

/// How a sentence went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    Clean,
    Fair,
    Rough,
}

impl Grade {
    pub fn of(accuracy: f64) -> Grade {
        if accuracy >= CLEAN {
            Grade::Clean
        } else if accuracy >= ROUGH {
            Grade::Fair
        } else {
            Grade::Rough
        }
    }
}

/// The score's multiplier in half steps: ×1, plus ×0.5 for each helper
/// off, plus ×0.5 at 無心.
pub fn multiplier(helpers: Helpers, tier: Tier) -> f64 {
    let halves = helpers.off_count() + usize::from(tier.is_mushin());
    1.0 + 0.5 * halves as f64
}

/// Points for a sentence: five per correct key, weighed by accuracy and
/// the multiplier, in tens.
pub fn points(keystrokes: usize, accuracy: f64, multiplier: f64) -> u64 {
    round_ten(keystrokes as f64 * 5.0 * accuracy * multiplier)
}

/// Bonus for reaching `tier`.
pub fn tier_bonus(tier: Tier) -> u64 {
    100 * tier.index() as u64
}

fn round_ten(x: f64) -> u64 {
    ((x / 10.0).round() * 10.0).max(0.0) as u64
}

/// A completed sentence, as the result row shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub accuracy: f64,
    pub wpm: f64,
    /// Scored: not in Zen, nor a retry of a sentence already completed.
    pub counted: bool,
    pub recorded: Recorded,
}

impl Ladder {
    /// Record a completed sentence; returns what changed. `helpers` were on
    /// at some point while it was typed.
    pub fn record(&mut self, accuracy: f64, keystrokes: usize, helpers: Helpers) -> Recorded {
        let mut r = Recorded::unscored(accuracy, helpers, self.tier);
        r.gain = points(keystrokes, accuracy, r.multiplier);
        match r.grade {
            Grade::Clean => {
                self.streak += 1;
                if !self.fixed {
                    self.pips += 1;
                    if self.pips >= PIPS {
                        self.pips = 0;
                        // At the top, the pips go round again.
                        if let Some(next) = self.tier.next() {
                            self.tier = next;
                            r.gain += tier_bonus(next);
                            r.tier_up = Some(next);
                        }
                    }
                }
            }
            Grade::Fair => self.streak = 0,
            Grade::Rough => {
                self.streak = 0;
                if !self.fixed {
                    self.pips = 0;
                    // The helper that faded last and is off now.
                    r.offer = Helper::ALL
                        .into_iter()
                        .filter(|h| h.faded(self.tier) && !helpers.get(*h))
                        .max_by_key(|h| h.fades_at());
                }
            }
        }
        self.score += r.gain;
        r
    }
}

/// What [`Ladder::record`] changed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Recorded {
    pub grade: Grade,
    pub multiplier: f64,
    /// Points added, the level's bonus included.
    pub gain: u64,
    /// The level reached with this sentence.
    pub tier_up: Option<Tier>,
    /// The helper offered back after a rough sentence.
    pub offer: Option<Helper>,
}

impl Recorded {
    /// A sentence's grade and multiplier, with nothing recorded (Zen, a
    /// retry).
    pub fn unscored(accuracy: f64, helpers: Helpers, tier: Tier) -> Recorded {
        Recorded {
            grade: Grade::of(accuracy),
            multiplier: multiplier(helpers, tier),
            gain: 0,
            tier_up: None,
            offer: None,
        }
    }
}

/// `n` in positional kanji digits: 1240 → 一二四〇, 1.5 → 一・五.
pub fn kanji_digits(n: &str) -> String {
    const DIGITS: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
    n.chars()
        .map(|c| match c.to_digit(10) {
            Some(d) => DIGITS[d as usize],
            None if c == '.' => '・',
            None => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestResult;

    const ALL_ON: Helpers = Helpers {
        hint: true,
        translation: true,
        furigana: true,
    };

    #[test]
    fn each_level_fades_one_more_helper() {
        let faded = |t: Tier| {
            Helper::ALL
                .into_iter()
                .filter(|h| h.faded(t))
                .collect::<Vec<_>>()
        };
        assert_eq!(faded(Tier::Nyumon), []);
        assert_eq!(faded(Tier::Shokyu), [Helper::Hint]);
        assert_eq!(faded(Tier::Chukyu), [Helper::Hint, Helper::Translation]);
        assert_eq!(faded(Tier::Jokyu), Helper::ALL);
        assert_eq!(faded(Tier::Mushin), Helper::ALL);
        assert_eq!(Tier::Chukyu.fades(), Some(Helper::Translation));
        assert_eq!(Tier::Mushin.fades(), None);
    }

    #[test]
    fn the_multiplier_counts_helpers_off_and_mushin() {
        assert_eq!(multiplier(ALL_ON, Tier::Nyumon), 1.0);
        let no_hint = Helpers {
            hint: false,
            ..ALL_ON
        };
        assert_eq!(multiplier(no_hint, Tier::Shokyu), 1.5);
        assert_eq!(multiplier(Helpers::default(), Tier::Jokyu), 2.5);
        assert_eq!(multiplier(Helpers::default(), Tier::Mushin), 3.0);
    }

    #[test]
    fn five_clean_sentences_climb_a_level_with_a_bonus() {
        let mut ladder = Ladder::default();
        for _ in 0..4 {
            let r = ladder.record(1.0, 40, ALL_ON);
            assert_eq!((r.grade, r.gain, r.tier_up), (Grade::Clean, 200, None));
        }
        let r = ladder.record(1.0, 40, ALL_ON);
        assert_eq!(r.tier_up, Some(Tier::Shokyu));
        assert_eq!(r.gain, 200 + 100);
        assert_eq!(ladder.tier, Tier::Shokyu);
        assert_eq!((ladder.pips, ladder.streak, ladder.score), (0, 5, 1100));
    }

    #[test]
    fn a_fair_sentence_breaks_the_streak_only() {
        let mut ladder = Ladder::default();
        ladder.record(1.0, 40, ALL_ON);
        let r = ladder.record(0.9, 40, ALL_ON);
        assert_eq!(r.grade, Grade::Fair);
        assert_eq!((ladder.pips, ladder.streak), (1, 0));
    }

    #[test]
    fn a_rough_sentence_starts_the_level_over_and_offers_the_last_helper() {
        let mut ladder = Ladder {
            tier: Tier::Chukyu,
            pips: 3,
            streak: 7,
            ..Ladder::default()
        };
        let none = Helpers {
            furigana: true,
            ..Helpers::default()
        };
        let r = ladder.record(0.7, 40, none);
        assert_eq!(r.grade, Grade::Rough);
        assert_eq!(r.offer, Some(Helper::Translation));
        assert_eq!((ladder.pips, ladder.streak), (0, 0));
        // A helper already back is not offered; the one before it is.
        let translation = Helpers {
            translation: true,
            ..none
        };
        assert_eq!(
            ladder.record(0.7, 40, translation).offer,
            Some(Helper::Hint)
        );
        // Nothing faded at 入門: nothing to offer.
        let mut first = Ladder::default();
        assert_eq!(first.record(0.5, 40, ALL_ON).offer, None);
    }

    #[test]
    fn a_fixed_level_stays() {
        let mut ladder = Ladder {
            tier: Tier::Shokyu,
            fixed: true,
            ..Ladder::default()
        };
        for _ in 0..10 {
            ladder.record(1.0, 40, ALL_ON);
        }
        assert_eq!(
            (ladder.tier, ladder.pips, ladder.streak),
            (Tier::Shokyu, 0, 10)
        );
        assert_eq!(ladder.record(0.5, 40, Helpers::default()).offer, None);
    }

    #[test]
    fn the_top_level_keeps_going() {
        let mut ladder = Ladder {
            tier: Tier::Mushin,
            pips: 4,
            ..Ladder::default()
        };
        let r = ladder.record(1.0, 40, Helpers::default());
        assert_eq!(
            (r.tier_up, ladder.tier, ladder.pips),
            (None, Tier::Mushin, 0)
        );
    }

    #[test]
    fn points_are_in_tens() {
        assert_eq!(points(37, 0.97, 1.5), 270);
        assert_eq!(points(0, 1.0, 3.0), 0);
    }

    #[test]
    fn numbers_turn_to_kanji_digits() {
        assert_eq!(kanji_digits("1240"), "一二四〇");
        assert_eq!(kanji_digits("1.5"), "一・五");
        assert_eq!(kanji_digits("+180"), "+一八〇");
    }

    #[test]
    fn ladders_are_saved_in_camel_case_and_tiers_by_name() -> TestResult {
        let ladder = Ladder {
            tier: Tier::Chukyu,
            pips: 2,
            ..Ladder::default()
        };
        let json = serde_json::to_string(&ladder)?;
        assert!(json.contains("\"tier\":\"chukyu\""), "{json}");
        assert_eq!(serde_json::from_str::<Ladder>(&json)?, ladder);
        Ok(())
    }
}
