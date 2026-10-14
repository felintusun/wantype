//! Tatoeba's language codes.

use std::borrow::Borrow;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The language translations fall back to.
pub(crate) const ENGLISH: &str = "eng";
const JAPANESE: &str = "jpn";

/// A Tatoeba language code, ISO 639-3 mostly (`eng`, `deu`, `cmn`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Lang(String);

impl Lang {
    pub fn new(code: impl Into<String>) -> Lang {
        Lang(code.into())
    }

    /// English, what translations fall back to.
    pub fn english() -> Lang {
        Lang::new(ENGLISH)
    }

    /// Japanese, the sentences' own language.
    pub fn is_japanese(&self) -> bool {
        self.0 == JAPANESE
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Maps keyed by [`Lang`] are looked up by `&str` too.
impl Borrow<str> for Lang {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
