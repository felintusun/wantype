//! What failed in a test, so tests use `?` instead of unwrapping.

use crate::matcher::RomajiError;
use crate::sentence::ParseError;

#[derive(Debug, thiserror::Error)]
pub enum TestError {
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Romaji(#[from] RomajiError),
    /// A value the test expects is not there.
    #[error("no {0}")]
    Missing(&'static str),
}

pub type TestResult<T = ()> = Result<T, TestError>;
