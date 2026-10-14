//! The game, sans-IO: frontends feed [`Event`]s with the current time and
//! carry out the [`Effect`]s the session asks for (load a sentence, play
//! audio, persist settings). The session never reads a clock, touches the
//! network or storage, or plays sound itself, so it runs unchanged on
//! desktop and wasm and is fully testable with synthetic time.
//!
//! Behaviour and key bindings follow upstream `index.js`.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::flick::{Composer, Gesture};
use crate::ladder::{self, Helper, Helpers, Ladder, Mode, Numerals, Outcome, Recorded, Tier};
use crate::lang::Lang;
use crate::matcher::Matcher;
use crate::sentence::{segments, Audio, Segment, Sentence};
use crate::slip::Slip;
use crate::tatoeba::Filter;

/// A key press, as far as the game cares. Frontends map their framework's
/// key event to this; [`Key::normalize`] then folds the text form of
/// control keys (`"\r"` for Enter, …) into the named keys, so a frontend
/// that forwards the event's text instead of its logical key still works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Space,
    Escape,
    Backspace,
    Delete,
    /// Arrows, function keys, bare modifiers…: ignored.
    Other,
}

impl Key {
    pub fn normalize(self) -> Key {
        match self {
            Key::Char(' ') => Key::Space,
            Key::Char('\r' | '\n') => Key::Enter,
            Key::Char('\t') => Key::Tab,
            Key::Char('\u{8}') => Key::Backspace,
            Key::Char('\u{7f}') => Key::Delete,
            Key::Char('\u{1b}') => Key::Escape,
            Key::Char(c) if c.is_control() => Key::Other,
            key => key,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Key(Key, Modifiers),
    /// A gesture on the flick pad (a phone's keyboard).
    Flick(Gesture),
    /// Answer to [`Effect::RequestSentence`].
    SentenceLoaded(Sentence),
    /// [`Effect::RequestSentence`] could not be fulfilled (e.g. no sentence
    /// matches the settings). The session allows another request.
    SentenceFailed,
    /// Next-sentence button (Enter).
    Next,
    /// Restart button (Escape, Backspace, Delete).
    Restart,
    /// Replay the recording (Space while audio is on).
    PlayAudio,
    /// Turn furigana, the translation or the key hint on or off.
    SetHelper(Helper, bool),
    /// Levels and score, or Zen (picked on the first run).
    SetMode(Mode),
    /// `Some`: play at this level, fixed. `None`: climb from the current
    /// one.
    SetTier(Option<Tier>),
    SetNumerals(Numerals),
    /// Keep the helper that fades from the next sentence on (it costs its
    /// share of the multiplier).
    KeepHelper,
    /// Bring back the helper offered after a rough sentence, or not.
    TakeOffer(bool),
    /// Translation language.
    /// Choosing one also turns translations on, as upstream does. A sentence
    /// without one in it shows the English one.
    SetLanguage(Lang),
    /// The translation languages the data has, and the one the browser
    /// prefers among them (`tatoeba::browser_language`). A first run
    /// ([`Session::first_run`]) takes the browser's; a saved language the
    /// data lacks (say, one dropped from Tatoeba's exports) falls back to
    /// English.
    LanguagesLoaded {
        available: Vec<Lang>,
        browser: Option<Lang>,
    },
    /// Turning audio on plays the current recording, or moves on to the
    /// next sentence if this one has none.
    SetAudio(bool),
    SetMincho(bool),
    /// "New kanji": `Some(n)` keeps only sentences with exactly `n` unknown
    /// readings, `None` allows any.
    SetUnknownReadings(Option<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Load a sentence matching [`Session::filter`] and answer with
    /// [`Event::SentenceLoaded`] (or [`Event::SentenceFailed`]). At most one
    /// request is outstanding.
    RequestSentence,
    /// Play this recording from the start, replacing anything playing.
    PlayAudio(Audio),
    /// Audio was asked for but the sentence has no recording.
    NoAudio,
    /// [`Session::settings`] changed: persist them. Sentence selection may
    /// depend on them too.
    SettingsChanged,
    /// A missed key spelled the kana by its sound (`wa` for は): explain
    /// how it is typed. The miss still counts.
    Slip(Slip),
}

/// Settings read back from storage, and what of them could not be kept.
#[derive(Debug)]
pub struct Restored {
    pub settings: Settings,
    pub problems: Vec<SavedError>,
}

/// Part of the saved settings that could not be read; it took its default.
#[derive(Debug, thiserror::Error)]
pub enum SavedError {
    #[error("the saved settings are not JSON; all took their defaults")]
    NotJson(#[source] serde_json::Error),
    #[error("the saved settings are not a JSON object; all took their defaults")]
    NotAnObject,
    #[error("the saved setting `{field}` took its default")]
    Field {
        field: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the saved settings do not fit together; all took their defaults")]
    Together(#[source] serde_json::Error),
}

/// Player preferences, persisted by the frontend. The serde form is
/// upstream's `localStorage["taipingu"]` JSON, so its saved settings and
/// known readings read as they are; missing fields take upstream's
/// defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Translation language. (Turning translations off is the helper's.)
    pub language: Lang,
    pub translation_enabled: bool,
    pub furigana: bool,
    /// Show the key to type next, while furigana are shown (without them
    /// the key would give the reading away). Upstream always shows it.
    pub hint: bool,
    /// How the game is played; `None` until picked on the first run (not
    /// upstream's, nor are the fields below up to `audio`).
    pub mode: Option<Mode>,
    pub ladder: Ladder,
    pub numerals: Numerals,
    /// Autoplay each sentence's recording when it loads.
    pub audio: bool,
    /// Serif (mincho) font.
    pub mincho: bool,
    pub unknown_readings: Option<u8>,
    /// Readings (`漢字|かんじ`) typed with furigana hidden. Shared: they grow
    /// to thousands, and settings are copied and compared often.
    pub known_readings: Arc<BTreeSet<String>>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: Lang::english(),
            translation_enabled: true,
            furigana: true,
            hint: true,
            mode: None,
            ladder: Ladder::default(),
            numerals: Numerals::default(),
            audio: false,
            mincho: false,
            unknown_readings: None,
            known_readings: Arc::default(),
        }
    }
}

impl Settings {
    /// Settings saved as JSON, keeping every valid field: a malformed one
    /// (hand-edited, or from another version) takes its default instead of
    /// discarding the rest, which the next save would then overwrite,
    /// known readings included. What could not be kept is listed.
    pub fn from_saved(json: &str) -> Restored {
        use serde_json::{Map, Value};
        let mut problems = Vec::new();
        let saved = match serde_json::from_str::<Value>(json) {
            Ok(Value::Object(saved)) => saved,
            Ok(_) => {
                problems.push(SavedError::NotAnObject);
                Map::new()
            }
            Err(e) => {
                problems.push(SavedError::NotJson(e));
                Map::new()
            }
        };
        // Each field is tried alone (the others take their defaults; no rule
        // spans fields): one that does not parse is left out, and those kept
        // are parsed together once. Keys that are no field pass, as serde
        // ignores them.
        let mut fields = Map::new();
        for (key, value) in saved {
            let field = Map::from_iter([(key.clone(), value)]);
            match Settings::deserialize(&field) {
                Ok(_) => fields.extend(field),
                Err(source) => problems.push(SavedError::Field { field: key, source }),
            }
        }
        let settings = Settings::deserialize(&fields).unwrap_or_else(|source| {
            problems.push(SavedError::Together(source));
            Settings::default()
        });
        Restored { settings, problems }
    }

    /// A helper's setting (whether it shows: [`Settings::shown`]).
    pub fn helper(&self, helper: Helper) -> bool {
        match helper {
            Helper::Hint => self.hint,
            Helper::Translation => self.translation_enabled,
            Helper::Furigana => self.furigana,
        }
    }

    pub fn set_helper(&mut self, helper: Helper, on: bool) {
        match helper {
            Helper::Hint => self.hint = on,
            Helper::Translation => self.translation_enabled = on,
            Helper::Furigana => self.furigana = on,
        }
    }

    /// The helpers shown, with one setting changed if `change` says so. The
    /// key hint only shows with furigana (it would give the reading away),
    /// the translation with a language.
    pub fn shown(&self, change: Option<(Helper, bool)>) -> Helpers {
        let on = |h| match change {
            Some((changed, on)) if changed == h => on,
            _ => self.helper(h),
        };
        Helpers {
            hint: on(Helper::Hint) && on(Helper::Furigana),
            translation: on(Helper::Translation),
            furigana: on(Helper::Furigana),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    /// Accepted keystrokes.
    pub keystrokes: usize,
    /// Rejected keystrokes (not tracked upstream).
    pub mistakes: usize,
    /// Upstream's formula, `keystrokes / 5` per minute from the first
    /// accepted key to the last: so far while typing, final once completed.
    /// `None` before two keys.
    pub wpm: Option<f64>,
}

impl Stats {
    /// Share of keystrokes that were correct (正確率), `None` before the
    /// first key.
    pub fn accuracy(&self) -> Option<f64> {
        let total = self.keystrokes + self.mistakes;
        (total > 0).then(|| self.keystrokes as f64 / total as f64)
    }
}

#[derive(Debug, Clone)]
struct Round {
    sentence: Sentence,
    matcher: Matcher,
    mistakes: usize,
    /// Targets (kana indices) where a wrong key was pressed.
    errors: BTreeSet<usize>,
    started: Option<Duration>,
    /// The last accepted key (the last of all once completed).
    last: Option<Duration>,
    /// Helpers shown at some point while this sentence was up. With
    /// furigana, completing it does not prove the readings are known; each
    /// one shown counts against the score's multiplier.
    seen: Helpers,
    /// Completed before a restart: a retry is not scored again.
    completed_before: bool,
    /// Flick input: the kana held back for ゛゜小.
    composer: Composer,
}

impl Round {
    fn new(sentence: Sentence, seen: Helpers) -> Self {
        Round {
            matcher: Matcher::new(&sentence.kana()),
            sentence,
            mistakes: 0,
            errors: BTreeSet::new(),
            started: None,
            last: None,
            seen,
            completed_before: false,
            composer: Composer::default(),
        }
    }

    fn reset(&mut self) {
        self.completed_before |= self.matcher.completed();
        self.matcher.reset();
        self.mistakes = 0;
        self.errors.clear();
        self.started = None;
        self.last = None;
        self.composer.clear();
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    settings: Settings,
    round: Option<Round>,
    loading: bool,
    /// How the last completed sentence went, until the next one or a
    /// restart.
    outcome: Option<Outcome>,
    /// The helper the level just reached fades from the next sentence.
    fade_pending: Option<Helper>,
    /// Nothing was saved: the browser chooses the translation language.
    first_run: bool,
    effects: VecDeque<Effect>,
}

impl Session {
    /// Starts without a sentence and immediately requests one.
    pub fn new(settings: Settings) -> Self {
        let mut session = Session {
            settings,
            round: None,
            loading: false,
            outcome: None,
            fade_pending: None,
            first_run: false,
            effects: VecDeque::new(),
        };
        session.request_sentence();
        session
    }

    /// [`Session::new`] with the default settings, nothing being saved yet:
    /// the translation follows the browser's language once the languages
    /// are loaded.
    pub fn first_run() -> Self {
        Session {
            first_run: true,
            ..Session::new(Settings::default())
        }
    }

    /// Feed one event. `now` is any monotonic clock reading (e.g. time since
    /// app start); only differences between readings are used.
    pub fn handle(&mut self, event: Event, now: Duration) {
        match event {
            Event::Key(key, mods) => self.handle_key(key, mods, now),
            Event::Flick(gesture) => self.flick(gesture, now),
            Event::SentenceLoaded(sentence) => self.load(sentence),
            Event::SentenceFailed => self.loading = false,
            Event::Next => self.request_sentence(),
            Event::Restart => {
                if let Some(round) = &mut self.round {
                    round.reset();
                }
                self.outcome = None;
            }
            Event::PlayAudio => self.play_audio(),
            Event::SetHelper(helper, on) => self.change_settings(|s| s.set_helper(helper, on)),
            Event::SetMode(mode) => self.change_settings(|s| {
                s.mode = Some(mode);
                if mode == Mode::Score {
                    // What the level fades goes.
                    let tier = s.ladder.tier;
                    for helper in Helper::ALL.into_iter().filter(|h| h.faded(tier)) {
                        s.set_helper(helper, false);
                    }
                }
            }),
            Event::SetTier(Some(tier)) => {
                self.change_settings(|s| {
                    s.ladder.tier = tier;
                    s.ladder.fixed = true;
                    s.ladder.pips = 0;
                    // Exactly the level's helpers.
                    for helper in Helper::ALL {
                        s.set_helper(helper, tier.helpers().get(helper));
                    }
                });
                self.fade_pending = None;
            }
            Event::SetTier(None) => self.change_settings(|s| s.ladder.fixed = false),
            Event::SetNumerals(numerals) => self.change_settings(|s| s.numerals = numerals),
            Event::KeepHelper => self.fade_pending = None,
            Event::TakeOffer(take) => {
                let offer = self.outcome.as_mut().and_then(|o| o.recorded.offer.take());
                if let (true, Some(helper)) = (take, offer) {
                    self.change_settings(|s| s.set_helper(helper, true));
                }
            }
            Event::SetLanguage(language) => self.change_settings(|s| {
                s.translation_enabled = true;
                s.language = language;
            }),
            Event::LanguagesLoaded { available, browser } => {
                let first_run = std::mem::take(&mut self.first_run);
                let language = &self.settings.language;
                let change = match browser {
                    Some(browser) if first_run && available.contains(&browser) => Some(browser),
                    _ if !available.contains(language) => Some(Lang::english()),
                    _ => None,
                };
                if let Some(language) = change.filter(|l| l != language) {
                    self.change_settings(|s| s.language = language);
                }
            }
            Event::SetMincho(on) => self.change_settings(|s| s.mincho = on),
            Event::SetAudio(on) => {
                self.change_settings(|s| s.audio = on);
                if on {
                    // Upstream's enableAudio: play this one, or move on to
                    // a sentence that may have a recording.
                    if self.current_audio().is_some() {
                        self.play_audio();
                    } else {
                        self.request_sentence();
                    }
                }
            }
            Event::SetUnknownReadings(n) => {
                self.change_settings(|s| s.unknown_readings = n);
                let mismatch = match (n, self.sentence()) {
                    (Some(n), Some(s)) => self.unknown_reading_count(s) != usize::from(n),
                    _ => false,
                };
                if mismatch {
                    self.request_sentence();
                }
            }
        }
    }

    /// Next effect for the frontend to carry out, in order.
    pub fn poll_effect(&mut self) -> Option<Effect> {
        self.effects.pop_front()
    }

    fn handle_key(&mut self, key: Key, mods: Modifiers, now: Duration) {
        // Leave shortcuts (Ctrl/Cmd+…) to the browser/OS.
        if mods.ctrl || mods.meta {
            return;
        }
        let event = match key.normalize() {
            Key::Char(c) => return self.type_char(c, now),
            Key::Tab => Event::SetHelper(Helper::Furigana, !self.settings.furigana),
            Key::Enter => Event::Next,
            Key::Escape | Key::Backspace | Key::Delete => Event::Restart,
            Key::Space if mods.shift => Event::SetAudio(false),
            Key::Space if self.settings.audio => Event::PlayAudio,
            Key::Space => Event::SetAudio(true),
            Key::Other => return,
        };
        self.handle(event, now);
    }

    fn load(&mut self, sentence: Sentence) {
        self.loading = false;
        // "One new kanji" hides furigana for each new sentence, so typing it
        // can mark the reading as known.
        if self.settings.unknown_readings == Some(1) {
            self.change_settings(|s| s.furigana = false);
        }
        // The level reached with the last sentence fades its helper now.
        if let Some(helper) = self.fade_pending.take() {
            self.change_settings(|s| s.set_helper(helper, false));
        }
        if self.settings.audio {
            if let Some(audio) = &sentence.audio {
                self.effects.push_back(Effect::PlayAudio(audio.clone()));
            }
        }
        self.outcome = None;
        self.round = Some(Round::new(sentence, self.helpers()));
    }

    /// A gesture types what the composer lets through ([`Composer::input`]).
    fn flick(&mut self, gesture: Gesture, now: Duration) {
        let Some(round) = &mut self.round else { return };
        let keys = round.composer.input(gesture, round.matcher.current_kana());
        for key in keys.chars() {
            self.type_char(key, now);
        }
    }

    fn type_char(&mut self, c: char, now: Duration) {
        let Some(round) = &mut self.round else { return };
        if round.matcher.completed() {
            return;
        }
        let first = round.matcher.keystroke_count() == 0;
        if !round.matcher.input(c) {
            round.mistakes += 1;
            round.errors.insert(round.matcher.japanese_index());
            if let Some(slip) = round.matcher.slip(c) {
                self.effects.push_back(Effect::Slip(slip));
            }
            return;
        }
        if first {
            round.started = Some(now);
        }
        round.last = Some(now);
        if !round.matcher.completed() {
            return;
        }
        // Typed without furigana, the readings are known now. Playing for
        // score, the sentence is recorded (once: not a retry).
        let (seen, retry) = (round.seen, round.completed_before);
        let readings = (!seen.furigana).then(|| round.sentence.readings());
        let stats = self.stats();
        let accuracy = stats.accuracy().unwrap_or(1.0);
        let counted = self.scoring() && !retry;
        let mut recorded = Recorded::unscored(accuracy, seen, self.settings.ladder.tier);
        self.change_settings(|s| {
            if let Some(readings) = readings {
                Arc::make_mut(&mut s.known_readings).extend(readings);
            }
            if counted {
                recorded = s.ladder.record(accuracy, stats.keystrokes, seen);
            }
        });
        self.fade_pending = recorded.tier_up.and_then(Tier::fades);
        self.outcome = Some(Outcome {
            accuracy,
            wpm: stats.wpm.unwrap_or(0.0),
            counted,
            recorded,
        });
    }

    /// The helpers shown now.
    pub fn helpers(&self) -> Helpers {
        self.settings.shown(None)
    }

    /// The multiplier the helpers shown now make, at the current level.
    pub fn multiplier(&self) -> f64 {
        ladder::multiplier(self.helpers(), self.settings.ladder.tier)
    }

    /// The multiplier with `helper` turned on or off.
    pub fn multiplier_with(&self, helper: Helper, on: bool) -> f64 {
        let shown = self.settings.shown(Some((helper, on)));
        ladder::multiplier(shown, self.settings.ladder.tier)
    }

    /// The helper is off by the level (playing for score), not by choice.
    pub fn faded(&self, helper: Helper) -> bool {
        self.scoring() && helper.faded(self.settings.ladder.tier)
    }

    /// Playing for levels and score (not Zen, and picked).
    pub fn scoring(&self) -> bool {
        self.settings.mode == Some(Mode::Score)
    }

    /// Numbers are shown in kanji digits.
    pub fn kanji_numerals(&self) -> bool {
        match self.settings.numerals {
            Numerals::Kanji => true,
            Numerals::Arabic => false,
            Numerals::Auto => self.scoring() && self.settings.ladder.tier.kanji_numerals(),
        }
    }

    /// How the last completed sentence went.
    pub fn outcome(&self) -> Option<&Outcome> {
        self.outcome.as_ref()
    }

    /// The helper fading from the next sentence, unless kept.
    pub fn fade_pending(&self) -> Option<Helper> {
        self.fade_pending
    }

    /// Apply a settings change; announce it if anything changed. Deliberate
    /// deviation: upstream only notices furigana being visible when some
    /// setting is saved while it is on, so with furigana on throughout, a
    /// completed sentence would count as typed without it.
    fn change_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        let before = self.settings.clone();
        change(&mut self.settings);
        // One save for however many changes an event makes.
        if self.settings != before && !self.effects.contains(&Effect::SettingsChanged) {
            self.effects.push_back(Effect::SettingsChanged);
        }
        let shown = self.helpers();
        if let Some(round) = &mut self.round {
            round.seen = round.seen.union(shown);
        }
    }

    fn request_sentence(&mut self) {
        if !self.loading {
            self.loading = true;
            self.effects.push_back(Effect::RequestSentence);
        }
    }

    fn play_audio(&mut self) {
        let effect = match self.current_audio() {
            Some(audio) => Effect::PlayAudio(audio.clone()),
            None => Effect::NoAudio,
        };
        self.effects.push_back(effect);
    }

    fn current_audio(&self) -> Option<&Audio> {
        self.round.as_ref()?.sentence.audio.as_ref()
    }

    fn unknown_reading_count(&self, sentence: &Sentence) -> usize {
        sentence
            .readings()
            .iter()
            .filter(|r| !self.settings.known_readings.contains(*r))
            .collect::<BTreeSet<_>>()
            .len()
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Which sentences [`Effect::RequestSentence`] should choose from.
    pub fn filter(&self) -> Filter<'_> {
        Filter {
            audio: self.settings.audio,
            unknown_readings: self.settings.unknown_readings,
            known_readings: &self.settings.known_readings,
        }
    }

    /// A sentence request is outstanding.
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn sentence(&self) -> Option<&Sentence> {
        self.round.as_ref().map(|r| &r.sentence)
    }

    /// The current sentence laid out with progress and mistakes. Kanji are
    /// marked unknown in "one new kanji" mode, as upstream does.
    pub fn segments(&self) -> Vec<Segment> {
        let Some(r) = &self.round else {
            return Vec::new();
        };
        let mark_unknown = self.settings.unknown_readings == Some(1);
        segments(
            &r.sentence,
            r.matcher.japanese_index(),
            |i| r.errors.contains(&i),
            |reading| mark_unknown && !self.settings.known_readings.contains(reading),
        )
    }

    /// Next key to press, `None` when completed or without a sentence.
    pub fn hint(&self) -> Option<char> {
        let r = self.round.as_ref()?;
        if r.matcher.completed() {
            None
        } else {
            r.matcher.hint()
        }
    }

    pub fn completed(&self) -> bool {
        self.round.as_ref().is_some_and(|r| r.matcher.completed())
    }

    /// The kana held back for ゛゜小 ([`Event::Flick`]), shown as being
    /// typed.
    pub fn held(&self) -> Option<char> {
        self.round.as_ref()?.composer.held()
    }

    /// The gesture on the flick pad that brings the kana at the cursor
    /// closer; `None` when completed or without a sentence.
    pub fn flick_hint(&self) -> Option<Gesture> {
        let round = self.round.as_ref()?;
        round.composer.hint(round.matcher.current_kana())
    }

    pub fn stats(&self) -> Stats {
        let Some(r) = &self.round else {
            return Stats {
                keystrokes: 0,
                mistakes: 0,
                wpm: None,
            };
        };
        let keystrokes = r.matcher.keystroke_count();
        let wpm = match (r.started, r.last) {
            (Some(start), Some(end)) => wpm(keystrokes, end.saturating_sub(start)),
            _ => None,
        };
        Stats {
            keystrokes,
            mistakes: r.mistakes,
            wpm,
        }
    }
}

/// `keystrokes / 5` words per minute; `None` for a zero duration.
pub fn wpm(keystrokes: usize, elapsed: Duration) -> Option<f64> {
    let minutes = elapsed.as_secs_f64() / 60.0;
    (minutes > 0.0).then(|| keystrokes as f64 / 5.0 / minutes)
}

#[cfg(test)]
mod tests {
    use crate::testing::{TestError, TestResult};
    use serde_json::json;
    use std::collections::BTreeMap;

    use super::*;
    use crate::flick::{Flick, Pad};
    use crate::sentence::AudioId;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn key(c: char) -> Event {
        Event::Key(Key::Char(c), Modifiers::default())
    }

    fn named(k: Key) -> Event {
        Event::Key(k, Modifiers::default())
    }

    fn sentence(transcription: &str, audio: bool) -> TestResult<Sentence> {
        let audio = audio.then(|| Audio {
            id: AudioId(1),
            author: "a".into(),
        });
        Ok(Sentence::parse(
            None,
            transcription,
            BTreeMap::new(),
            audio,
        )?)
    }

    fn effects(s: &mut Session) -> Vec<Effect> {
        std::iter::from_fn(|| s.poll_effect()).collect()
    }

    /// A session with `transcription` loaded and all effects drained.
    fn loaded_with(settings: Settings, transcription: &str, audio: bool) -> TestResult<Session> {
        let mut s = Session::new(settings);
        assert_eq!(effects(&mut s), [Effect::RequestSentence]);
        s.handle(
            Event::SentenceLoaded(sentence(transcription, audio)?),
            ms(0),
        );
        effects(&mut s);
        Ok(s)
    }

    fn loaded(transcription: &str, audio: bool) -> TestResult<Session> {
        loaded_with(Settings::default(), transcription, audio)
    }

    fn type_all(s: &mut Session, input: &str) {
        for c in input.chars() {
            s.handle(key(c), ms(0));
        }
    }

    #[test]
    fn a_flicked_kana_waits_for_its_turn_within_the_round() -> TestResult {
        let mut s = loaded("ちゃ", false)?;
        // ち, then や held for ゃ.
        let hold = |s: &mut Session| {
            for gesture in [Pad::Ta.flick(Flick::Left), Pad::Ya.tap()] {
                s.handle(Event::Flick(gesture), ms(0));
            }
        };
        hold(&mut s);
        assert_eq!(s.held(), Some('や'));
        assert_eq!(s.flick_hint(), Some(Gesture::TURN));
        // A retry, or a new sentence, drops it.
        s.handle(Event::Restart, ms(0));
        assert_eq!(s.held(), None);
        hold(&mut s);
        s.handle(Event::SentenceLoaded(sentence("ちゃ", false)?), ms(0));
        assert_eq!(s.held(), None);
        hold(&mut s);
        s.handle(Event::Flick(Gesture::TURN), ms(0));
        assert!(s.completed());
        assert_eq!(s.stats().mistakes, 0);
        Ok(())
    }

    #[test]
    fn types_through_kanji_readings() -> TestResult {
        let mut s = loaded("[日|に][本|ほん]", false)?;
        type_all(&mut s, "nihon");
        assert!(s.completed());
        assert_eq!(s.hint(), None);
        Ok(())
    }

    #[test]
    fn wpm_is_frozen_at_completion_and_starts_on_first_accepted_key() -> TestResult {
        let mut s = loaded("かか", false)?;
        s.handle(key('x'), ms(0)); // wrong first key does not start the clock
        s.handle(key('k'), ms(1_000));
        s.handle(key('a'), ms(2_000));
        s.handle(key('k'), ms(3_000));
        s.handle(key('a'), ms(13_000));
        let stats = s.stats();
        assert_eq!(stats.keystrokes, 4);
        assert_eq!(stats.mistakes, 1);
        assert_eq!(stats.accuracy(), Some(0.8));
        // 4 keys = 0.8 words in 12 s = 0.2 min → 4 wpm.
        assert_eq!(stats.wpm, Some(4.0));
        // Later keys change nothing.
        s.handle(key('k'), ms(60_000));
        assert_eq!(s.stats(), stats);
        Ok(())
    }

    #[test]
    fn a_key_spelling_the_sound_is_a_miss_with_a_note() -> TestResult {
        let mut s = loaded("これは", false)?;
        type_all(&mut s, "korew");
        assert_eq!(
            effects(&mut s),
            [Effect::Slip(Slip {
                kind: crate::SlipKind::Particle,
                kana: 'は',
                typed: "ha".into(),
                said: "wa".into(),
            })]
        );
        assert_eq!(s.stats().mistakes, 1);
        // Inside a spelling (h…) the same key is a plain miss.
        type_all(&mut s, "hw");
        assert_eq!(effects(&mut s), []);
        type_all(&mut s, "a");
        assert!(s.completed());
        Ok(())
    }

    #[test]
    fn mistakes_mark_the_active_target_until_reset() -> TestResult {
        let mut s = loaded("[日|に]は", false)?;
        s.handle(key('q'), ms(0));
        let Segment::Ruby { error, .. } = &s.segments()[0] else {
            return Err(TestError::Missing("the segment the test expects"));
        };
        assert!(*error);
        s.handle(named(Key::Backspace), ms(0));
        assert!(s.segments().iter().all(|seg| match seg {
            Segment::Ruby { error, .. } => !error,
            Segment::Plain(t) => !t.error,
        }));
        assert_eq!(s.stats().mistakes, 0);
        Ok(())
    }

    #[test]
    fn enter_requests_one_sentence_at_a_time() -> TestResult {
        let mut s = loaded("か", false)?;
        s.handle(named(Key::Enter), ms(0));
        s.handle(Event::Next, ms(0));
        assert_eq!(effects(&mut s), [Effect::RequestSentence]);
        assert!(s.is_loading());
        s.handle(Event::SentenceFailed, ms(0));
        s.handle(named(Key::Enter), ms(0));
        assert_eq!(effects(&mut s), [Effect::RequestSentence]);
        Ok(())
    }

    #[test]
    fn control_chars_from_key_text_act_as_named_keys() -> TestResult {
        let mut s = loaded("か", false)?;
        s.handle(key('\r'), ms(0));
        assert_eq!(effects(&mut s), [Effect::RequestSentence]);
        s.handle(key('k'), ms(0));
        s.handle(key('\u{8}'), ms(0));
        assert_eq!(s.stats().keystrokes, 0);
        assert_eq!(s.stats().mistakes, 0);
        Ok(())
    }

    #[test]
    fn tab_toggles_furigana_and_shortcuts_are_ignored() -> TestResult {
        let mut s = loaded("か", false)?;
        assert!(s.settings().furigana);
        s.handle(named(Key::Tab), ms(0));
        assert!(!s.settings().furigana);
        assert_eq!(effects(&mut s), [Effect::SettingsChanged]);
        let ctrl = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        s.handle(Event::Key(Key::Char('k'), ctrl), ms(0));
        assert_eq!(s.stats().keystrokes, 0);
        Ok(())
    }

    #[test]
    fn space_enables_and_plays_audio() -> TestResult {
        let mut s = loaded("か", true)?;
        s.handle(named(Key::Space), ms(0));
        assert!(s.settings().audio);
        assert!(matches!(
            effects(&mut s)[..],
            [Effect::SettingsChanged, Effect::PlayAudio(_)]
        ));
        s.handle(named(Key::Space), ms(0));
        assert!(matches!(effects(&mut s)[..], [Effect::PlayAudio(_)]));
        // Autoplay on the next sentence.
        s.handle(named(Key::Enter), ms(0));
        assert_eq!(effects(&mut s), [Effect::RequestSentence]);
        s.handle(Event::SentenceLoaded(sentence("き", true)?), ms(0));
        assert!(matches!(effects(&mut s)[..], [Effect::PlayAudio(_)]));
        // Shift+Space turns it off.
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        s.handle(Event::Key(Key::Space, shift), ms(0));
        assert!(!s.settings().audio);
        assert_eq!(effects(&mut s), [Effect::SettingsChanged]);
        Ok(())
    }

    #[test]
    fn enabling_audio_without_a_recording_moves_on() -> TestResult {
        let mut s = loaded("か", false)?;
        s.handle(Event::SetAudio(true), ms(0));
        assert_eq!(
            effects(&mut s),
            [Effect::SettingsChanged, Effect::RequestSentence]
        );
        s.handle(Event::SentenceLoaded(sentence("き", false)?), ms(0));
        s.handle(Event::PlayAudio, ms(0));
        assert_eq!(effects(&mut s), [Effect::NoAudio]);
        Ok(())
    }

    #[test]
    fn keys_before_the_first_sentence_are_harmless() {
        let mut s = Session::new(Settings::default());
        s.handle(key('k'), ms(0));
        s.handle(named(Key::Backspace), ms(0));
        assert!(s.segments().is_empty());
        assert_eq!(s.hint(), None);
        assert_eq!(effects(&mut s), [Effect::RequestSentence]);
    }

    #[test]
    fn readings_typed_without_furigana_become_known() -> TestResult {
        let off = Settings {
            furigana: false,
            ..Settings::default()
        };
        let mut s = loaded_with(off, "[日|に]は", false)?;
        type_all(&mut s, "niha");
        assert!(s.settings().known_readings.contains("日|に"));
        assert_eq!(effects(&mut s), [Effect::SettingsChanged]);
        Ok(())
    }

    #[test]
    fn readings_seen_with_furigana_stay_unknown() -> TestResult {
        // Furigana on throughout (upstream would wrongly count this).
        let mut s = loaded("[日|に]は", false)?;
        type_all(&mut s, "niha");
        assert!(s.settings().known_readings.is_empty());

        // Revealed midway, then hidden again: still not "known".
        let off = Settings {
            furigana: false,
            ..Settings::default()
        };
        let mut s = loaded_with(off, "[日|に]は", false)?;
        s.handle(named(Key::Tab), ms(0));
        s.handle(named(Key::Tab), ms(0));
        type_all(&mut s, "niha");
        assert!(s.settings().known_readings.is_empty());
        Ok(())
    }

    #[test]
    fn one_new_kanji_hides_furigana_and_marks_unknown_kanji() -> TestResult {
        let settings = Settings {
            unknown_readings: Some(1),
            ..Settings::default()
        };
        let mut s = Session::new(settings);
        effects(&mut s);
        s.handle(Event::SentenceLoaded(sentence("[日|に]は", false)?), ms(0));
        assert!(!s.settings().furigana);
        assert_eq!(effects(&mut s), [Effect::SettingsChanged]);
        let Segment::Ruby { unknown, .. } = &s.segments()[0] else {
            return Err(TestError::Missing("the segment the test expects"));
        };
        assert!(*unknown);
        Ok(())
    }

    #[test]
    fn changing_new_kanji_mode_moves_on_if_the_sentence_does_not_fit() -> TestResult {
        let mut s = loaded("[日|に]は", false)?;
        s.handle(Event::SetUnknownReadings(Some(1)), ms(0));
        assert_eq!(effects(&mut s), [Effect::SettingsChanged]); // one unknown: fits
        s.handle(Event::SetUnknownReadings(Some(0)), ms(0));
        assert_eq!(
            effects(&mut s),
            [Effect::SettingsChanged, Effect::RequestSentence]
        );
        assert_eq!(s.filter().unknown_readings, Some(0));
        Ok(())
    }

    #[test]
    fn choosing_a_language_turns_translations_on() -> TestResult {
        let mut s = loaded("か", false)?;
        s.handle(Event::SetHelper(Helper::Translation, false), ms(0));
        s.handle(Event::SetLanguage(lang("fra")), ms(0));
        assert!(s.settings().translation_enabled);
        assert_eq!(s.settings().language, lang("fra"));
        // Unchanged settings are not announced.
        effects(&mut s);
        s.handle(Event::SetMincho(false), ms(0));
        assert!(effects(&mut s).is_empty());
        Ok(())
    }

    #[test]
    fn settings_use_upstreams_storage_format() -> TestResult {
        let stored = json!({
            "language": "fra", "translationEnabled": false, "furigana": true,
            "audio": false, "mincho": true, "unknownReadings": 1, "knownReadings": ["日|に"],
        });
        let s: Settings = serde_json::from_value(stored)?;
        assert_eq!(s.language, lang("fra"));
        assert!(!s.translation_enabled && s.mincho);
        assert_eq!(s.unknown_readings, Some(1));
        assert!(s.known_readings.contains("日|に"));
        let saved = serde_json::to_value(&s)?;
        assert_eq!(saved["translationEnabled"], json!(false));
        assert_eq!(saved["unknownReadings"], json!(1));
        // Older or partial data falls back to the defaults.
        let partial: Settings = serde_json::from_value(json!({ "audio": true }))?;
        assert_eq!(
            partial,
            Settings {
                audio: true,
                ..Settings::default()
            }
        );
        let any: Settings = serde_json::from_value(json!({ "unknownReadings": null }))?;
        assert_eq!(any.unknown_readings, None);
        Ok(())
    }

    #[test]
    fn a_language_the_sentence_lacks_keeps_it() -> TestResult {
        let mut s = Session::new(Settings::default());
        effects(&mut s);
        let translations = BTreeMap::from([(Lang::new("deu"), "Katze".to_string())]);
        let sentence = Sentence::parse(None, "ねこ", translations, None)?;
        s.handle(Event::SentenceLoaded(sentence), ms(0));
        effects(&mut s);
        s.handle(Event::SetLanguage(lang("deu")), ms(0));
        assert_eq!(effects(&mut s), [Effect::SettingsChanged]);
        // No French: it stays, showing the English translation.
        s.handle(Event::SetLanguage(lang("fra")), ms(0));
        assert_eq!(effects(&mut s), [Effect::SettingsChanged]);
        Ok(())
    }

    #[test]
    fn an_unknown_language_falls_back() -> TestResult {
        let mut s = loaded("か", false)?;
        s.handle(Event::SetLanguage(lang("xyz")), ms(0));
        s.handle(Event::SetHelper(Helper::Translation, false), ms(0));
        effects(&mut s);
        s.handle(loaded_languages(&["deu", "eng"], Some("deu")), ms(0));
        assert_eq!(s.settings().language, lang("eng"));
        // Only the language changes.
        assert!(!s.settings().translation_enabled);
        assert!(!effects(&mut s).is_empty());
        // A known language stays.
        s.handle(loaded_languages(&["eng"], None), ms(0));
        assert!(effects(&mut s).is_empty());
        // Upstream's "" (no translation) is just another code the data
        // lacks: translations themselves are turned off with the helper.
        let Restored { settings, .. } = Settings::from_saved(r#"{ "language": "" }"#);
        let mut s = Session::new(settings);
        s.handle(loaded_languages(&["deu", "eng"], None), ms(0));
        assert_eq!(s.settings().language, lang("eng"));
        Ok(())
    }

    fn lang(code: &str) -> Lang {
        Lang::new(code)
    }

    fn loaded_languages(available: &[&str], browser: Option<&str>) -> Event {
        Event::LanguagesLoaded {
            available: available.iter().map(|&l| Lang::new(l)).collect(),
            browser: browser.map(Lang::new),
        }
    }

    #[test]
    fn a_first_run_takes_the_browser_language() {
        let mut s = Session::first_run();
        // Not one the data lacks.
        s.handle(loaded_languages(&["eng"], Some("deu")), ms(0));
        assert_eq!(s.settings().language, lang("eng"));
        let mut s = Session::first_run();
        s.handle(loaded_languages(&["deu", "eng"], Some("deu")), ms(0));
        assert_eq!(s.settings().language, lang("deu"));
        // Only once: a later list keeps the player's choice. (Saved
        // settings never take it: see `an_unknown_language_falls_back`.)
        s.handle(Event::SetLanguage(lang("fra")), ms(0));
        s.handle(loaded_languages(&["deu", "eng", "fra"], Some("deu")), ms(0));
        assert_eq!(s.settings().language, lang("fra"));
    }

    #[test]
    fn a_malformed_field_keeps_the_rest() {
        let saved = json!({
            "language": "deu", "furigana": "yes", "knownReadings": ["日|に"], "audio": 7,
        });
        let Restored {
            settings: s,
            problems,
        } = Settings::from_saved(&saved.to_string());
        assert_eq!(s.language, lang("deu"));
        assert_eq!(*s.known_readings, ["日|に".to_string()].into());
        // The malformed ones take their defaults, and are named.
        assert!(s.furigana && !s.audio);
        let fields: Vec<&str> = problems
            .iter()
            .filter_map(|p| match p {
                SavedError::Field { field, .. } => Some(field.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(fields, ["audio", "furigana"], "{problems:?}");
        let not_json = Settings::from_saved("not json");
        assert_eq!(not_json.settings, Settings::default());
        assert!(matches!(not_json.problems[..], [SavedError::NotJson(_)]));
        let array = Settings::from_saved(&json!([1]).to_string());
        assert_eq!(array.settings, Settings::default());
        assert!(matches!(array.problems[..], [SavedError::NotAnObject]));
        assert!(Settings::from_saved("{}").problems.is_empty());
    }

    #[test]
    fn keys_that_are_no_setting_pass() {
        // Of other versions, or written by another page of the origin.
        let restored = Settings::from_saved(&json!({ "mincho": true, "stray": 1 }).to_string());
        assert!(restored.settings.mincho);
        assert!(restored.problems.is_empty(), "{:?}", restored.problems);
    }

    #[test]
    fn modes_and_levels_are_saved_by_name() {
        let saved =
            json!({ "mode": "zen", "ladder": { "tier": "jokyu", "score": 120 }, "mincho": true });
        let s = Settings::from_saved(&saved.to_string()).settings;
        assert_eq!(s.mode, Some(Mode::Zen));
        assert_eq!((s.ladder.tier, s.ladder.score), (Tier::Jokyu, 120));
        // One this version does not know (a newer one's) takes the default;
        // the first run has none.
        let saved = json!({ "mode": "marble", "mincho": true });
        let s = Settings::from_saved(&saved.to_string()).settings;
        assert_eq!(s.mode, None);
        assert!(s.mincho);
    }

    fn scoring() -> Settings {
        Settings {
            mode: Some(Mode::Score),
            ..Settings::default()
        }
    }

    /// Type `input` with a second between keys.
    fn type_timed(s: &mut Session, input: &str) {
        for (i, c) in input.chars().enumerate() {
            s.handle(key(c), ms(1_000 * i as u64));
        }
    }

    #[test]
    fn a_completed_sentence_scores_with_the_helpers_shown() -> TestResult {
        let mut s = loaded_with(scoring(), "かか", false)?;
        type_timed(&mut s, "kaka");
        let o = s.outcome().ok_or(TestError::Missing("an outcome"))?.clone();
        assert!(o.counted);
        assert_eq!(
            (o.recorded.grade, o.recorded.multiplier),
            (crate::Grade::Clean, 1.0)
        );
        // 4 keys × 5 points, in tens.
        assert_eq!(o.recorded.gain, 20);
        assert_eq!(s.settings().ladder.score, 20);
        assert_eq!(s.settings().ladder.pips, 1);
        assert!(effects(&mut s).contains(&Effect::SettingsChanged));
        Ok(())
    }

    #[test]
    fn a_helper_shown_at_any_point_counts() -> TestResult {
        let settings = Settings {
            hint: false,
            ..scoring()
        };
        let mut s = loaded_with(settings, "かか", false)?;
        assert_eq!(s.multiplier(), 1.5);
        s.handle(Event::SetHelper(Helper::Hint, true), ms(0));
        s.handle(Event::SetHelper(Helper::Hint, false), ms(0));
        type_timed(&mut s, "kaka");
        assert_eq!(
            s.outcome()
                .ok_or(TestError::Missing("an outcome"))?
                .recorded
                .multiplier,
            1.0
        );
        Ok(())
    }

    #[test]
    fn zen_and_retries_are_not_scored() -> TestResult {
        let zen = Settings {
            mode: Some(Mode::Zen),
            ..Settings::default()
        };
        let mut s = loaded_with(zen, "か", false)?;
        type_timed(&mut s, "ka");
        assert!(!s.outcome().ok_or(TestError::Missing("an outcome"))?.counted);
        assert_eq!(s.settings().ladder, Ladder::default());

        let mut s = loaded_with(scoring(), "か", false)?;
        type_timed(&mut s, "ka");
        let score = s.settings().ladder.score;
        s.handle(named(Key::Escape), ms(0));
        assert_eq!(s.outcome(), None);
        type_timed(&mut s, "ka");
        assert!(!s.outcome().ok_or(TestError::Missing("an outcome"))?.counted);
        assert_eq!(s.settings().ladder.score, score);
        Ok(())
    }

    #[test]
    fn a_new_level_fades_its_helper_from_the_next_sentence_unless_kept() -> TestResult {
        let settings = Settings {
            ladder: Ladder {
                pips: 4,
                ..Ladder::default()
            },
            ..scoring()
        };
        let mut s = loaded_with(settings.clone(), "か", false)?;
        type_timed(&mut s, "ka");
        assert_eq!(
            s.outcome()
                .ok_or(TestError::Missing("an outcome"))?
                .recorded
                .tier_up,
            Some(Tier::Shokyu)
        );
        assert_eq!(s.fade_pending(), Some(Helper::Hint));
        // Still shown on the completed sentence.
        assert!(s.settings().hint);
        s.handle(Event::SentenceLoaded(sentence("か", false)?), ms(0));
        assert!(!s.settings().hint);

        // Kept: it stays.
        let mut s = loaded_with(settings, "か", false)?;
        type_timed(&mut s, "ka");
        s.handle(Event::KeepHelper, ms(0));
        s.handle(Event::SentenceLoaded(sentence("か", false)?), ms(0));
        assert!(s.settings().hint);
        Ok(())
    }

    #[test]
    fn a_rough_sentence_offers_the_last_faded_helper() -> TestResult {
        let settings = Settings {
            ladder: Ladder {
                tier: Tier::Chukyu,
                ..Ladder::default()
            },
            hint: false,
            translation_enabled: false,
            ..scoring()
        };
        let mut s = loaded_with(settings, "か", false)?;
        type_all(&mut s, "xxka");
        assert_eq!(
            s.outcome()
                .ok_or(TestError::Missing("an outcome"))?
                .recorded
                .offer,
            Some(Helper::Translation)
        );
        s.handle(Event::TakeOffer(true), ms(0));
        assert!(s.settings().translation_enabled);
        assert_eq!(
            s.outcome()
                .ok_or(TestError::Missing("an outcome"))?
                .recorded
                .offer,
            None
        );
        Ok(())
    }

    #[test]
    fn choosing_a_level_sets_its_helpers_and_fixes_it() -> TestResult {
        let mut s = loaded_with(scoring(), "か", false)?;
        s.handle(Event::SetTier(Some(Tier::Chukyu)), ms(0));
        let settings = s.settings();
        assert!(settings.ladder.fixed);
        assert!(!settings.hint && !settings.translation_enabled && settings.furigana);
        s.handle(Event::SetTier(Some(Tier::Nyumon)), ms(0));
        assert!(s.settings().hint && s.settings().translation_enabled);
        s.handle(Event::SetTier(None), ms(0));
        assert!(!s.settings().ladder.fixed);
        Ok(())
    }

    #[test]
    fn picking_levels_fades_what_the_level_fades() -> TestResult {
        let settings = Settings {
            mode: Some(Mode::Zen),
            ladder: Ladder {
                tier: Tier::Shokyu,
                ..Ladder::default()
            },
            ..Settings::default()
        };
        let mut s = loaded_with(settings, "か", false)?;
        assert!(s.settings().hint);
        s.handle(Event::SetMode(Mode::Score), ms(0));
        assert!(!s.settings().hint && s.settings().translation_enabled);
        Ok(())
    }

    #[test]
    fn numbers_turn_to_kanji_from_jokyu() {
        let at = |tier, numerals| {
            Session::new(Settings {
                ladder: Ladder {
                    tier,
                    ..Ladder::default()
                },
                numerals,
                ..scoring()
            })
            .kanji_numerals()
        };
        assert!(!at(Tier::Chukyu, Numerals::Auto));
        assert!(at(Tier::Jokyu, Numerals::Auto));
        assert!(!at(Tier::Mushin, Numerals::Arabic));
        assert!(at(Tier::Nyumon, Numerals::Kanji));
    }
    /// Every char, exhaustively: normalizing twice changes nothing, and a
    /// `Char` that survives is printable (control chars became named keys
    /// or `Other`, space became `Space`).
    #[test]
    fn normalize_is_idempotent_for_every_char() {
        for c in (0..=char::MAX as u32).filter_map(char::from_u32) {
            let key = Key::Char(c).normalize();
            assert_eq!(key.normalize(), key, "{c:?}");
            if let Key::Char(c) = key {
                assert!(!c.is_control() && c != ' ', "{c:?}");
            }
        }
    }

    #[test]
    fn wpm_is_finite_and_non_negative() -> TestResult {
        assert_eq!(wpm(10, Duration::ZERO), None);
        for keystrokes in [0, 1, 5, usize::MAX] {
            for elapsed in [
                Duration::from_nanos(1),
                Duration::from_secs(60),
                Duration::MAX,
            ] {
                let w = wpm(keystrokes, elapsed).ok_or(TestError::Missing("a speed"))?;
                assert!(w.is_finite() && w >= 0.0, "{keystrokes} {elapsed:?}: {w}");
            }
        }
        assert_eq!(wpm(5, Duration::from_secs(60)), Some(1.0));
        Ok(())
    }
}
