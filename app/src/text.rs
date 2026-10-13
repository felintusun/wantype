//! The interface's names, numbers and level texts. From 上級 (or always,
//! per the options) numbers are written in positional kanji digits, as
//! vertical typesetting does: 1240 → 一二四〇.

use wantype_core::ladder::{kanji_digits, Ladder, PIPS};
use wantype_core::{Helper, Numerals, Tier};

/// A helper's English and Japanese label.
pub fn helper_label(helper: Helper) -> (&'static str, &'static str) {
    match helper {
        Helper::Furigana => ("Furigana", "ふりがな"),
        Helper::Translation => ("Translation", "翻訳"),
        Helper::Hint => ("Key hint", "ヒント"),
    }
}

/// A helper's name within a sentence.
pub fn helper_name(helper: Helper) -> String {
    helper_label(helper).0.to_lowercase()
}

/// What a level shows, for the options.
pub fn tier_description(tier: Tier) -> &'static str {
    match tier {
        Tier::Nyumon => "everything shown",
        Tier::Shokyu => "no key hint",
        Tier::Chukyu => "no translation",
        Tier::Jokyu => "no furigana · 漢数字",
        Tier::Mushin => "nothing shown while typing",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Numbers {
    pub kanji: bool,
}

impl Numbers {
    pub fn int(self, n: impl Into<u64>) -> String {
        self.digits(n.into().to_string())
    }

    /// The running score, grouped by thousands in arabic digits.
    pub fn score(self, n: u64) -> String {
        if self.kanji {
            return self.int(n);
        }
        let digits = n.to_string();
        let mut grouped = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                grouped.push(',');
            }
            grouped.push(c);
        }
        grouped
    }

    pub fn multiplier(self, m: f64) -> String {
        let m = format!("{m:.1}");
        match self.kanji {
            true => format!("{}倍", kanji_digits(&m)),
            false => format!("{m}倍"),
        }
    }

    pub fn percent(self, accuracy: f64) -> String {
        let n = self.int((accuracy * 100.0).round() as u64);
        match self.kanji {
            true => format!("{n}％"),
            false => format!("{n}%"),
        }
    }

    /// Speed and accuracy: `42 wpm · 96%`, or `毎分 四二 語 · 正確 九六％`.
    pub fn stats(self, wpm: f64, accuracy: f64) -> String {
        let speed = self.int(wpm.round().max(0.0) as u64);
        match self.kanji {
            true => format!("毎分 {speed} 語 · 正確 {}", self.percent(accuracy)),
            false => format!("{speed} wpm · {}", self.percent(accuracy)),
        }
    }

    pub fn streak(self, n: u32) -> String {
        match self.kanji {
            true => format!("連続 {}", self.int(n)),
            false => format!("streak {n}"),
        }
    }

    fn digits(self, n: String) -> String {
        match self.kanji {
            true => kanji_digits(&n),
            false => n,
        }
    }
}

/// What the next level brings, after a completed sentence.
pub fn progress(ladder: &Ladder, numerals: Numerals) -> String {
    if ladder.fixed {
        return format!(
            "Fixed at {} {} · Options to climb again",
            ladder.tier.japanese(),
            ladder.tier.english()
        );
    }
    let Some(next) = ladder.tier.next() else {
        return "Top level. Keep the streak going.".into();
    };
    let left = PIPS.saturating_sub(ladder.pips);
    let what = match next.fades() {
        Some(helper) => {
            let name = helper_name(helper);
            let starts = next.kanji_numerals() && !ladder.tier.kanji_numerals();
            let kanji = match starts && numerals == Numerals::Auto {
                true => " and numbers turn to 漢数字",
                false => "",
            };
            format!("{name} fades{kanji}")
        }
        None => "everything fades".into(),
    };
    format!(
        "{left} more clean sentence{} to {} {}, where {what}",
        if left == 1 { "" } else { "s" },
        next.japanese(),
        next.english(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARABIC: Numbers = Numbers { kanji: false };
    const KANJI: Numbers = Numbers { kanji: true };

    #[test]
    fn numbers_read_as_designed() {
        assert_eq!(ARABIC.score(1240), "1,240");
        assert_eq!(ARABIC.score(999), "999");
        assert_eq!(ARABIC.score(1_234_567), "1,234,567");
        assert_eq!(KANJI.score(1240), "一二四〇");
        assert_eq!(ARABIC.multiplier(1.5), "1.5倍");
        assert_eq!(KANJI.multiplier(2.5), "二・五倍");
        assert_eq!(ARABIC.stats(41.6, 0.957), "42 wpm · 96%");
        assert_eq!(KANJI.stats(42.0, 0.96), "毎分 四二 語 · 正確 九六％");
        assert_eq!(KANJI.streak(12), "連続 一二");
    }

    #[test]
    fn progress_names_what_the_next_level_fades() {
        let ladder = |tier, pips| Ladder {
            tier,
            pips,
            ..Ladder::default()
        };
        assert_eq!(
            progress(&ladder(Tier::Nyumon, 4), Numerals::Auto),
            "1 more clean sentence to 初級 Beginner, where key hint fades"
        );
        assert_eq!(
            progress(&ladder(Tier::Chukyu, 0), Numerals::Auto),
            "5 more clean sentences to 上級 Advanced, where furigana fades and numbers turn to 漢数字"
        );
        assert_eq!(
            progress(&ladder(Tier::Jokyu, 2), Numerals::Arabic),
            "3 more clean sentences to 無心 No-mind, where everything fades"
        );
        assert_eq!(
            progress(&ladder(Tier::Mushin, 2), Numerals::Auto),
            "Top level. Keep the streak going."
        );
    }
}
