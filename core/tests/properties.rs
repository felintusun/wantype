//! Generated-input checks of the matcher and session invariants.

use std::time::Duration;

use proptest::prelude::*;
use wantype_core::{Event, Key, Matcher, Modifiers, Segment, Sentence, Session, Settings};

/// Kana-heavy sentences, including the chars the rules treat specially.
fn sentence_text() -> impl Strategy<Value = String> {
    let chars: Vec<char> = "あいうえおかがきぎくしじちっつてでとどなにのはぱふまやゃゅょらわをんーアイウカキシジチッツテトフヴンァィゥェォャュョー、。！？「」・abcXYZ０１ＡＢ"
        .chars()
        .collect();
    prop::collection::vec(prop::sample::select(chars), 0..16).prop_map(|v| v.into_iter().collect())
}

proptest! {
    /// Following the hint always completes the sentence, one char at a time.
    #[test]
    fn hints_complete_any_sentence(text in sentence_text()) {
        let mut m = Matcher::new(&text);
        let len = text.chars().count();
        let mut steps = 0;
        while !m.completed() {
            let before = m.japanese_index();
            let Some(hint) = m.hint() else {
                return Err(TestCaseError::fail(format!("no hint before the end of {text:?}")));
            };
            prop_assert!(m.input(hint), "hint {hint:?} rejected in {text:?}");
            prop_assert!(m.japanese_index() >= before);
            steps += 1;
            prop_assert!(steps <= 5 * len, "no progress in {text:?}");
        }
        prop_assert_eq!(m.japanese_index(), len);
    }

    /// Arbitrary keys never panic, never overrun the sentence, and a
    /// rejected key leaves progress untouched. (It may still switch the hint
    /// between romaji and kana: upstream sets the input mode on every key.)
    #[test]
    fn arbitrary_input_is_safe(text in sentence_text(), keys in prop::collection::vec(any::<char>(), 0..40)) {
        let mut m = Matcher::new(&text);
        for k in keys {
            let before = (m.japanese_index(), m.keystroke_count());
            if !m.input(k) {
                prop_assert_eq!(before, (m.japanese_index(), m.keystroke_count()));
            }
            prop_assert!(m.japanese_index() <= m.len());
            prop_assert_eq!(m.completed(), m.japanese_index() == m.len());
        }
    }

    /// Rendering stays aligned with the matcher: one target per kana char,
    /// and exactly one active target until completion.
    #[test]
    fn segments_align_with_matcher(text in sentence_text(), keys in prop::collection::vec(any::<char>(), 0..40)) {
        let sentence = Sentence::parse(None, &text, Default::default(), None)
            .map_err(|e| TestCaseError::fail(format!("{text:?}: {e}")))?;
        let mut session = Session::new(Settings::default());
        session.handle(Event::SentenceLoaded(sentence.clone()), Duration::ZERO);
        for (i, k) in keys.into_iter().enumerate() {
            session.handle(Event::Key(Key::Char(k), Modifiers::default()), Duration::from_millis(i as u64));
        }
        let targets: Vec<_> = session
            .segments()
            .into_iter()
            .flat_map(|s| match s {
                Segment::Plain(t) => vec![t],
                Segment::Ruby { reading, .. } => reading,
            })
            .collect();
        prop_assert_eq!(targets.len(), sentence.kana().chars().count());
        let active = targets.iter().filter(|t| t.progress == wantype_core::Progress::Active).count();
        prop_assert_eq!(active, usize::from(!session.completed()));
    }
}
