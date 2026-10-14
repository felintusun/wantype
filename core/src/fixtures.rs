//! A small built-in sentence pool: the starter sentence, shown before
//! any data is loaded, and real Tatoeba sentences
//! (<https://tatoeba.org/en/sentences/show/ID>, text CC BY 2.0 FR) from
//! upstream's data, chosen to cover multi-kanji readings, っ, ん, katakana
//! with ー, and `、`. Used by tests.

use std::collections::BTreeMap;

use crate::lang::Lang;
use crate::sentence::{Audio, AudioId, ParseError, Sentence, SentenceId};

struct Fixture {
    id: Option<SentenceId>,
    transcription: &'static str,
    english: &'static str,
    /// (Tatoeba audio id, speaker)
    audio: Option<(AudioId, &'static str)>,
}

const STARTER: Fixture = Fixture {
    id: None,
    transcription: "チャンスは[何度|なん|ど]でもあるワン！",
    english: "Chances come again and again, woof!",
    audio: None,
};

const FIXTURES: &[Fixture] = &[
    STARTER,
    Fixture {
        id: Some(SentenceId(78642)),
        transcription:
            "[来週|らい|しゅう]の[日曜日|にち|よう|び]に[遊|あそ]びに[来|き]てください。",
        english: "Come and see me on Sunday next week.",
        audio: Some((AudioId(12200), "huizi99")),
    },
    Fixture {
        id: Some(SentenceId(4754)),
        transcription: "[急|いそ]いで！",
        english: "Step on it!",
        audio: Some((AudioId(1686), "yomi")),
    },
    Fixture {
        id: Some(SentenceId(6018)),
        transcription: "[暇|ひま]だったら、[手伝|て|つだ]ってください。",
        english: "If you are free, give me a hand.",
        audio: Some((AudioId(1968), "huizi99")),
    },
    Fixture {
        id: Some(SentenceId(78157)),
        transcription: "[旅行|りょ|こう]は[楽|たの]しかったですか。",
        english: "Did you enjoy your trip?",
        audio: Some((AudioId(12197), "Mizu")),
    },
    Fixture {
        id: Some(SentenceId(5064)),
        transcription: "[一晩|ひと|ばん][泊|と]めてもらいたいんだけど。",
        english: "I'd like to stay for one night.",
        audio: Some((AudioId(1690), "yomi")),
    },
    Fixture {
        id: Some(SentenceId(80023)),
        transcription: "[目|め]がゴロゴロします。",
        english: "My eyes feel gritty.",
        audio: Some((AudioId(12204), "Mizu")),
    },
];

impl Fixture {
    fn parse(&self) -> Result<Sentence, ParseError> {
        let audio = self.audio.map(|(id, author)| Audio {
            id,
            author: author.into(),
        });
        let translations = BTreeMap::from([(Lang::english(), self.english.to_string())]);
        Sentence::parse(self.id, self.transcription, translations, audio)
    }
}

/// All fixture sentences, in order.
pub fn sentences() -> Result<Vec<Sentence>, ParseError> {
    FIXTURES.iter().map(Fixture::parse).collect()
}

/// The starter sentence, in Pochi's voice: everyday words, one kanji word
/// to show the furigana, and no punctuation to find but the last key.
pub fn starter() -> Result<Sentence, ParseError> {
    STARTER.parse()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::Matcher;
    use crate::testing::TestResult;

    #[test]
    fn fixtures_parse() -> Result<(), ParseError> {
        assert_eq!(sentences()?.len(), FIXTURES.len());
        assert_eq!(starter()?.text(), "チャンスは何度でもあるワン！");
        Ok(())
    }

    /// Every fixture can be completed by following the hint.
    #[test]
    fn fixtures_are_typeable_from_hints() -> TestResult {
        for s in sentences()? {
            Matcher::romaji(&s.kana())?;
        }
        Ok(())
    }
}
