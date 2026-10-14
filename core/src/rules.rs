//! Which keyboard inputs type the kana at the cursor.
//!
//! The table started as upstream `matcher.js`'s regexes turned into plain
//! Rust data (`/^[しシ](?=[ゃャ])/` became `r("しシ", &["ゃャ"], …)`) and is
//! now held to standard IME romaji: Google's IME table (Mozc, vendored in
//! `tests/data`) is the reference, tested in both directions. Small kana
//! that combine with the kana before them (ゃ, ぃ, …) are in [`SMALL_KANA`],
//! with their combined spelling (しゃ = "sh" + "a") apart from their
//! standalone ones ("xya").
//!
//! A context rule *adds* spellings to the kana's general ones, so small kana
//! can be typed separately (しゃ as "shi" + "xya"); only ん's context rules
//! replace them, since a plain "n" before a vowel reads as な-row.

use std::collections::HashMap;
use std::sync::LazyLock;

use wana_kana::utils::{is_char_kana, katakana_to_hiragana};

use crate::chars::split_voiced;

/// One romaji rule: applies when the char at the cursor is one of `kana` and
/// the following chars match `next` (the i-th following char must be one of
/// `next[i]`). `romaji` lists the accepted keyboard spellings in order.
#[derive(Debug)]
pub(crate) struct Rule {
    pub kana: &'static str,
    pub next: &'static [&'static str],
    pub romaji: &'static [&'static str],
    /// A context rule whose spellings replace the general ones.
    pub exclusive: bool,
}

impl Rule {
    /// `rest[0]` is one of `self.kana` (guaranteed by the index); does the
    /// lookahead hold for the chars after it?
    fn matches(&self, rest: &[char]) -> bool {
        self.next.len() < rest.len()
            && self
                .next
                .iter()
                .zip(&rest[1..])
                .all(|(set, c)| set.contains(*c))
    }
}

const fn r(
    kana: &'static str,
    next: &'static [&'static str],
    romaji: &'static [&'static str],
) -> Rule {
    Rule {
        kana,
        next,
        romaji,
        exclusive: false,
    }
}

/// A context rule whose spellings are the only ones allowed.
const fn only(
    kana: &'static str,
    next: &'static [&'static str],
    romaji: &'static [&'static str],
) -> Rule {
    Rule {
        kana,
        next,
        romaji,
        exclusive: true,
    }
}

/// Order matters: the first matching context rule applies, so a rule with a
/// longer lookahead must precede a shorter one for the same kana; the
/// general rule (no `next`) comes last. Within a rule the first spelling is
/// the one the hint shows: Hepburn (shi, chi, tsu), which learners know from
/// textbooks, before Kunrei (si, ti, tu), and "nn" for ん, as Japanese typing
/// games show it.
#[rustfmt::skip]
pub(crate) const ROMAJI_RULES: &[Rule] = &[
    r("きキ", &["ゃャぃィゅュぇェょョ"], &["ky"]),
    r("ぎギ", &["ゃャぃィゅュぇェょョ"], &["gy"]),
    r("にニ", &["ゃャぃィゅュぇェょョ"], &["ny"]),
    r("みミ", &["ゃャぃィゅュぇェょョ"], &["my"]),
    r("りリ", &["ゃャぃィゅュぇェょョ"], &["ry"]),
    r("ひヒ", &["ゃャぃィゅュぇェょョ"], &["hy"]),
    r("びビ", &["ゃャぃィゅュぇェょョ"], &["by"]),
    r("ぴピ", &["ゃャぃィゅュぇェょョ"], &["py"]),

    r("ふフ", &["ぃィぇェ"], &["f", "hw"]),
    r("ふフ", &["ぁァぉォ"], &["f", "hw"]),
    r("ふフ", &["ゅュ"], &["fy", "hwy"]),
    r("ふフ", &["ゃャょョ"], &["fy"]),

    r("いイ", &["ぇェ"], &["y"]),

    r("くク", &["ぁァぃィぇェぉォ"], &["q", "kw"]),
    r("くク", &["ぅゥ"], &["kw"]),
    r("ぐグ", &["ぁァぃィぅゥぇェぉォ"], &["gw"]),
    r("すス", &["ぁァぃィぅゥぇェぉォ"], &["sw"]),
    r("ずズ", &["ぁァぃィぅゥぇェぉォ"], &["zw"]),

    r("しシ", &["ゃャゅュぇェょョ"], &["sh", "sy"]),
    r("しシ", &["ぃィ"], &["sy"]),

    r("じジ", &["ゃャゅュぇェょョ"], &["j", "zy", "jy"]),
    r("じジ", &["ぃィ"], &["zy", "jy"]),

    r("ちチ", &["ゃャゅュぇェょョ"], &["ch", "ty", "cy"]),
    r("ちチ", &["ぃィ"], &["ty", "cy"]),

    r("ぢヂ", &["ゃャぃィゅュぇェょョ"], &["dy"]),

    r("うウ", &["ぃィぇェ"], &["w", "wh"]),
    r("うウ", &["ぁァぉォ"], &["wh"]),

    // Upstream lists katakana ヴ only; hiragana ゔ is the same sound.
    r("ヴゔ", &["ぃィぇェ"], &["v", "vy"]),
    r("ヴゔ", &["ぁァぉォ"], &["v"]),
    r("ヴゔ", &["ゃャゅュょョ"], &["vy"]),

    r("てテ", &["ぃィ"], &["th", "t'"]),
    r("てテ", &["ゅュ"], &["th", "t'y"]),
    r("てテ", &["ゃャぇェょョ"], &["th"]),

    r("でデ", &["ぃィ"], &["dh", "d'"]),
    r("でデ", &["ゅュ"], &["dh", "d'y"]),
    r("でデ", &["ゃャぇェょョ"], &["dh"]),

    r("つツ", &["ぁァぃィぇェぉォ"], &["ts"]),

    r("とト", &["ぅゥ"], &["tw", "t'"]),
    r("とト", &["ぁァぃィぇェぉォ"], &["tw"]),
    r("どド", &["ぅゥ"], &["dw", "d'"]),
    r("どド", &["ぁァぃィぇェぉォ"], &["dw"]),

    r("あア", &[], &["a"]),
    r("いイ", &[], &["i"]),
    r("うウ", &[], &["u", "wu", "whu"]),
    r("えエ", &[], &["e"]),
    r("おオ", &[], &["o"]),

    // ca/cu/co/qu: not in upstream; common in Japanese typing games.
    r("かカ", &[], &["ka", "ca"]),
    r("がガ", &[], &["ga"]),
    r("きキ", &[], &["ki"]),
    r("ぎギ", &[], &["gi"]),
    r("くク", &[], &["ku", "cu", "qu"]),
    r("ぐグ", &[], &["gu"]),
    r("けケ", &[], &["ke"]),
    r("げゲ", &[], &["ge"]),
    r("こコ", &[], &["ko", "co"]),
    r("ごゴ", &[], &["go"]),

    r("さサ", &[], &["sa"]),
    r("ざザ", &[], &["za"]),
    r("しシ", &[], &["shi", "si", "ci"]),
    r("じジ", &[], &["ji", "zi"]),
    r("すス", &[], &["su"]),
    r("ずズ", &[], &["zu"]),
    r("せセ", &[], &["se", "ce"]),
    r("ぜゼ", &[], &["ze"]),
    r("そソ", &[], &["so"]),
    r("ぞゾ", &[], &["zo"]),

    r("たタ", &[], &["ta"]),
    r("だダ", &[], &["da"]),
    r("ちチ", &[], &["chi", "ti"]),
    r("ぢヂ", &[], &["di"]),
    r("つツ", &[], &["tsu", "tu"]),
    r("づヅ", &[], &["du"]),
    r("てテ", &[], &["te"]),
    r("でデ", &[], &["de"]),
    r("とト", &[], &["to"]),
    r("どド", &[], &["do"]),

    r("なナ", &[], &["na"]),
    r("にニ", &[], &["ni"]),
    r("ぬヌ", &[], &["nu"]),
    r("ねネ", &[], &["ne"]),
    r("のノ", &[], &["no"]),

    r("はハ", &[], &["ha"]),
    r("ばバ", &[], &["ba"]),
    r("ぱパ", &[], &["pa"]),
    r("ひヒ", &[], &["hi"]),
    r("びビ", &[], &["bi"]),
    r("ぴピ", &[], &["pi"]),
    r("ふフ", &[], &["fu", "hu"]),
    r("ぶブ", &[], &["bu"]),
    r("ぷプ", &[], &["pu"]),
    r("へヘ", &[], &["he"]),
    r("べベ", &[], &["be"]),
    r("ぺペ", &[], &["pe"]),
    r("ほホ", &[], &["ho"]),
    r("ぼボ", &[], &["bo"]),
    r("ぽポ", &[], &["po"]),

    r("まマ", &[], &["ma"]),
    r("みミ", &[], &["mi"]),
    r("むム", &[], &["mu"]),
    r("めメ", &[], &["me"]),
    r("もモ", &[], &["mo"]),

    r("やヤ", &[], &["ya"]),
    r("ゆユ", &[], &["yu"]),
    r("よヨ", &[], &["yo"]),

    r("らラ", &[], &["ra"]),
    r("りリ", &[], &["ri"]),
    r("るル", &[], &["ru"]),
    r("れレ", &[], &["re"]),
    r("ろロ", &[], &["ro"]),

    r("わワ", &[], &["wa"]),
    r("ゐヰ", &[], &["wyi"]),
    r("ゑヱ", &[], &["wye"]),
    r("をヲ", &[], &["wo"]),

    r("ヴゔ", &[], &["vu"]),

    // Before a vowel, the な-row, the や-row or another ん a plain "n" would
    // read as な/にゃ/ん… (upstream only handles vowels and the な-row). Upstream also accepts a
    // redundant "nn"/"n'" before other kana through a special case in the
    // matcher; here they are simply spellings of ん, as in IMEs.
    only("んン", &["あいうえおアイウエオなにぬねのナニヌネノやゆよヤユヨんン"], &["nn", "n'", "xn"]),
    r("んン", &[], &["n", "nn", "n'", "xn"]),

    r("ー", &[], &["-"]),

    // Doubled consonants (っか = "kka") are derived from the next kana's
    // spellings in `steps`, as IMEs do, instead of upstream's per-row rules.
    r("っッ", &[], &["xtu", "ltu", "xtsu", "ltsu"]),

    r("ゎヮ", &[], &["xwa", "lwa"]),
    r("ヵ", &[], &["xka", "lka"]),
    r("ヶ", &[], &["xke", "lke"]),

    // Punctuation as a Japanese IME types it (Microsoft IME and Mozc
    // defaults), plus Mozc's z-keys. Fullwidth and halfwidth forms (！, （,
    // ｢…) fold to their plain form first, and ASCII text is typed as is.
    r("、", &[], &[","]),
    r("。", &[], &["."]),
    r("・", &[], &["/", "z/"]),
    r("「", &[], &["["]),
    r("」", &[], &["]"]),
    r("〜", &[], &["~", "z-"]),
    r("…", &[], &["z."]),
    r("‥", &[], &["z,"]),
    r("『", &[], &["z["]),
    r("』", &[], &["z]"]),
    r("←", &[], &["zh"]),
    r("↓", &[], &["zj"]),
    r("↑", &[], &["zk"]),
    r("→", &[], &["zl"]),

    // No direct IME key (they need conversion): the nearest key.
    r("—―", &[], &["-"]),
    r("“”", &[], &["\""]),
];

/// A small kana that combines with the kana before it.
#[derive(Debug)]
pub(crate) struct Small {
    pub kana: &'static str,
    /// Its spelling after the previous kana's combining spelling
    /// (ゃ in しゃ: "sh" + "a").
    pub combined: &'static str,
    /// Its spellings when typed on its own.
    pub alone: &'static [&'static str],
}

const fn small(
    kana: &'static str,
    combined: &'static str,
    alone: &'static [&'static str],
) -> Small {
    Small {
        kana,
        combined,
        alone,
    }
}

#[rustfmt::skip]
pub(crate) const SMALL_KANA: &[Small] = &[
    small("ぁァ", "a", &["xa", "la"]),
    small("ぃィ", "i", &["xi", "li", "xyi", "lyi"]),
    small("ぅゥ", "u", &["xu", "lu"]),
    small("ぇェ", "e", &["xe", "le", "xye", "lye"]),
    small("ぉォ", "o", &["xo", "lo"]),
    small("ゃャ", "a", &["xya", "lya"]),
    small("ゅュ", "u", &["xyu", "lyu"]),
    small("ょョ", "o", &["xyo", "lyo"]),
];

/// Romaji rules grouped by leading kana, in table order.
static ROMAJI_INDEX: LazyLock<HashMap<char, Vec<&'static Rule>>> = LazyLock::new(|| {
    let mut index: HashMap<char, Vec<&'static Rule>> = HashMap::new();
    for rule in ROMAJI_RULES {
        for kana in rule.kana.chars() {
            index.entry(kana).or_default().push(rule);
        }
    }
    index
});

static SMALL_INDEX: LazyLock<HashMap<char, &'static Small>> = LazyLock::new(|| {
    SMALL_KANA
        .iter()
        .flat_map(|small| small.kana.chars().map(move |kana| (kana, small)))
        .collect()
});

/// A small kana that combines with the kana before it ([`SMALL_KANA`]).
pub(crate) fn is_small(kana: char) -> bool {
    SMALL_INDEX.contains_key(&kana)
}

/// How the previous kana was spelled, which decides how a small kana that
/// follows it may be spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The previous kana was spelled on its own: a small kana must be typed
    /// on its own too ("shi" + "xya", "ne" + "xe" for ねぇ), as in IMEs.
    Free,
    /// The previous kana started a combination (し as "sh"): the small kana
    /// completes it ("a").
    Combined,
    /// The previous kana was っ typed as a doubled consonant: this kana's
    /// spelling must start with that letter ("k" + "ka").
    Before(char),
}

/// One way to type the kana at the cursor, and the mode it leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Step {
    pub keys: String,
    pub then: Mode,
}

/// Direct kana input as typed on a JIS kana keyboard: the base hiragana,
/// then the separate (spacing) dakuten/handakuten key if any (`ガ` → `か゛`).
pub(crate) fn kana_keys(c: char) -> Option<String> {
    if !is_char_kana(c) {
        return None;
    }
    let mut keys = String::new();
    for c in katakana_to_hiragana(&c.to_string()).chars() {
        match split_voiced(c) {
            Some((base, mark)) => keys.extend([base, mark]),
            None => keys.push(c),
        }
    }
    Some(keys)
}

/// The ways to type `rest[0]` given how the previous kana was spelled:
/// romaji spellings first (context, then general), then direct kana input,
/// then the char itself. Order is preserved (the first decides the hint) and
/// duplicates are dropped.
pub(crate) fn steps(rest: &[char], mode: Mode) -> Vec<Step> {
    let Some(&c) = rest.first() else {
        return Vec::new();
    };
    let mut found: Vec<Step> = Vec::new();
    let mut push = |keys: &str, then: Mode| {
        if !found.iter().any(|s| s.keys == keys && s.then == then) {
            found.push(Step {
                keys: keys.to_string(),
                then,
            });
        }
    };

    if matches!(c, 'っ' | 'ッ') {
        // Doubled consonant, as in IMEs: the first letter of a spelling of
        // the next kana, which must then follow ("kka" for っか), except n
        // ("nn" is ん) and vowels. Before "ch" also "t" ("tchi" for っち).
        for next in steps(&rest[1..], Mode::Free) {
            let Some(first) = next.keys.chars().next() else {
                continue;
            };
            // Hepburn writes っち as "tchi", so that comes first.
            if next.keys.starts_with("ch") {
                push("t", Mode::Before('c'));
            }
            if first.is_ascii_alphabetic() && !"aiueon".contains(first) {
                push(&first.to_string(), Mode::Before(first));
            }
        }
    }
    if let Some(small) = SMALL_INDEX.get(&c) {
        if mode == Mode::Combined {
            push(small.combined, Mode::Free);
        } else {
            small.alone.iter().for_each(|keys| push(keys, Mode::Free));
        }
    } else if let Some(rules) = ROMAJI_INDEX.get(&c) {
        let context = rules
            .iter()
            .find(|rule| !rule.next.is_empty() && rule.matches(rest));
        if let Some(rule) = context {
            rule.romaji
                .iter()
                .for_each(|keys| push(keys, Mode::Combined));
        }
        if context.is_none_or(|rule| !rule.exclusive)
            && let Some(general) = rules.iter().find(|rule| rule.next.is_empty())
        {
            general
                .romaji
                .iter()
                .for_each(|keys| push(keys, Mode::Free));
        }
    }
    if let Some(keys) = kana_keys(c) {
        push(&keys, Mode::Free);
    }
    push(&c.to_string(), Mode::Free);
    if let Mode::Before(first) = mode {
        found.retain(|step| step.keys.starts_with(first));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_is_well_formed() {
        for rule in ROMAJI_RULES {
            assert!(!rule.kana.is_empty(), "{rule:?}");
            assert!(!rule.romaji.is_empty(), "{rule:?}");
            assert!(rule.next.iter().all(|set| !set.is_empty()), "{rule:?}");
            // Not necessarily ASCII: quotes also accept `«`/`»` (AZERTY etc.).
            assert!(rule.romaji.iter().all(|s| !s.is_empty()), "{rule:?}");
            assert!(!rule.exclusive || !rule.next.is_empty(), "{rule:?}");
            let mut unique = rule.romaji.to_vec();
            unique.sort();
            unique.dedup();
            assert_eq!(
                unique.len(),
                rule.romaji.len(),
                "duplicate spelling: {rule:?}"
            );
        }
        for small in SMALL_KANA {
            assert!(!small.alone.is_empty(), "{small:?}");
            assert!(
                ROMAJI_INDEX.keys().all(|k| !small.kana.contains(*k)),
                "{small:?}"
            );
        }
    }

    /// Rules are looked up by folded chars, so a rule for a char that folds
    /// to something else (`！` → `!`) could never apply.
    #[test]
    fn every_rule_char_is_folded() {
        let chars = ROMAJI_RULES
            .iter()
            .flat_map(|rule| {
                rule.kana
                    .chars()
                    .chain(rule.next.iter().flat_map(|n| n.chars()))
            })
            .chain(SMALL_KANA.iter().flat_map(|small| small.kana.chars()));
        for c in chars {
            assert_eq!(
                crate::chars::fold(c),
                c,
                "{c:?} folds to {:?}",
                crate::chars::fold(c)
            );
        }
    }

    /// `earlier` wins over `later` whenever `later` would match: its
    /// lookahead is no longer and each of its sets covers `later`'s.
    fn shadows(earlier: &Rule, later: &Rule) -> bool {
        earlier.next.len() <= later.next.len()
            && earlier
                .next
                .iter()
                .zip(later.next)
                .all(|(e, l)| l.chars().all(|c| e.contains(c)))
    }

    #[test]
    fn every_rule_is_reachable() {
        for (i, later) in ROMAJI_RULES.iter().enumerate() {
            let reachable = later.kana.chars().any(|kana| {
                !ROMAJI_RULES[..i]
                    .iter()
                    .any(|earlier| earlier.kana.contains(kana) && shadows(earlier, later))
            });
            assert!(
                reachable,
                "rule {i} is shadowed by an earlier rule: {later:?}"
            );
        }
    }

    #[test]
    fn kana_input_decomposes_dakuten() {
        assert_eq!(kana_keys('が').as_deref(), Some("か゛"));
        assert_eq!(kana_keys('パ').as_deref(), Some("は゜"));
        assert_eq!(kana_keys('ん').as_deref(), Some("ん"));
        assert_eq!(kana_keys('ヴ').as_deref(), Some("う゛"));
        assert_eq!(kana_keys('a'), None);
    }

    fn keys(rest: &str, mode: Mode) -> Vec<String> {
        let rest: Vec<char> = rest.chars().collect();
        steps(&rest, mode).into_iter().map(|s| s.keys).collect()
    }

    #[test]
    fn context_spellings_come_first_and_general_ones_stay() {
        assert_eq!(
            keys("しゃ", Mode::Free),
            ["sh", "sy", "shi", "si", "ci", "し"]
        );
        assert_eq!(keys("し", Mode::Free), ["shi", "si", "ci", "し"]);
        // ん before a vowel: no plain "n".
        assert_eq!(keys("んい", Mode::Free), ["nn", "n'", "xn", "ん"]);
        assert!(keys("", Mode::Free).is_empty());
    }

    #[test]
    fn small_kana_follow_the_previous_spelling() {
        assert_eq!(keys("ゃ", Mode::Combined), ["a", "ゃ"]);
        assert_eq!(keys("ゃ", Mode::Free), ["xya", "lya", "ゃ"]);
        assert_eq!(keys("ぇ", Mode::Free), ["xe", "le", "xye", "lye", "ぇ"]);
    }
}

#[cfg(test)]
mod standard {
    //! The other direction of `tests/mozc.rs`: every romaji spelling the
    //! rules accept for kana is standard, i.e. Google's IME (Mozc, table
    //! vendored in `tests/data`) turns it into the same kana. Punctuation is
    //! left out: Mozc handles `/` → ・ in its symbol settings, not this table,
    //! and dashes have no direct key.

    use super::*;

    /// (typed keys, output, keys carried over to the next output)
    fn mozc() -> Vec<(&'static str, &'static str, &'static str)> {
        include_str!("../tests/data/mozc-romanji-hiragana.tsv")
            .lines()
            .map(|line| {
                let mut fields = line.split('\t');
                let mut next = || fields.next().unwrap_or_default();
                (next(), next(), next())
            })
            .collect()
    }

    /// Mozc's conversion: the longest matching entry, its carried-over keys
    /// put back in front of the rest; unmatched keys pass through.
    fn convert(table: &[(&str, &str, &str)], keys: &str) -> String {
        let mut out = String::new();
        let mut rest = keys.to_string();
        while let Some(c) = rest.chars().next() {
            let entry = table
                .iter()
                .filter(|(k, _, _)| rest.starts_with(k))
                .max_by_key(|(k, _, _)| k.len());
            rest = match entry {
                Some((k, output, carried)) => {
                    out.push_str(output);
                    format!("{carried}{}", &rest[k.len()..])
                }
                None => {
                    out.push(c);
                    rest[c.len_utf8()..].to_string()
                }
            };
        }
        out
    }

    /// Every all-ASCII key sequence the rules accept for `text`.
    fn spellings(text: &[char], pos: usize, mode: Mode, typed: String, out: &mut Vec<String>) {
        if pos == text.len() {
            out.push(typed);
            return;
        }
        for step in steps(&text[pos..], mode) {
            if step.keys.is_ascii() {
                spellings(text, pos + 1, step.then, typed.clone() + &step.keys, out);
            }
        }
    }

    #[test]
    fn every_accepted_kana_spelling_is_standard() {
        let table = mozc();
        let mut kana: Vec<char> = ROMAJI_INDEX
            .keys()
            .chain(SMALL_INDEX.keys())
            .copied()
            .filter(|&c| wana_kana::utils::is_char_hiragana(c) || c == 'ー')
            .collect();
        kana.sort();
        // Each kana alone, followed by each small kana, and after っ and ん.
        let mut texts: Vec<String> = kana.iter().map(char::to_string).collect();
        for &k in &kana {
            texts.extend(
                SMALL_INDEX
                    .keys()
                    .filter(|&&s| wana_kana::utils::is_char_hiragana(s))
                    .map(|s| format!("{k}{s}")),
            );
            texts.push(format!("っ{k}"));
            texts.push(format!("ん{k}"));
        }
        let mut nonstandard = Vec::new();
        for text in &texts {
            let chars: Vec<char> = text.chars().collect();
            let mut accepted = Vec::new();
            spellings(&chars, 0, Mode::Free, String::new(), &mut accepted);
            for keys in accepted {
                let converted = convert(&table, &keys);
                if converted != *text {
                    nonstandard.push(format!("{keys} for {text} (an IME gives {converted})"));
                }
            }
        }
        assert!(
            nonstandard.is_empty(),
            "non-standard spellings:\n{}",
            nonstandard.join("\n")
        );
    }
}
