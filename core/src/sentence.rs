//! Sentences as upstream models them: parts of surface text, kanji parts
//! carrying the kana the player types (shown as furigana).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::lang::{ENGLISH, Lang};

/// Tatoeba's site, in English.
const TATOEBA: &str = "https://tatoeba.org/en";

/// A Tatoeba sentence id (a translation is a sentence too).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SentenceId(pub u32);

impl SentenceId {
    /// The sentence's page on Tatoeba.
    pub fn url(self) -> String {
        format!("{TATOEBA}/sentences/show/{self}")
    }
}

/// A Tatoeba recording id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AudioId(pub u32);

impl AudioId {
    /// The recording's sources, in the order upstream tries them.
    /// (Upstream's second one, `api.tatoeba.org/unstable/audio/<id>/file`,
    /// now answers 404; the stable API serves recordings at `v1/audios`.)
    pub fn urls(self) -> [String; 2] {
        [
            format!("{TATOEBA}/audio/download/{self}"),
            format!("https://api.tatoeba.org/v1/audios/{self}/file"),
        ]
    }
}
impl fmt::Display for SentenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for AudioId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// One displayed unit. Kana and punctuation are one char each with no
/// transcription; a kanji part carries the kana typed for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub text: String,
    pub transcription: Option<String>,
    /// For kanji: the reading this part belongs to, `漢字|かんじ` (the whole
    /// bracket group, not just this char). Identifies a "known reading".
    pub reading: Option<String>,
}

impl Part {
    /// What the player types for this part.
    pub fn kana(&self) -> &str {
        self.transcription.as_deref().unwrap_or(&self.text)
    }
}

/// A native-speaker recording on Tatoeba.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Audio {
    pub id: AudioId,
    pub author: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sentence {
    /// Tatoeba sentence id, if the sentence comes from Tatoeba.
    pub id: Option<SentenceId>,
    pub parts: Vec<Part>,
    /// Translations by Tatoeba language code (`eng`, `fra`, …).
    pub translations: BTreeMap<Lang, String>,
    pub audio: Option<Audio>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("unclosed `[` at byte {at}")]
    UnclosedBracket { at: usize },
    #[error("empty kanji in group at byte {at}")]
    EmptyKanji { at: usize },
    #[error("group at byte {at} has no kana")]
    MissingKana { at: usize },
}

impl Sentence {
    /// Parse Tatoeba's transcription format, e.g. `[私|わたし]は[月給|げっ|きゅう]です。`.
    /// Like upstream's `parseSentenceObject`: one part per plain char; a
    /// group with one kana reading per kanji is split into one part per
    /// kanji, otherwise it stays one part with the readings joined.
    pub fn parse_parts(transcription: &str) -> Result<Vec<Part>, ParseError> {
        let mut parts = Vec::new();
        let mut rest = transcription;
        let mut offset = 0;
        while let Some(open) = rest.find('[') {
            parts.extend(rest[..open].chars().map(plain_part));
            let at = offset + open;
            let after = &rest[open + 1..];
            let close = after.find(']').ok_or(ParseError::UnclosedBracket { at })?;
            let mut fields = after[..close].split('|');
            let kanji = fields.next().unwrap_or_default();
            let kana: Vec<&str> = fields.collect();
            if kanji.is_empty() {
                return Err(ParseError::EmptyKanji { at });
            }
            if kana.is_empty() {
                return Err(ParseError::MissingKana { at });
            }
            let reading = reading_key(kanji, &kana);
            if kana.iter().all(|k| k.is_empty()) {
                parts.extend(unread_group(kanji, &reading));
            } else if kanji.chars().count() == kana.len() {
                // One reading per kanji; an empty one means the previous
                // reading spans this kanji too: [田舎道|いなか||みち].
                if kana[0].is_empty() {
                    return Err(ParseError::MissingKana { at });
                }
                for (k, r) in kanji.chars().zip(&kana) {
                    if r.is_empty() {
                        // The first reading is not empty (checked above), so
                        // this group has a part already.
                        let Some(last) = parts.last_mut() else {
                            return Err(ParseError::MissingKana { at });
                        };
                        last.text.push(k);
                    } else {
                        parts.push(Part {
                            text: k.to_string(),
                            transcription: Some(r.to_string()),
                            reading: Some(reading.clone()),
                        });
                    }
                }
            } else {
                parts.push(Part {
                    text: kanji.to_string(),
                    transcription: Some(kana.concat()),
                    reading: Some(reading),
                });
            }
            let consumed = open + 1 + close + 1;
            rest = &rest[consumed..];
            offset += consumed;
        }
        parts.extend(rest.chars().map(plain_part));
        Ok(parts)
    }

    pub fn parse(
        id: Option<SentenceId>,
        transcription: &str,
        translations: BTreeMap<Lang, String>,
        audio: Option<Audio>,
    ) -> Result<Self, ParseError> {
        Ok(Sentence {
            id,
            parts: Self::parse_parts(transcription)?,
            translations,
            audio,
        })
    }

    /// Distinct readings in order of appearance (consecutive parts of one
    /// group share a reading), like upstream's `readings`.
    pub fn readings(&self) -> Vec<String> {
        let mut readings: Vec<String> = Vec::new();
        for reading in self.parts.iter().filter_map(|p| p.reading.as_ref()) {
            if readings.last() != Some(reading) {
                readings.push(reading.clone());
            }
        }
        readings
    }

    /// The surface text, e.g. `私は月給です。`.
    pub fn text(&self) -> String {
        self.parts.iter().map(|p| p.text.as_str()).collect()
    }

    /// What the player types, e.g. `わたしはげっきゅうです。`.
    pub fn kana(&self) -> String {
        self.parts.iter().map(Part::kana).collect()
    }

    /// The translation into `language`, or else into English. Sentences are
    /// picked whatever the language, and nearly all have an English one.
    pub fn translation(&self, language: &Lang) -> Option<Translation<'_>> {
        [language.as_str(), ENGLISH]
            .into_iter()
            .find_map(|lang| self.translations.get_key_value(lang))
            .map(|(lang, text)| Translation { lang, text })
    }
}

/// A sentence's translation: the language it is in, and its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Translation<'a> {
    pub lang: &'a Lang,
    pub text: &'a str,
}

fn plain_part(c: char) -> Part {
    Part {
        text: c.to_string(),
        transcription: None,
        reading: None,
    }
}

/// A group without a reading, like `[．|]` or `[㌔|]` in upstream's data.
/// Upstream renders such a group as a plain char to type as is, which
/// strands the player on chars no keyboard produces. Symbols whose
/// compatibility form is kana (`㌔` → `キロ`) get that as their reading;
/// anything else stays plain (punctuation is typeable through the rules).
/// The reading key is kept so the readings match upstream's index.
fn unread_group(text: &str, reading: &str) -> Vec<Part> {
    let compat: Option<String> = text
        .chars()
        .map(|c| match crate::chars::symbol_kana(c) {
            Some(kana) => Some(kana.to_string()),
            None => wana_kana::utils::is_char_kana(c).then(|| c.to_string()),
        })
        .collect();
    if let Some(compat) = compat.filter(|kana| kana != text) {
        return vec![Part {
            text: text.to_string(),
            transcription: Some(compat),
            reading: Some(reading.to_string()),
        }];
    }
    text.chars()
        .map(|c| Part {
            reading: Some(reading.to_string()),
            ..plain_part(c)
        })
        .collect()
}

/// Upstream's reading key: the kanji (with chars unsafe in file names
/// replaced by `_`) and the joined kana, e.g. `月給|げっきゅう`.
fn reading_key(kanji: &str, kana: &[&str]) -> String {
    let kanji: String = kanji
        .chars()
        .map(|c| if "/?<>\\:*|\":".contains(c) { '_' } else { c })
        .collect();
    format!("{kanji}|{}", kana.concat())
}

/// Where a typing target is relative to the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Done,
    Active,
    Todo,
}

/// One char of [`Sentence::kana`], i.e. one step of the matcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub ch: char,
    pub progress: Progress,
    /// A wrong key was pressed while this char was active (cleared on reset).
    pub error: bool,
}

/// A part ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Kana or punctuation: the displayed char is itself the target.
    Plain(Target),
    /// Kanji with furigana: progress lives on the reading's chars; `progress`
    /// summarizes them for styling the kanji itself.
    Ruby {
        text: String,
        progress: Progress,
        error: bool,
        /// Upstream's "new kanji" mode: this part's reading is not known yet.
        unknown: bool,
        reading: Vec<Target>,
    },
}

/// Lay out `sentence` for rendering, given how many targets are done, which
/// targets saw a mistake, and which readings count as unknown.
pub fn segments(
    sentence: &Sentence,
    done: usize,
    is_error: impl Fn(usize) -> bool,
    is_unknown: impl Fn(&str) -> bool,
) -> Vec<Segment> {
    let mut index = 0;
    let mut target = |ch: char| {
        let i = index;
        index += 1;
        let progress = match i.cmp(&done) {
            std::cmp::Ordering::Less => Progress::Done,
            std::cmp::Ordering::Equal => Progress::Active,
            std::cmp::Ordering::Greater => Progress::Todo,
        };
        Target {
            ch,
            progress,
            error: is_error(i),
        }
    };
    let mut segments = Vec::new();
    for part in &sentence.parts {
        match &part.transcription {
            // One segment per char, matching what `Sentence::kana` feeds the
            // matcher (parsed sentences only have one-char plain parts).
            None => segments.extend(part.text.chars().map(|c| Segment::Plain(target(c)))),
            Some(kana) => {
                let reading: Vec<Target> = kana.chars().map(&mut target).collect();
                let progress = if reading.iter().all(|t| t.progress == Progress::Done) {
                    Progress::Done
                } else if reading.iter().any(|t| t.progress == Progress::Active) {
                    Progress::Active
                } else {
                    Progress::Todo
                };
                segments.push(Segment::Ruby {
                    text: part.text.clone(),
                    progress,
                    error: reading.iter().any(|t| t.error),
                    unknown: part.reading.as_deref().is_some_and(&is_unknown),
                    reading,
                });
            }
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TestError, TestResult};

    fn part(text: &str, transcription: Option<&str>, reading: Option<&str>) -> Part {
        Part {
            text: text.into(),
            transcription: transcription.map(Into::into),
            reading: reading.map(Into::into),
        }
    }

    fn parse(transcription: &str) -> TestResult<Sentence> {
        Ok(Sentence::parse(None, transcription, BTreeMap::new(), None)?)
    }

    #[test]
    fn translations_fall_back_to_english() -> TestResult {
        let translations = [("deu", "Katze"), ("eng", "Cat")]
            .map(|(lang, text)| (Lang::new(lang), text.to_string()));
        let sentence = Sentence::parse(None, "ねこ", translations.into(), None)?;
        let [deu, eng, fra] = ["deu", "eng", "fra"].map(Lang::new);
        let translation = |lang, text| Some(Translation { lang, text });
        assert_eq!(sentence.translation(&deu), translation(&deu, "Katze"));
        assert_eq!(sentence.translation(&fra), translation(&eng, "Cat"));
        assert_eq!(parse("ねこ")?.translation(&fra), None);
        Ok(())
    }

    #[test]
    fn parses_tatoeba_transcription() -> TestResult {
        let parts = Sentence::parse_parts("[私|わたし]は[月給|げっ|きゅう]です。")?;
        assert_eq!(
            parts,
            [
                part("私", Some("わたし"), Some("私|わたし")),
                part("は", None, None),
                part("月", Some("げっ"), Some("月給|げっきゅう")),
                part("給", Some("きゅう"), Some("月給|げっきゅう")),
                part("で", None, None),
                part("す", None, None),
                part("。", None, None),
            ]
        );
        Ok(())
    }

    #[test]
    fn unsplittable_group_stays_whole() -> TestResult {
        let parts = Sentence::parse_parts("[６月|ろくがつ]")?;
        assert_eq!(
            parts,
            [part("６月", Some("ろくがつ"), Some("６月|ろくがつ"))]
        );
        let parts = Sentence::parse_parts("[一晩|ひと|ばん]")?;
        assert_eq!(
            parts,
            [
                part("一", Some("ひと"), Some("一晩|ひとばん")),
                part("晩", Some("ばん"), Some("一晩|ひとばん"))
            ]
        );
        Ok(())
    }

    #[test]
    fn rejects_malformed_transcriptions() {
        assert_eq!(
            Sentence::parse_parts("あ[私|わたし"),
            Err(ParseError::UnclosedBracket { at: 3 })
        );
        assert_eq!(
            Sentence::parse_parts("[|わ]"),
            Err(ParseError::EmptyKanji { at: 0 })
        );
        assert_eq!(
            Sentence::parse_parts("[私]"),
            Err(ParseError::MissingKana { at: 0 })
        );
        assert_eq!(
            Sentence::parse_parts("[私達||たち]"),
            Err(ParseError::MissingKana { at: 0 })
        );
    }

    #[test]
    fn empty_readings_in_upstream_data() -> TestResult {
        // An empty field: the previous reading spans this kanji too.
        let parts = Sentence::parse_parts("[田舎道|いなか||みち]")?;
        let key = Some("田舎道|いなかみち");
        assert_eq!(
            parts,
            [
                part("田舎", Some("いなか"), key),
                part("道", Some("みち"), key)
            ]
        );
        // No reading at all: typeable symbols stay plain…
        let parts = Sentence::parse_parts("[．|]")?;
        assert_eq!(parts, [part("．", None, Some("．|"))]);
        // …symbols with a kana compatibility form get it as reading.
        let parts = Sentence::parse_parts("[㌔|]")?;
        assert_eq!(parts, [part("㌔", Some("キロ"), Some("㌔|"))]);
        Ok(())
    }

    #[test]
    fn text_and_kana() -> TestResult {
        let s = parse("[旅行|りょ|こう]は[楽|たの]しい")?;
        assert_eq!(s.text(), "旅行は楽しい");
        assert_eq!(s.kana(), "りょこうはたのしい");
        Ok(())
    }

    #[test]
    fn readings_like_upstream() -> TestResult {
        let s = parse("[日|に][本|ほん]の[日本|に|ほん]と[日|ひ]")?;
        assert_eq!(s.readings(), ["日|に", "本|ほん", "日本|にほん", "日|ひ"]);
        // Chars that upstream keeps out of file names become `_`.
        assert_eq!(parse("[A/B|えー]")?.readings(), ["A_B|えー"]);
        Ok(())
    }

    #[test]
    fn segments_track_progress_through_readings() -> TestResult {
        let s = parse("[日|に]は")?;
        let segs = segments(&s, 1, |i| i == 0, |r| r == "日|に");
        assert_eq!(
            segs,
            [
                Segment::Ruby {
                    text: "日".into(),
                    progress: Progress::Done,
                    error: true,
                    unknown: true,
                    reading: vec![Target {
                        ch: 'に',
                        progress: Progress::Done,
                        error: true
                    }],
                },
                Segment::Plain(Target {
                    ch: 'は',
                    progress: Progress::Active,
                    error: false
                }),
            ]
        );

        let s = parse("[旅|りょ]")?;
        let Segment::Ruby { progress, .. } = &segments(&s, 1, |_| false, |_| false)[0] else {
            return Err(TestError::Missing("the segment the test expects"));
        };
        assert_eq!(*progress, Progress::Active);
        Ok(())
    }
}
