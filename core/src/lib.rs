//! Shared, UI-agnostic engine for WanType, a port of Taipingu.
//!
//! - [`Matcher`]: upstream `matcher.js`, keystroke by keystroke, and
//!   [`Slip`]: keys that spell a kana by its sound instead.
//! - [`Sentence`]: upstream's sentence model (parts with furigana) and the
//!   per-target progress every frontend renders ([`Segment`]).
//! - [`Session`]: the game as a sans-IO state machine. Frontends feed it
//!   [`Event`]s with the current time and carry out its [`Effect`]s.
//! - [`ladder`]: levels and score, the helpers fading as the player improves.
//! - [`tatoeba`]: the static sentence data's format and how sentences are
//!   picked from it.
//!
//! No clock, I/O or platform API is used, so the crate builds for
//! `wasm32-unknown-unknown` as is.

mod chars;
mod compat;
pub mod fixtures;
pub mod ladder;
mod matcher;
mod rules;
mod sentence;
mod session;
mod slip;
pub mod tatoeba;
#[cfg(test)]
mod testing;

pub use chars::fold;
pub use ladder::{Grade, Helper, Helpers, Ladder, Mode, Numerals, Outcome, Tier};
pub use matcher::{Matcher, RomajiError};
pub use sentence::{segments, Audio, ParseError, Part, Progress, Segment, Sentence, Target};
pub use session::{
    wpm, Effect, Event, Key, Modifiers, Restored, SavedError, Session, Settings, Stats,
};
pub use slip::{Slip, SlipKind};

/// ワンタイプ as a newtype, which is all it is: a wrapper around one
/// chance (ワン being "one" and Pochi's bark alike), so whether there is
/// one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WanType(pub bool);

/// ワンチャン, "maybe": there is a chance. Pochi says there are more.
pub const ワンチャン: WanType = WanType(true);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestResult;

    #[test]
    fn there_is_a_chance_and_it_is_wantype() -> TestResult {
        assert_eq!(ワンチャン, WanType(true));
        assert_eq!(Matcher::romaji(stringify!(ワンチャン))?, "wanchan");
        Ok(())
    }
}
