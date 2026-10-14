//! Checks the built sentence data as the app reads it. Opt-in, since the
//! data is not in this repo (`just test-data` builds it first):
//!
//!     cargo test -p wantype-core --test data -- --ignored
//!
//! Reads the app's copy, or `$WANTYPE_DATA` if set.

use std::collections::BTreeSet;
use std::path::PathBuf;

use unicode_normalization::UnicodeNormalization;
use wantype_core::flick::{self, FlickError};
use wantype_core::tatoeba::{
    locate, translated_languages, with_audio, DataFile, Index, RawSentence, Readings,
};
use wantype_core::{fold, Lang, Matcher, ParseError, RomajiError, SentenceId};

#[derive(Debug, thiserror::Error)]
enum DataError {
    #[error("{path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path:?}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("sentence {id}")]
    Parse {
        id: SentenceId,
        #[source]
        source: ParseError,
    },
    #[error("sentence {id} cannot be typed")]
    Untypeable {
        id: SentenceId,
        #[source]
        source: RomajiError,
    },
    #[error("sentence {id} cannot be flicked")]
    Unflickable {
        id: SentenceId,
        #[source]
        source: FlickError,
    },
}

/// What `fold` did with the full NFKC tables.
fn nfkc_fold(c: char) -> char {
    let single = |s: String| {
        let mut chars = s.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    };
    let c = single(c.nfkc().collect()).unwrap_or(c);
    let c = match c {
        '\u{3099}' => '゛',
        '\u{309A}' => '゜',
        c => c,
    };
    single(c.to_lowercase().collect()).unwrap_or(c)
}

fn data_dir() -> PathBuf {
    std::env::var_os("WANTYPE_DATA").map_or_else(
        || concat!(env!("CARGO_MANIFEST_DIR"), "/../app/public/sentences").into(),
        PathBuf::from,
    )
}

fn read<T: serde::de::DeserializeOwned>(file: DataFile, version: &str) -> Result<T, DataError> {
    let path = data_dir().join(file.path(version));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) => return Err(DataError::Read { path, source }),
    };
    serde_json::from_str(&text).map_err(|source| DataError::Json { path, source })
}

/// Every sentence parses and can be typed to the end in romaji by following
/// the hint, and the index lists agree with the chunks.
#[test]
#[ignore]
fn the_data_is_consistent_and_every_sentence_typeable() -> Result<(), DataError> {
    let index: Index = read(DataFile::Index, "")?;
    let version = index.version.as_str();
    let readings: Readings = read(DataFile::Readings, version)?;
    assert_eq!(readings.sentences.len(), index.sentences as usize);

    let mut sentences: Vec<RawSentence> = Vec::new();
    let mut folded_otherwise = std::collections::BTreeMap::new();
    let mut off_the_pad = std::collections::BTreeMap::new();
    for i in 0..index.sentences {
        let (n, offset) = locate(i);
        if offset == 0 {
            sentences.extend(read::<Vec<RawSentence>>(DataFile::Chunk(n), version)?);
        }
        let raw = sentences[i as usize].clone();
        let id = raw.id;
        let sentence = raw
            .into_sentence()
            .map_err(|source| DataError::Parse { id, source })?;

        let ours: BTreeSet<String> = sentence.readings().into_iter().collect();
        let listed: BTreeSet<String> = readings.sentences[i as usize]
            .iter()
            .map(|&r| readings.readings[r as usize].clone())
            .collect();
        assert_eq!(ours, listed, "readings of sentence {id}");

        Matcher::romaji(&sentence.kana()).map_err(|source| DataError::Untypeable { id, source })?;
        // On a phone's flick pad: a char the pad lacks leaves the sentence
        // out there; anything else is a bug.
        match flick::gestures(&sentence.kana()) {
            Ok(_) => {}
            Err(FlickError::NoKey(c)) => *off_the_pad.entry(c).or_insert(0) += 1,
            Err(source) => return Err(DataError::Unflickable { id, source }),
        }
        // The game folds with tables (`compat.rs`), not the full NFKC: every
        // char a player types here must fold as NFKC would.
        for c in sentence.kana().chars().chain(sentence.text().chars()) {
            if fold(c) != nfkc_fold(c) {
                folded_otherwise.entry(c).or_insert(id);
            }
        }
    }
    assert!(
        folded_otherwise.is_empty(),
        "chars the tables fold otherwise than NFKC (char, a sentence): {folded_otherwise:?}"
    );
    let unflickable: usize = off_the_pad.values().sum();
    assert!(
        unflickable * 200 <= index.sentences as usize,
        "over 0.5% of the sentences cannot be flicked (char, sentences): {off_the_pad:?}"
    );
    assert_eq!(sentences.len(), index.sentences as usize, "sentences");

    assert_eq!(with_audio(&sentences), index.audio, "sentences with audio");
    let languages: BTreeSet<&Lang> = index.languages.iter().map(|l| &l.lang).collect();
    assert_eq!(languages, translated_languages(&sentences), "languages");
    Ok(())
}
