//! Standard romaji input: everything Google's IME (Mozc) accepts, the
//! matcher accepts too. The table is vendored in `tests/data/`.

use wantype_core::Matcher;

/// A Mozc entry that carries keys over, but no vowel completes them.
#[derive(Debug, thiserror::Error)]
#[error("Mozc has no completion for {0}")]
struct NoCompletion(String);

/// (typed keys, output, keys carried over to the next output)
fn mozc() -> Vec<(&'static str, &'static str, &'static str)> {
    include_str!("data/mozc-romanji-hiragana.tsv")
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.split('\t');
            let keys = fields.next().unwrap_or_default();
            let output = fields.next().unwrap_or_default();
            let pending = fields.next().unwrap_or_default();
            (keys, output, pending)
        })
        .collect()
}

/// What typing `keys` must produce, as (text, keys) pairs: entries with
/// carried-over keys (`kk` → っ + `k`) are completed with a vowel so the
/// carried keys produce something (`kka` → っか).
fn cases() -> Result<Vec<(String, String)>, NoCompletion> {
    let table = mozc();
    let lookup = |keys: &str| {
        table
            .iter()
            .find(|(k, _, pending)| *k == keys && pending.is_empty())
            .map(|(_, output, _)| *output)
    };
    table
        .iter()
        // `www` → `w`: Mozc's way to type a literal w, not Japanese.
        .filter(|(_, output, _)| !output.is_ascii())
        .map(|&(keys, output, pending)| {
            if pending.is_empty() {
                return Ok((output.to_string(), keys.to_string()));
            }
            let (vowel, rest) = "aiueo"
                .chars()
                .find_map(|v| lookup(&format!("{pending}{v}")).map(|o| (v, o)))
                .ok_or_else(|| NoCompletion(keys.to_string()))?;
            Ok((format!("{output}{rest}"), format!("{keys}{vowel}")))
        })
        .collect()
}

#[test]
fn every_mozc_spelling_is_accepted() -> Result<(), NoCompletion> {
    let cases = cases()?;
    assert!(cases.len() > 300, "table not loaded");
    let mut rejected = Vec::new();
    for (text, keys) in &cases {
        // Followed by か, as mid-sentence: at the very end of a sentence the
        // first key that completes it wins ("n" for ん, so "nn" can't).
        let (text, keys) = (format!("{text}か"), format!("{keys}ka"));
        let mut m = Matcher::new(&text);
        let accepted: String = keys.chars().take_while(|&c| m.input(c)).collect();
        if accepted != keys || !m.completed() {
            rejected.push(format!("{keys} → {text} (accepted {accepted:?})"));
        }
    }
    assert!(
        rejected.is_empty(),
        "{} of {} standard spellings rejected:\n{}",
        rejected.len(),
        cases.len(),
        rejected.join("\n")
    );
    Ok(())
}
