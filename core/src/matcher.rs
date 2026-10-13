//! Matches keyboard input against a kana sentence, keystroke by keystroke.
//!
//! Ported from upstream `matcher.js`, with one structural change: upstream
//! keeps a single list of target spellings and takes the first one a key
//! completes, which makes longer alternatives unreachable ("jya" for じゃ:
//! "j" already completes じ) and rules out typing small kana on their own
//! ("kixyo"). This matcher keeps every spelling that is still possible
//! (a small NFA over the rule table) and only rejects a key that fits none.

use crate::chars::fold;
use crate::rules::{steps, Mode};
use crate::slip::{self, Slip};

/// One way the input typed so far can continue: typing the kana at `pos`
/// with the keys in `rest`, leaving `then` for the kana after it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Path {
    pos: usize,
    rest: String,
    then: Mode,
    /// No key of this kana typed yet.
    fresh: bool,
}

/// Why following the hint does not type a text to its end.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RomajiError {
    /// The hint is a key a romaji keyboard does not have.
    #[error("{at:?} is hinted as {key:?}, not a romaji key")]
    NotAKey { at: Option<char>, key: char },
    /// The hinted key is refused, or the hint goes round in circles.
    #[error("stuck at {at:?} after {typed:?}")]
    Stuck { at: Option<char>, typed: String },
}

#[derive(Debug, Clone)]
pub struct Matcher {
    japanese_text: Vec<char>,
    /// Live paths, most preferred first (that order decides the hint).
    paths: Vec<Path>,
    completed: bool,
    keystrokes: usize,
    current_char_is_romaji: bool,
}

impl Matcher {
    /// `jp` is the kana to type (for a sentence with kanji, its transcription).
    pub fn new(jp: &str) -> Self {
        let mut m = Matcher {
            // `fold` keeps one char per char, so indices into the folded
            // text also index the displayed sentence.
            japanese_text: jp.chars().map(fold).collect(),
            paths: Vec::new(),
            completed: false,
            keystrokes: 0,
            current_char_is_romaji: true,
        };
        m.reset();
        m
    }

    pub fn reset(&mut self) {
        self.keystrokes = 0;
        self.completed = self.japanese_text.is_empty();
        self.paths = self.expand(0, Mode::Free);
    }

    /// Paths for typing the kana at `pos`.
    fn expand(&self, pos: usize, mode: Mode) -> Vec<Path> {
        steps(&self.japanese_text[pos..], mode)
            .into_iter()
            .map(|step| Path {
                pos,
                rest: step.keys,
                then: step.then,
                fresh: true,
            })
            .collect()
    }

    pub fn completed(&self) -> bool {
        self.completed
    }

    /// The single character the player should type next: the next key of
    /// the preferred spelling, or of the preferred kana spelling while the
    /// player is typing kana.
    pub fn hint(&self) -> Option<char> {
        if self.completed {
            return None;
        }
        let first = |path: &Path| path.rest.chars().next();
        if !self.current_char_is_romaji {
            let kana = self.paths.iter().filter_map(first).find(|c| !c.is_ascii());
            if kana.is_some() {
                return kana;
            }
        }
        self.paths.first().and_then(first)
    }

    /// The romaji keys that type `text` by following the hint from start to
    /// end, or why that is impossible on a plain keyboard (a hint that is
    /// not ASCII, or no way to the end).
    pub fn romaji(text: &str) -> Result<String, RomajiError> {
        let mut m = Matcher::new(text);
        let mut typed = String::new();
        while let Some(key) = m.hint() {
            let at = m.japanese_text.get(m.japanese_index()).copied();
            if !key.is_ascii() {
                return Err(RomajiError::NotAKey { at, key });
            }
            // Every accepted key moves a spelling on; this bounds a stuck hint.
            if !m.input(key) || typed.len() > 8 * m.len() {
                return Err(RomajiError::Stuck { at, typed });
            }
            typed.push(key);
        }
        Ok(typed)
    }

    /// Accepted keystrokes only: a rejected key is never recorded (upstream
    /// behaviour, and what the WPM figure counts).
    pub fn keystroke_count(&self) -> usize {
        self.keystrokes
    }

    /// Number of sentence chars typed: the furthest any live spelling got.
    pub fn japanese_index(&self) -> usize {
        if self.completed {
            self.japanese_text.len()
        } else {
            self.paths.iter().map(|p| p.pos).max().unwrap_or(0)
        }
    }

    pub fn len(&self) -> usize {
        self.japanese_text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.japanese_text.is_empty()
    }

    pub fn current_char_is_romaji(&self) -> bool {
        self.current_char_is_romaji
    }

    /// Whether `key`, which [`Matcher::input`] rejected, starts a spelling
    /// by sound at a kana not yet begun ([`Slip`]).
    pub fn slip(&self, key: char) -> Option<Slip> {
        // The next kana's preferred romaji, as the hint shows it.
        let next = self.paths.iter().find(|p| p.fresh && p.rest.is_ascii())?;
        slip::detect(&self.japanese_text, next.pos, fold(key), &next.rest)
    }

    /// Advance the state machine. Returns `false` and leaves progress
    /// unchanged for a wrong key, or for any key once completed. Any key
    /// before completion sets the input mode (romaji or kana) used by
    /// [`Matcher::hint`], as upstream does.
    pub fn input(&mut self, ch: char) -> bool {
        if self.completed {
            return false;
        }
        let ch = fold(ch);
        self.current_char_is_romaji = ch.is_ascii();

        let mut next: Vec<Path> = Vec::new();
        let mut completed = false;
        for path in &self.paths {
            let Some(rest) = path.rest.strip_prefix(ch) else {
                continue;
            };
            if !rest.is_empty() {
                next.push(Path {
                    pos: path.pos,
                    rest: rest.to_string(),
                    then: path.then,
                    fresh: false,
                });
            } else if path.pos + 1 == self.japanese_text.len() {
                completed = true;
            } else {
                next.extend(self.expand(path.pos + 1, path.then));
            }
        }
        if !completed && next.is_empty() {
            return false;
        }
        let mut unique: Vec<Path> = Vec::with_capacity(next.len());
        for path in next {
            if !unique.contains(&path) {
                unique.push(path);
            }
        }
        self.paths = unique;
        self.completed = completed;
        self.keystrokes += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    //! Upstream `matcher.test.js`, test for test, then what this matcher adds.

    use super::*;
    use crate::testing::{TestError, TestResult};

    /// Feed `input` until the first rejected key; returns (matched sentence
    /// prefix, accepted input) like upstream's `match` helper. The prefix is
    /// of the sentence as given (upstream returns its lowercased copy).
    fn matched(jp: &str, input: &str) -> (String, String) {
        let mut m = Matcher::new(jp);
        let mut accepted = String::new();
        for ch in input.chars() {
            if !m.input(ch) {
                break;
            }
            accepted.push(ch);
        }
        let jp_prefix = jp.chars().take(m.japanese_index()).collect();
        (jp_prefix, accepted)
    }

    fn assert_match(jp: &str, input: &str, expected: (&str, &str)) {
        let (a, b) = matched(jp, input);
        assert_eq!((a.as_str(), b.as_str()), expected, "{jp} <- {input}");
    }

    fn assert_full(jp: &str, input: &str) {
        assert_match(jp, input, (jp, input));
    }

    #[test]
    fn n() {
        assert_full("ん", "n");
        assert_full("んか", "nka");
        // Allow redundant nn -> ん
        assert_full("んか", "nnka");
        assert_full("んか", "n'ka");
        assert_match("んい", "ni", ("", "n"));
        assert_full("んい", "nni");
        assert_match("んに", "nni", ("ん", "nn"));
        assert_full("んに", "nnni");
        assert_full("んに", "n'ni");
        assert_full("こんにちは", "kon'nichiha");
        assert_full("さんねんまえににっぽんに", "san'nenmaeninippon'ni");
        // Kana matching
        assert_full("ン", "ん");
        assert_full("ほんとう", "ほんとう");
        assert_full("ピンポン", "ひ゜んほ゜ん");
    }

    #[test]
    fn dakuten() {
        assert_full("にほんご", "にほんこ゛");
        assert_full("ベッド", "へ゛っと゛");
        assert_full("ペラペラ", "へ゜らへ゜ら");
    }

    #[test]
    fn long_vowels() {
        assert_full("かあ", "kaa");
        assert_full("まあマー", "maama-");
        assert_full("とうきょう", "toukyou");
        assert_full("トウキョウ", "toukyou");
        assert_full("とーきょー", "to-kyo-");
        assert_full("トーキョー", "to-kyo-");
        assert_full("アート", "a-to");
    }

    #[test]
    fn useless_stuff() {
        // Upstream accepts "wya" for うゃ; no IME does, so this matches
        // standard input instead: う, then ゃ typed on its own.
        assert_match("うゃうゅうょ", "wyawyuwy", ("", "w"));
        assert_full("うゃ", "uxya");
    }

    #[test]
    fn partial_match() {
        assert_match("じゅ", "j", ("じ", "j"));
        assert_full("じゅ", "ju");
        // Commented out upstream; work here.
        assert_match("じゅ", "jy", ("じ", "jy"));
        assert_full("じゅ", "jyu");
        assert_full("じゅ", "jilyu");
    }

    #[test]
    fn small_tsu() {
        assert_full("いっちゃった", "icchatta");
        assert_full("いっちゃった", "ittyatta");
        assert_full("いっちゃった", "いっちゃった");
        assert_full("ペット", "へ゜っと");
        // `ぇ` is in upstream's lookahead set (the old port dropped it).
        assert_full("っちぇ", "cche");
    }

    #[test]
    fn case_insensitive() {
        assert_full("かれは", "KarEhA");
    }

    #[test]
    fn latin() {
        assert_match("Tatoebaは", "tatOEBAha", ("Tatoebaは", "tatOEBAha"));
        assert_match(
            "Ｔａｔｏｅｂａは",
            "tatOEBAha",
            ("Ｔａｔｏｅｂａは", "tatOEBAha"),
        );
    }

    #[test]
    fn other() {
        assert_full("トゥどぅ", "twudwu");
        assert_full("きょ", "kyo");
        assert_match("きょ", "ky", ("き", "ky"));
        assert_full("しゃ", "sha");
        assert_match("しゃ", "sh", ("し", "sh"));
        assert_full("シャー", "sha-");
        assert_match("じゃあ", "jaa", ("じゃあ", "jaa"));
        assert_match("じゃあ", "j", ("じ", "j"));
        assert_full("ファミリ", "famiri");
        assert_full("ティッシュ", "thisshu");
        assert_match("ティッシュ", "th", ("テ", "th"));
        assert_full("デューティー", "dhu-thi-");
        assert_full("スターウォーズ", "suta-who-zu");
        assert_full("ウェーター", "we-ta-");
        assert_match("ウェ", "w", ("ウ", "w"));
        assert_full("オリーヴ", "ori-vu");
        assert_full("ヴァージニア", "va-jinia");
        assert_full("ヴォルフガング・サヴァリッシュ", "vorufugangu/savarisshu");
        assert_full("アンリ・ヴュータン", "anri/vyu-tan");
        assert_full("ティーシャツ", "thi-shatsu");
        assert_full("ファイアーフォックス", "faia-fokkusu");
        // Commented out upstream; work here.
        assert_full("ティッシュ", "texixtusixyu");
        assert_full("ジャーナリスト", "jya-narisuto");
        assert_full("ウェーター", "whe-ta-");
        assert_full("ティーシャツ", "teli-shatsu");
    }

    #[test]
    fn punctuation() {
        assert_full("a、b、「foo！？」、x・y。", "a,b,[foo!?],x/y.");
        assert_full("a,b,\"foo!?\",x・y.", "a,b,\"foo!?\",x/y.");
        assert_full("はい。", "hai.");
        assert_full("か!?", "か！？");
        // `＂` and `”` are in upstream's set (the old port lost them).
        assert_full("＂a”", "\"a\"");
    }

    #[test]
    fn hint_romaji_only() {
        let input = "syuppatudekiru";
        let mut m = Matcher::new(input);
        assert!(m.current_char_is_romaji());
        let chars: Vec<char> = input.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            m.input(ch);
            assert!(m.current_char_is_romaji());
            if i + 1 < chars.len() {
                assert_eq!(m.hint(), Some(chars[i + 1]));
            }
        }
    }

    #[test]
    fn hint_kana_only() {
        let input = "しゅっは゜つて゛きる";
        let mut m = Matcher::new(input);
        assert!(m.current_char_is_romaji());
        let chars: Vec<char> = input.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            m.input(ch);
            assert!(!m.current_char_is_romaji());
            if i + 1 < chars.len() {
                assert_eq!(m.hint(), Some(chars[i + 1]));
            }
        }
    }

    #[test]
    fn hint_mixed() {
        let input = "syuppatuて゛きる";
        let mut m = Matcher::new(input);
        assert!(m.current_char_is_romaji());
        for ch in input.chars() {
            m.input(ch);
            assert_eq!(m.current_char_is_romaji(), ch.is_ascii());
        }
    }

    // What this matcher adds over upstream.

    /// Spellings Japanese typists commonly use; upstream rejects all of these.
    #[test]
    fn alternative_spellings() {
        // Listed in upstream's table but unreachable there.
        assert_full("じゃ", "jya");
        assert_full("ふゅ", "fyu");
        assert_full("うぃ", "whi");
        // Small kana typed on their own.
        assert_full("きょ", "kixyo");
        assert_full("しゃ", "shixya");
        assert_full("しゃ", "silya");
        assert_full("ティ", "texi");
        assert_full("ファ", "fuxa");
        // Small っ typed on its own before a consonant.
        assert_full("きって", "kixtute");
        assert_full("きって", "kiltsute");
        // Not in upstream's table at all.
        assert_full("かんじ", "kaxnji");
        assert_full("か", "ca");
        assert_full("く", "qu");
        assert_full("ゔ", "vu");
    }

    #[test]
    fn small_kana_follow_how_the_kana_before_was_typed() {
        // After "shi" (し on its own) ゃ must be typed on its own too:
        // "shia" would read しあ.
        assert_match("しゃ", "shia", ("し", "shi"));
        assert_match("きょ", "kio", ("き", "ki"));
        // After a combining spelling, the small kana completes it.
        assert_match("しゃ", "shxya", ("し", "sh"));
        // Small kana after a kana they don't combine with are typed on their
        // own, as in IMEs ("nee" is ねえ). Upstream also accepted "nee".
        assert_match("ねぇ", "nee", ("ね", "ne"));
        assert_full("ねぇ", "nexe");
        assert_full("ねぇ", "nele");
    }

    #[test]
    fn hint_prefers_the_first_spelling() {
        let mut m = Matcher::new("じゃ");
        assert_eq!(m.hint(), Some('j'));
        m.input('j');
        // じ is done ("j"); the combined ゃ comes next.
        assert_eq!(m.japanese_index(), 1);
        assert_eq!(m.hint(), Some('a'));
    }

    fn hint_path(text: &str) -> TestResult<String> {
        Ok(Matcher::romaji(text)?)
    }

    #[test]
    fn hints_teach_hepburn_and_nn() -> TestResult {
        assert_eq!(hint_path("こんにちは")?, "konnnichiha");
        assert_eq!(hint_path("しんぶん")?, "shinbun");
        assert_eq!(hint_path("つくえ")?, "tsukue");
        assert_eq!(hint_path("ふじさん")?, "fujisan");
        assert_eq!(hint_path("きっちゃてん")?, "kitchaten");
        assert_eq!(hint_path("きって")?, "kitte");
        assert_eq!(hint_path("おかあさんよ")?, "okaasannyo");
        assert_eq!(hint_path("しゃしん")?, "shashin");
        Ok(())
    }

    #[test]
    fn hint_for_fullwidth_and_halfwidth_text_is_the_ascii_key() {
        for (text, hint) in [("Ｔａ", 't'), ("－", '-'), ("６", '6')] {
            assert_eq!(Matcher::new(text).hint(), Some(hint), "{text}");
        }
        assert_full("ｶ", "ka");
        // Typing fullwidth (IME on) still matches ASCII text.
        assert_match("ab", "ａｂ", ("ab", "ａｂ"));
    }

    #[test]
    fn input_after_completion_is_rejected() {
        let mut m = Matcher::new("a");
        assert!(m.input('a'));
        assert!(m.completed());
        assert!(!m.input('a'));
        assert_eq!(m.japanese_index(), 1);
        assert!(m.completed());
        assert_eq!(m.hint(), None);
    }

    #[test]
    fn index_stays_aligned_with_unfolded_text() {
        let mut m = Matcher::new("İか");
        assert_eq!(m.len(), 2);
        assert!(m.input('İ'));
        assert_eq!(m.japanese_index(), 1);
    }

    #[test]
    fn slips_only_at_a_kana_not_yet_begun() -> TestResult {
        let mut m = Matcher::new("こんは");
        for key in "kon".chars() {
            assert!(m.input(key));
        }
        // ん may still take a second n, but は is up next too.
        assert!(!m.input('w'));
        let slip = m.slip('w').ok_or(TestError::Missing("a slip"))?;
        assert_eq!(
            (slip.kind, slip.typed.as_str()),
            (crate::SlipKind::Particle, "ha")
        );
        assert!(m.input('h'));
        assert!(!m.input('w'));
        assert_eq!(m.slip('w'), None);
        Ok(())
    }

    #[test]
    fn empty_text_is_complete() {
        let m = Matcher::new("");
        assert!(m.completed());
        assert_eq!(m.hint(), None);
    }
}
