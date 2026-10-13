//! The sentence data (built by `cargo xtask data` from Tatoeba's exports)
//! and how a sentence is picked from it, ported from upstream's `api.js`.
//!
//! The data is a set of JSON files ([`DataFile`]) the app fetches itself
//! (sans-IO: this module only defines, parses and decides). The [`Index`] is
//! small and fetched first; it names the directory ([`Index::version`])
//! holding the rest:
//!
//! - chunks: [`RawSentence`]s, [`CHUNK_SIZE`] per file. A sentence's
//!   *index* (its position in the data set, not its Tatoeba id) locates it:
//!   chunk `index / CHUNK_SIZE`, entry `index % CHUNK_SIZE`. Sentences are
//!   shuffled, so a chunk holds unrelated ones and the app can play all of
//!   it.
//! - every sentence's [`Readings`].

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::sentence::{Audio, ParseError, Sentence};

/// Bumped when the files change shape, so stale data gets rebuilt.
pub const FORMAT: u32 = 1;

/// Sentences per chunk file.
pub const CHUNK_SIZE: u32 = 50;

/// The data's files.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DataFile {
    /// The [`Index`].
    Index,
    /// Chunk `n` of [`RawSentence`]s.
    Chunk(u32),
    /// The [`Readings`].
    Readings,
}

impl DataFile {
    /// Where the file lives under the data root: the index at the top, the
    /// rest in the directory of the `version` the index names.
    pub fn path(&self, version: &str) -> String {
        match self {
            DataFile::Index => "index.json".into(),
            DataFile::Chunk(n) => format!("{version}/chunks/{n}.json"),
            DataFile::Readings => format!("{version}/readings.json"),
        }
    }

    /// Whether the file changes in place, so caches must check it again:
    /// only the index. The rest changes only with the version directory.
    pub fn changes_in_place(&self) -> bool {
        matches!(self, DataFile::Index)
    }
}

/// Where sentence `index` lives: (chunk file number, position in it).
pub fn locate(index: u32) -> (u32, usize) {
    (index / CHUNK_SIZE, (index % CHUNK_SIZE) as usize)
}

/// What the data holds ([`DataFile::Index`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    /// [`FORMAT`] of the files.
    pub format: u32,
    /// Directory of the other files ([`DataFile::path`]). Changes with the
    /// content, so caches never mix two builds.
    pub version: String,
    /// When Tatoeba exported the data (HTTP date).
    pub exported: String,
    /// Number of sentences; indices are `0..sentences`.
    pub sentences: u32,
    /// Translation languages, by name.
    pub languages: Vec<Language>,
    /// Sentences with a recording.
    pub audio: Ids,
}

/// Sorted sentence indices, stored as the gaps between them (`[3, 1, 1, 5]`
/// for 3, 4, 5, 10): runs of neighbours become runs of `1`, which compress
/// to almost nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ids(pub Vec<u32>);

impl Serialize for Ids {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut previous = 0;
        let gaps: Vec<u32> = self
            .0
            .iter()
            .map(|&id| {
                let gap = id - previous;
                previous = id;
                gap
            })
            .collect();
        gaps.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Ids {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut sum = 0u32;
        Vec::<u32>::deserialize(deserializer)?
            .into_iter()
            .map(|gap| {
                sum = sum
                    .checked_add(gap)
                    .ok_or_else(|| serde::de::Error::custom("sentence index overflow"))?;
                Ok(sum)
            })
            .collect::<Result<_, _>>()
            .map(Ids)
    }
}

/// [`DataFile::Readings`]: each sentence's distinct readings (`漢字|かんじ`), as
/// positions in a table of all readings, most frequent first so the
/// common ones take the fewest digits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readings {
    pub readings: Vec<String>,
    /// By sentence index.
    pub sentences: Vec<Vec<u32>>,
}

impl Readings {
    /// From each sentence's readings, in index order.
    pub fn new<'a>(sentences: impl IntoIterator<Item = &'a [String]>) -> Self {
        let sentences: Vec<&[String]> = sentences.into_iter().collect();
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for readings in &sentences {
            for reading in readings.iter().collect::<BTreeSet<_>>() {
                *counts.entry(reading).or_default() += 1;
            }
        }
        let mut table: Vec<(&str, usize)> = counts.into_iter().collect();
        // Most frequent first; ties alphabetically, for a stable output.
        table.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let position: BTreeMap<&str, u32> =
            table.iter().zip(0..).map(|(&(r, _), i)| (r, i)).collect();
        Readings {
            sentences: sentences
                .iter()
                .map(|readings| {
                    let ids: BTreeSet<u32> =
                        readings.iter().map(|r| position[r.as_str()]).collect();
                    ids.into_iter().collect()
                })
                .collect(),
            readings: table.into_iter().map(|(r, _)| r.to_string()).collect(),
        }
    }
}

/// One entry of a chunk file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSentence {
    /// Tatoeba sentence id.
    pub id: u32,
    /// e.g. `[今日|きょう]は…`.
    pub transcription: String,
    /// By Tatoeba language code.
    #[serde(default)]
    pub translations: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<RawAudio>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawAudio {
    /// Tatoeba audio id.
    pub id: u32,
    /// The speaker's Tatoeba username.
    pub author: String,
}

impl RawSentence {
    pub fn into_sentence(self) -> Result<Sentence, ParseError> {
        let audio = self.audio.map(|a| Audio {
            id: a.id.to_string(),
            author: a.author,
        });
        Sentence::parse(
            Some(&self.id.to_string()),
            &self.transcription,
            self.translations,
            audio,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Language {
    /// Tatoeba code, e.g. `eng`.
    pub lang: String,
    /// English name, e.g. `English`.
    pub name: String,
}

/// What the player asked for, as far as choosing sentences goes.
#[derive(Debug, Clone, Copy)]
pub struct Filter<'a> {
    /// Only sentences with a recording.
    pub audio: bool,
    /// "New kanji": only sentences with exactly this many readings outside
    /// `known_readings`. `None` means any.
    pub unknown_readings: Option<u8>,
    pub known_readings: &'a BTreeSet<String>,
}

/// The data a [`Filter`] needs, as fetched by the app.
#[derive(Debug, Clone, Copy)]
pub struct Indices<'a> {
    pub index: &'a Index,
    /// Required when `Filter::unknown_readings`.
    pub readings: Option<&'a Readings>,
}

/// Indices matching `filter`, sorted (upstream's `getSentenceIds`).
pub fn candidates(indices: Indices, filter: Filter) -> Vec<u32> {
    // Known readings by position in the readings table.
    let known: Vec<bool> = indices
        .readings
        .map(|r| &r.readings)
        .into_iter()
        .flatten()
        .map(|reading| filter.known_readings.contains(reading))
        .collect();
    // From all sentences whatever the translation language (one without it
    // shows the English translation), or those with a recording (sorted).
    let all = (!filter.audio).then_some(0..indices.index.sentences);
    let recorded = filter.audio.then(|| indices.index.audio.0.iter().copied());
    all.into_iter()
        .flatten()
        .chain(recorded.into_iter().flatten())
        .filter(|&i| {
            let Some(wanted) = filter.unknown_readings else {
                return true;
            };
            // A sentence's readings are distinct already.
            let readings = indices.readings.and_then(|r| r.sentences.get(i as usize));
            let unknown = readings
                .into_iter()
                .flatten()
                .filter(|&&r| !known.get(r as usize).copied().unwrap_or(false))
                .count();
            unknown == usize::from(wanted)
        })
        .collect()
}

/// Which sentences have a recording ([`Index::audio`]), from the chunks'
/// sentences in index order.
pub fn with_audio<'a>(sentences: impl IntoIterator<Item = &'a RawSentence>) -> Ids {
    Ids((0..)
        .zip(sentences)
        .filter_map(|(i, raw)| raw.audio.is_some().then_some(i))
        .collect())
}

/// The languages the sentences have translations in ([`Index::languages`]).
pub fn translated_languages<'a>(
    sentences: impl IntoIterator<Item = &'a RawSentence>,
) -> BTreeSet<&'a str> {
    sentences
        .into_iter()
        .flat_map(|raw| raw.translations.keys().map(String::as_str))
        .collect()
}

/// The translation language for a browser's preferred languages
/// (`navigator.languages`, BCP 47 tags such as `de-AT`): the first one
/// `languages` has, by primary language subtag. `tag` gives a Tatoeba
/// code's BCP 47 tag (`cmn` → `zh`). Japanese is passed over: the
/// sentences are Japanese already.
pub fn browser_language<'a>(
    preferred: &[String],
    languages: &'a [Language],
    tag: impl Fn(&str) -> String,
) -> Option<&'a str> {
    let tags: Vec<(String, &str)> = languages
        .iter()
        .filter(|l| l.lang != "jpn")
        .map(|l| (tag(&l.lang).to_ascii_lowercase(), l.lang.as_str()))
        .collect();
    preferred.iter().find_map(|p| {
        let primary = p.split('-').next()?.to_ascii_lowercase();
        tags.iter()
            .find(|(tag, _)| *tag == primary)
            .map(|&(_, code)| code)
    })
}

/// Random picks without repeats until 90% of the candidates were seen, then
/// starts over (upstream's `makeIdRandomizer`). Randomness comes from the
/// caller as a float in `[0, 1)`, like `Math.random()`.
#[derive(Debug, Clone)]
pub struct Picker {
    candidates: Vec<u32>,
    seen: HashSet<u32>,
}

impl Picker {
    /// `candidates` sorted, as [`candidates`] returns them.
    pub fn new(candidates: Vec<u32>) -> Self {
        debug_assert!(candidates.is_sorted());
        Picker {
            candidates,
            seen: HashSet::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// A random pick ([`Picker::next`]) and its chunk number, followed by
    /// the chunk's other unseen candidates in random order, now seen too:
    /// once a chunk is fetched for one pick, the rest of it is free.
    pub fn next_batch(&mut self, mut random: impl FnMut() -> f64) -> Option<(u32, Vec<u32>)> {
        let pick = self.next(&mut random)?;
        let n = locate(pick).0;
        // Candidates are sorted: the chunk is one slice of them.
        let start = self.candidates.partition_point(|&i| i < n * CHUNK_SIZE);
        let end = self
            .candidates
            .partition_point(|&i| i < (n + 1) * CHUNK_SIZE);
        let mut rest: Vec<u32> = self.candidates[start..end]
            .iter()
            .copied()
            .filter(|&i| self.seen.insert(i))
            .collect();
        // Fisher–Yates.
        for i in (1..rest.len()).rev() {
            let j = ((random() * (i + 1) as f64) as usize).min(i);
            rest.swap(i, j);
        }
        Some((n, [pick].into_iter().chain(rest).collect()))
    }

    /// `None` when there are no candidates.
    pub fn next(&mut self, mut random: impl FnMut() -> f64) -> Option<u32> {
        if self.candidates.is_empty() {
            return None;
        }
        if self.seen.len() * 10 >= self.candidates.len() * 9 {
            self.seen.clear();
        }
        loop {
            let i = (random() * self.candidates.len() as f64) as usize;
            let pick = self.candidates[i.min(self.candidates.len() - 1)];
            if self.seen.insert(pick) {
                return Some(pick);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TestError, TestResult};
    use serde_json::json;

    /// Deterministic stand-in for `Math.random()`.
    fn lcg(seed: u64) -> impl FnMut() -> f64 {
        let mut state = seed;
        move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    #[test]
    fn parses_a_chunk_entry() -> TestResult {
        let chunk = json!([{
            "id": 4705,
            "transcription": "[今日|きょう]は[誕生|たん|じょう][日|び]です！",
            "translations": { "eng": "Today is my birthday!" },
            "audio": { "id": 1682, "author": "yomi" },
        }]);
        let raw: Vec<RawSentence> = serde_json::from_value(chunk)?;
        let s = raw[0].clone().into_sentence()?;
        assert_eq!(s.id.as_deref(), Some("4705"));
        assert_eq!(s.text(), "今日は誕生日です！");
        assert_eq!(s.translations["eng"], "Today is my birthday!");
        assert_eq!(s.audio.as_ref().map(|a| a.author.as_str()), Some("yomi"));
        assert_eq!(s.readings(), ["今日|きょう", "誕生|たんじょう", "日|び"]);
        Ok(())
    }

    #[test]
    fn entries_without_translations_or_audio_parse() -> TestResult {
        let raw: RawSentence =
            serde_json::from_value(json!({ "id": 1, "transcription": "はい。" }))?;
        let s = raw.into_sentence()?;
        assert!(s.translations.is_empty() && s.audio.is_none());
        Ok(())
    }

    #[test]
    fn locates_sentences_in_chunks() {
        assert_eq!(locate(0), (0, 0));
        assert_eq!(locate(49), (0, 49));
        assert_eq!(locate(1234), (24, 34));
    }

    #[test]
    fn filters_like_upstream() {
        let index = Index {
            format: FORMAT,
            version: String::new(),
            exported: String::new(),
            sentences: 4,
            languages: vec![],
            audio: Ids(vec![1, 3]),
        };
        let per_sentence = [
            vec![],
            vec!["日|に".to_string()],
            vec!["日|に".to_string(), "本|ほん".to_string()],
            vec!["本|ほん".to_string(), "本|ほん".to_string()],
        ];
        let readings = Readings::new(per_sentence.iter().map(Vec::as_slice));
        let known: BTreeSet<String> = ["日|に".to_string()].into();
        let indices = Indices {
            index: &index,
            readings: Some(&readings),
        };
        let any = Filter {
            audio: false,
            unknown_readings: None,
            known_readings: &known,
        };
        assert_eq!(candidates(indices, any), [0, 1, 2, 3]);
        assert_eq!(candidates(indices, Filter { audio: true, ..any }), [1, 3]);
        // Unknown readings are counted once each ("本|ほん" twice is one).
        let one = Filter {
            unknown_readings: Some(1),
            ..any
        };
        assert_eq!(candidates(indices, one), [2, 3]);
        let none = Filter {
            unknown_readings: Some(0),
            ..any
        };
        assert_eq!(candidates(indices, none), [0, 1]);
        assert_eq!(
            candidates(
                indices,
                Filter {
                    audio: true,
                    ..none
                }
            ),
            [1]
        );
    }

    #[test]
    fn picker_does_not_repeat_until_ninety_percent_seen() -> TestResult {
        let mut picker = Picker::new((0..20).collect());
        let mut random = lcg(7);
        let first: Vec<u32> = (0..18)
            .map(|_| picker.next(&mut random))
            .collect::<Option<_>>()
            .ok_or(TestError::Missing("a pick"))?;
        let distinct: HashSet<u32> = first.iter().copied().collect();
        assert_eq!(distinct.len(), 18);
        // 18 of 20 seen = 90%: the next pick starts a new round.
        assert!(picker.next(&mut random).is_some());
        assert_eq!(Picker::new(vec![]).next(&mut random), None);
        Ok(())
    }

    #[test]
    fn ids_are_stored_as_gaps() -> TestResult {
        let ids = Ids(vec![3, 4, 5, 10]);
        assert_eq!(serde_json::to_value(&ids)?, json!([3, 1, 1, 5]));
        assert_eq!(serde_json::from_value::<Ids>(json!([3, 1, 1, 5]))?, ids);
        assert_eq!(serde_json::from_value::<Ids>(json!([0]))?, Ids(vec![0]));
        assert!(serde_json::from_value::<Ids>(json!([u32::MAX, 1])).is_err());
        Ok(())
    }

    #[test]
    fn readings_are_interned_by_frequency() {
        let per_sentence: [Vec<String>; 2] = [
            vec!["本|ほん".into(), "日|に".into(), "本|ほん".into()],
            vec!["日|に".into()],
        ];
        let r = Readings::new(per_sentence.iter().map(Vec::as_slice));
        assert_eq!(r.readings, ["日|に", "本|ほん"]);
        assert_eq!(r.sentences, [vec![0, 1], vec![0]]);
    }

    #[test]
    fn picker_hands_out_whole_chunks() {
        let mut picker = Picker::new(vec![1, 7, 49, 50, 99, 100]);
        let mut random = lcg(3);
        let mut chunks = BTreeMap::new();
        while let Some((n, batch)) = picker.next_batch(&mut random) {
            let (pick, mut rest) = (batch[0], batch[1..].to_vec());
            assert_eq!(locate(pick).0, n);
            rest.push(pick);
            rest.sort();
            // Each chunk comes once per round, with all its candidates.
            if chunks.insert(n, rest).is_some() {
                break;
            }
        }
        assert_eq!(
            chunks,
            [(0, vec![1, 7, 49]), (1, vec![50, 99]), (2, vec![100])].into()
        );
    }

    #[test]
    fn lists_follow_the_chunks() {
        let raw = |translations: &[&str], audio: bool| RawSentence {
            id: 1,
            transcription: "はい。".into(),
            translations: translations
                .iter()
                .map(|l| (l.to_string(), "…".into()))
                .collect(),
            audio: audio.then(|| RawAudio {
                id: 1,
                author: "yomi".into(),
            }),
        };
        let sentences = [raw(&["eng"], true), raw(&["deu", "eng"], false)];
        assert_eq!(with_audio(&sentences), Ids(vec![0]));
        assert_eq!(translated_languages(&sentences), ["deu", "eng"].into());
    }

    #[test]
    fn the_browser_language_is_matched_by_its_primary_subtag() {
        let languages: Vec<Language> = ["cmn", "deu", "eng", "jpn", "yue"]
            .into_iter()
            .map(|lang| Language {
                lang: lang.into(),
                name: String::new(),
            })
            .collect();
        // As the browser canonicalizes them.
        let tag = |code: &str| match code {
            "cmn" => "zh".to_string(),
            "deu" => "de".to_string(),
            "eng" => "en".to_string(),
            "jpn" => "ja".to_string(),
            code => code.to_string(),
        };
        let pick = |preferred: &[&str]| {
            let preferred: Vec<String> = preferred.iter().map(|p| p.to_string()).collect();
            browser_language(&preferred, &languages, tag).map(str::to_string)
        };
        assert_eq!(pick(&["de-AT", "en"]).as_deref(), Some("deu"));
        assert_eq!(pick(&["zh-TW"]).as_deref(), Some("cmn"));
        assert_eq!(pick(&["yue-HK"]).as_deref(), Some("yue"));
        // Japanese is passed over, then the next one taken.
        assert_eq!(pick(&["ja-JP", "en-US"]).as_deref(), Some("eng"));
        assert_eq!(pick(&["ja"]), None);
        assert_eq!(pick(&["pt-BR"]), None);
    }

    #[test]
    fn picker_tolerates_random_returning_one() {
        let mut picker = Picker::new(vec![5, 6]);
        assert_eq!(picker.next(|| 1.0), Some(6));
    }
}
