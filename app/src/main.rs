//! WanType (ワンタイプ) in Leptos (client-side rendering, static site).
//!
//! The game itself is `wantype_core::Session` (sans-IO). This crate is the
//! browser side: it draws the sentence on manuscript paper written down the
//! page, feeds keys and UI actions to the session, and carries out its
//! effects (fetch a sentence, play a recording, save settings).
//!
//! Under the sentence, a red rule and two rows: the play row (the helpers
//! while typing, the result once done; retry and next on the right) and
//! the status row (options, voice, level and score). The options sheet and
//! the first run's choice of mode cover the card.

mod data;
mod genko;
mod js;
mod mobile;
mod text;

use std::sync::Arc;
use std::time::Duration;

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wantype_core::tatoeba::{self, Language};
// Renamed: Leptos has its own `Effect`.
use wantype_core::Effect as Fx;
use wantype_core::{
    fixtures, Event, Helper, Key, Mode, Modifiers, Numerals, Outcome, Restored, Session, Settings,
    Slip, SlipKind, Tier,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_time::Instant;

use js::{JsFailure, Reported};
use text::Numbers;

#[wasm_bindgen(module = "/js/animations.js")]
extern "C" {
    #[wasm_bindgen(js_name = squareMistake)]
    fn square_mistake(el: &web_sys::Element);
}

/// The localStorage key, the app's own: all of an account's GitHub Pages
/// projects share one origin. The format is `Settings`' serde form.
const STORAGE_KEY: &str = "wantype";
/// The source, linked from Pochi in the options.
const REPO: &str = env!("CARGO_PKG_REPOSITORY");

/// Why the settings could not be read or kept.
#[derive(Debug, thiserror::Error)]
enum StorageError {
    #[error("no browser window")]
    NoWindow,
    #[error("this browser has no local storage")]
    Missing,
    #[error("local storage is blocked")]
    Blocked(#[source] JsFailure),
    #[error("the saved settings could not be read")]
    Read(#[source] JsFailure),
    #[error("the settings could not be saved")]
    Write(#[source] JsFailure),
    #[error("the settings could not be written as JSON")]
    Serialize(#[source] serde_json::Error),
}

fn storage() -> Result<web_sys::Storage, StorageError> {
    web_sys::window()
        .ok_or(StorageError::NoWindow)?
        .local_storage()
        .map_err(|e| StorageError::Blocked(e.into()))?
        .ok_or(StorageError::Missing)
}

/// The saved settings, and what of them could not be kept; `None` when
/// none are saved yet, the first run.
fn load_settings() -> Result<Option<Restored>, StorageError> {
    let saved = storage()?
        .get_item(STORAGE_KEY)
        .map_err(|e| StorageError::Read(e.into()))?;
    Ok(saved.map(|json| Settings::from_saved(&json)))
}

/// The browser's preferred languages (`navigator.languages`), most
/// preferred first.
fn browser_languages() -> Vec<String> {
    web_sys::window()
        .map(|w| w.navigator().languages())
        .into_iter()
        .flat_map(|list| list.to_vec())
        .filter_map(|tag| tag.as_string())
        .collect()
}

fn save_settings(settings: &Settings) -> Result<(), StorageError> {
    let json = serde_json::to_string(settings).map_err(StorageError::Serialize)?;
    storage()?
        .set_item(STORAGE_KEY, &json)
        .map_err(|e| StorageError::Write(e.into()))
}

/// DOM `KeyboardEvent.key` as the core sees it: one char is typed text,
/// longer values are key names ("Enter", "ArrowLeft", "Unidentified"…).
fn to_core_key(key: &str) -> Key {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Key::Char(c),
        _ => match key {
            "Enter" => Key::Enter,
            "Tab" => Key::Tab,
            "Escape" => Key::Escape,
            "Backspace" => Key::Backspace,
            "Delete" => Key::Delete,
            _ => Key::Other,
        },
    }
}

/// The key went to a Japanese IME instead of the page: it reports
/// `"Process"` (legacy code 229) or comes inside a composition. MDN's check.
fn is_ime_key(key: &str, composing: bool, key_code: u32) -> bool {
    composing || key == "Process" || key_code == 229
}

/// HTML `lang` for a Tatoeba language code (ISO 639-3), canonicalized by
/// the browser's CLDR data: `eng` → `en`, `cmn` → `zh`, `lvs` → `lv`; codes
/// without a shorter form (`yue`, `jbo`) stay as they are.
fn html_lang(code: &str) -> String {
    // `getCanonicalLocales` throws on malformed tags; Tatoeba codes are plain
    // 2–3 letter language subtags.
    let simple = (2..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase());
    if !simple {
        return code.to_string();
    }
    js_sys::Intl::get_canonical_locales(&JsValue::from_str(code))
        .get(0)
        .as_string()
        .unwrap_or_else(|| code.to_string())
}

/// A control the player reached with the keyboard (Shift+Tab): keys then
/// work natively there instead of driving the game.
fn keyboard_focused_control() -> Option<web_sys::HtmlElement> {
    let active = document().active_element()?;
    let in_controls = active
        .closest(".controls")
        .reported("finding the controls")
        .flatten()
        .is_some();
    let keyboard = active
        .matches(":focus-visible")
        .reported("checking keyboard focus")
        .unwrap_or(false);
    (in_controls && keyboard)
        .then(|| active.dyn_ref::<web_sys::HtmlElement>().cloned())
        .flatten()
}

fn blur_active() {
    let active = document().active_element();
    if let Some(active) = active
        .as_ref()
        .and_then(|a| a.dyn_ref::<web_sys::HtmlElement>())
    {
        active.blur().reported("leaving a control");
    }
}

/// Why a recording did not play.
#[derive(Debug, thiserror::Error)]
enum VoiceError {
    #[error("the page has no audio element yet")]
    NoElement,
    #[error("the browser refused")]
    Refused(#[from] JsFailure),
}

/// A line of text shown for a while. Text and visibility are apart so the
/// text stays while the notice fades out.
#[derive(Clone, Copy)]
struct Notice {
    text: RwSignal<String>,
    shown: RwSignal<bool>,
    /// Hides a flashed notice.
    timer: StoredValue<Option<TimeoutHandle>, LocalStorage>,
}

impl Notice {
    fn new() -> Self {
        Notice {
            text: RwSignal::new(String::new()),
            shown: RwSignal::new(false),
            timer: StoredValue::new_local(None),
        }
    }

    /// Show `text` until hidden.
    fn show(self, text: String) {
        if let Some(timer) = self.timer.write_value().take() {
            timer.clear();
        }
        self.text.set(text);
        self.shown.set(true);
    }

    /// Show `text` for 4 s, as upstream's messages.
    fn flash(self, text: String) {
        self.show(text);
        let shown = self.shown;
        let timer = set_timeout_with_handle(move || shown.set(false), Duration::from_secs(4));
        self.timer.set_value(timer.reported("timing a notice"));
    }
}

/// How to type the kana a key spelled by its sound.
fn slip_note(slip: &Slip) -> String {
    let Slip {
        kana, typed, said, ..
    } = slip;
    match slip.kind {
        SlipKind::Particle => format!("{kana} is typed {typed}, even where it is said {said}"),
        SlipKind::LongVowel => format!("{kana} (long vowel) is typed {typed}"),
        SlipKind::Small => format!("Small {kana} is typed {typed}"),
    }
}

/// The 音声 button does what Space does: turns the voice on, or plays the
/// recording again once on. Shift turns it off, as Shift+Space.
fn voice_click(on: bool, shift: bool) -> Event {
    match (shift, on) {
        (true, _) => Event::SetAudio(false),
        (false, true) => Event::PlayAudio,
        (false, false) => Event::SetAudio(true),
    }
}

/// What the play row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Typing,
    Complete,
    /// A new level was reached with this sentence.
    LevelUp,
    /// A rough sentence; a faded helper is offered back.
    DropBack,
}

/// The session has an outcome exactly while a completed sentence is up.
fn panel(outcome: Option<&Outcome>) -> Panel {
    match outcome.map(|o| o.recorded) {
        None => Panel::Typing,
        Some(r) if r.tier_up.is_some() => Panel::LevelUp,
        Some(r) if r.offer.is_some() => Panel::DropBack,
        Some(_) => Panel::Complete,
    }
}

#[derive(Clone, Copy)]
struct Refs {
    sentence: NodeRef<html::Div>,
    done: NodeRef<html::Button>,
    audio: NodeRef<html::Audio>,
}

/// Everything the event handlers need, all `Copy` handles. Leptos 0.8
/// signals require `Send + Sync`; browser handles (audio element, timer, the
/// data cache) are `StoredValue`s in `LocalStorage` instead. Their
/// `update_value` returns `()`: use `try_update_value` to get a result out.
#[derive(Clone, Copy)]
struct App {
    session: RwSignal<Session>,
    store: data::Store,
    /// The recording the `<audio>` element has its sources for.
    loaded: StoredValue<Option<wantype_core::Audio>, LocalStorage>,
    /// The last sentence request failed, and why (shown in its place).
    load: RwSignal<Result<(), Arc<data::DataError>>>,
    /// Over the 音声 button: the voice actor, or a passing message.
    voice: Notice,
    /// The last key went to a Japanese IME, not the game.
    ime_on: RwSignal<bool>,
    /// How to type the kana a key just spelled by its sound.
    slip: Notice,
    /// Something went wrong the player should know: settings not saved or
    /// not read back.
    problem: Notice,
    /// The mouse moved lately: controls that step back while typing show.
    pointer: RwSignal<bool>,
    pointer_timer: StoredValue<Option<TimeoutHandle>, LocalStorage>,
    options: RwSignal<bool>,
    starter_pending: StoredValue<bool>,
    start: Instant,
    refs: Refs,
}

impl App {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    /// Play `audio` from the start (sources are Tatoeba's URLs, as
    /// upstream). The promise settles once playing, or refused: autoplay
    /// before the first key, or no source that loads.
    fn play(self, audio: wantype_core::Audio) -> Result<js_sys::Promise, VoiceError> {
        let el = self
            .refs
            .audio
            .get_untracked()
            .ok_or(VoiceError::NoElement)?;
        if self.loaded.read_value().as_ref() != Some(&audio) {
            el.replace_children_with_node_0();
            let document = document();
            for url in audio.urls() {
                let source = document.create_element("source").map_err(JsFailure::from)?;
                source.set_attribute("src", &url).map_err(JsFailure::from)?;
                el.append_child(&source).map_err(JsFailure::from)?;
            }
            el.load();
            self.loaded.set_value(Some(audio));
        }
        el.set_current_time(0.0);
        Ok(el.play().map_err(JsFailure::from)?)
    }

    /// Feed an event to the session and carry out the resulting effects.
    fn dispatch(self, event: Event) {
        let before = self.session.with_untracked(|s| s.stats().mistakes);
        let now = self.now();
        let mut effects = Vec::new();
        self.session.update(|s| {
            s.handle(event, now);
            effects.extend(std::iter::from_fn(|| s.poll_effect()));
        });
        if self.session.with_untracked(|s| s.stats().mistakes) > before {
            // The square to type flashes, once redrawn (a first miss marks
            // it).
            let sentence = self.refs.sentence;
            request_animation_frame(move || {
                let square = sentence.get_untracked().and_then(|s| {
                    s.query_selector(".genko-square.active")
                        .reported("finding the square to type")
                        .flatten()
                });
                if let Some(square) = square {
                    square_mistake(&square);
                }
            });
        }
        for effect in effects {
            self.run(effect);
        }
    }

    fn run(self, effect: Fx) {
        match effect {
            Fx::RequestSentence => {
                // Like upstream, the first sentence is the built-in starter;
                // the next ones are fetched meanwhile. (A starter that does
                // not parse is reported, and one is fetched instead.)
                if self.starter_pending.get_value() {
                    self.starter_pending.set_value(false);
                    match fixtures::starter() {
                        Ok(starter) => {
                            self.dispatch(Event::SentenceLoaded(starter));
                            return data::preload(self.store, self.selection());
                        }
                        Err(error) => js::report("the starter sentence", &error),
                    }
                }
                let selection = self.selection();
                spawn_local(async move {
                    match data::next_sentence(self.store, selection).await {
                        Ok(sentence) => {
                            self.load.set(Ok(()));
                            self.dispatch(Event::SentenceLoaded(sentence));
                        }
                        Err(error) => {
                            js::report("loading a sentence", &error);
                            self.load.set(Err(Arc::new(error)));
                            self.dispatch(Event::SentenceFailed);
                        }
                    }
                });
            }
            Fx::PlayAudio(audio) => {
                let voice = self.voice;
                let refused = move |error: VoiceError| {
                    js::report("playing the recording", &error);
                    voice.flash(format!("The recording did not play: {}", js::chain(&error)));
                };
                match self.play(audio) {
                    Ok(promise) => spawn_local(async move {
                        if let Err(error) = JsFuture::from(promise).await {
                            refused(VoiceError::Refused(error.into()));
                        }
                    }),
                    Err(error) => refused(error),
                }
            }
            Fx::NoAudio => self.voice.flash("No audio".into()),
            Fx::Slip(slip) => self.slip.flash(slip_note(&slip)),
            Fx::SettingsChanged => {
                if let Err(error) = self.session.with_untracked(|s| save_settings(s.settings())) {
                    js::report("saving the settings", &error);
                    self.problem.flash(js::chain(&error));
                }
                // Have sentences ready for the new selection, if it changed.
                data::preload(self.store, self.selection());
            }
        }
    }

    fn selection(&self) -> data::Selection {
        self.session
            .with_untracked(|s| data::Selection::new(s.settings()))
    }

    /// The mouse moved: the controls show until it rests for 2 s.
    fn on_pointer(self) {
        if !self.pointer.get_untracked() {
            self.pointer.set(true);
        }
        if let Some(timer) = self.pointer_timer.write_value().take() {
            timer.clear();
        }
        let pointer = self.pointer;
        let timer = set_timeout_with_handle(move || pointer.set(false), Duration::from_secs(2));
        self.pointer_timer
            .set_value(timer.reported("timing the mouse"));
    }

    fn open_options(self, open: bool) {
        self.options.set(open);
        if open {
            let done = self.refs.done;
            request_animation_frame(move || {
                if let Some(done) = done.get_untracked() {
                    done.focus().reported("focusing the options");
                }
            });
        } else {
            blur_active();
        }
    }

    fn on_keydown(self, e: web_sys::KeyboardEvent) {
        // Leave shortcuts (Ctrl/Cmd+…) to the browser.
        if e.ctrl_key() || e.meta_key() {
            return;
        }
        // With the IME on, keys never reach the game: say so instead of
        // seeming frozen.
        let ime = is_ime_key(&e.key(), e.is_composing(), e.key_code());
        if self.ime_on.get_untracked() != ime {
            self.ime_on.set(ime);
        }
        if ime {
            return;
        }
        // The first run: 1 or 2 picks the mode; Tab and Enter work on its
        // two buttons natively.
        if self.session.with_untracked(|s| s.settings().mode.is_none()) {
            let mode = match e.key().as_str() {
                "1" => Mode::Score,
                "2" => Mode::Zen,
                _ => return,
            };
            e.prevent_default();
            return self.dispatch(Event::SetMode(mode));
        }
        // The options sheet: its controls work natively; Escape closes it.
        if self.options.get_untracked() {
            if e.key() == "Escape" {
                e.prevent_default();
                self.open_options(false);
            }
            return;
        }
        // Keyboard access to the controls: Shift+Tab moves into them, keys
        // work natively there, Escape returns to the game. (Tab itself
        // toggles furigana, as upstream.)
        if let Some(control) = keyboard_focused_control() {
            if e.key() == "Escape" {
                control.blur().reported("leaving a control");
                e.prevent_default();
            }
            return;
        }
        if e.key() == "Tab" && e.shift_key() {
            return;
        }
        let key = to_core_key(&e.key());
        // Keep focus, quick-find and page scrolling out of the way.
        if matches!(
            key.normalize(),
            Key::Tab | Key::Enter | Key::Space | Key::Char(_)
        ) {
            e.prevent_default();
        }
        let mods = Modifiers {
            ctrl: false,
            alt: e.alt_key(),
            meta: false,
            shift: e.shift_key(),
        };
        self.dispatch(Event::Key(key, mods));
    }
}

/// A sparkle (✧) of Pochi's, centred on the origin.
const SPARKLE: &str = "M0 -3.6Q0.5 -0.5 3.6 0Q0.5 0.5 0 3.6Q-0.5 0.5 -3.6 0Q-0.5 -0.5 0 -3.6Z";

/// Pochi, the shiba of the favicon (▼・ᴥ・▼✧), in the ink of the text.
#[component]
fn Pochi() -> impl IntoView {
    view! {
        // The favicon's drawing without its tile, cropped to it.
        <svg width="36" height="34" viewBox="1.5 0.5 29 27.5" aria-hidden="true">
            <path d="M2.5 10h7l-3.5 6.5z M22.5 10h7l-3.5 6.5z" fill="currentColor"
                stroke="currentColor" stroke-width="1" stroke-linejoin="round" />
            <circle cx="12.3" cy="14" r="1.5" fill="currentColor" />
            <circle cx="19.7" cy="14" r="1.5" fill="currentColor" />
            <path d="M13.4 17.2h5.2l-2.6 2.4z" fill="currentColor" stroke="currentColor"
                stroke-width="2" stroke-linejoin="round" />
            <path d="M16 19.6v1.2M13 21.6c1.2 1.6 3 1.6 3 -0.8c0 2.4 1.8 2.4 3 0.8" fill="none"
                stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
            <path class="sparkle" transform="translate(26 4.8)" d=SPARKLE />
            <path class="sparkle" transform="translate(5.8 26.5) scale(0.55)" d=SPARKLE />
        </svg>
    }
}

/// A helper as the play row shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelperState {
    On,
    /// Off with the level: it can come back (a dashed box with a plus).
    Faded,
    /// Off by choice (an empty box).
    Off,
}

/// What a helper does to the score, in one slot as wide as the widest note,
/// so toggling never changes the button's width. (倍, not ×: in Japan × is
/// batsu, "wrong".)
const HELPER_NOTES: [&str; 2] = ["+0.5倍", "−0.5倍"];

/// A helper's toggle in the play row, with what it does to the score
/// beside it.
#[component]
fn HelperButton(
    app: App,
    helper: Helper,
    scoring: Memo<bool>,
    legends: Signal<bool>,
) -> impl IntoView {
    let state = Memo::new(move |_| {
        app.session
            .with(|s| match (s.settings().helper(helper), s.faded(helper)) {
                (true, _) => HelperState::On,
                (false, true) => HelperState::Faded,
                (false, false) => HelperState::Off,
            })
    });
    let (label, japanese) = text::helper_label(helper);
    let class = move || match state.get() {
        HelperState::Faded => "button helper faded",
        HelperState::On | HelperState::Off => "button helper",
    };
    let faded = move || app.session.with_untracked(|s| s.faded(helper));
    let note = move || match (scoring.get(), state.get()) {
        (false, _) => "",
        (true, HelperState::On) if faded() => "−0.5倍",
        // Faded by the level: the dashed box says so, the score is as set.
        (true, HelperState::On | HelperState::Faded) => "",
        (true, HelperState::Off) => "+0.5倍",
    };
    let on = move || state.get() == HelperState::On;
    view! {
        <button
            type="button"
            class=class
            aria-label=label
            aria-pressed=move || on().to_string()
            on:click=move |_| app.dispatch(Event::SetHelper(helper, !on()))
        >
            {move || legends.get().then(|| view! { <span class="legend">{label}</span> })}
            <ToggleBox state=state />
            <span lang="ja">{japanese}</span>
            {move || scoring.get().then(|| view! {
                <span class="note slot">
                    {HELPER_NOTES
                        .map(|n| view! { <span class:shown=move || note() == n>{n}</span> })
                        .collect_view()}
                </span>
            })}
            {(helper == Helper::Furigana).then(|| view! { <kbd class="key">"tab"</kbd> })}
        </button>
    }
}

/// A toggle's box, drawn like a square of the sheet: vermilion with a maru
/// (〇, "yes"; a ✓ in Japan marks what is wrong or to check) when on, empty
/// when off, dashed with a plus when faded. The shape tells the state, the
/// colour only adds to it.
#[component]
fn ToggleBox(#[prop(into)] state: Signal<HelperState>) -> impl IntoView {
    let mark = move || match state.get() {
        HelperState::On => Some(view! { <circle cx="12" cy="12" r="6"></circle> }.into_any()),
        HelperState::Faded => Some(view! { <path d="M12 7v10M7 12h10"></path> }.into_any()),
        HelperState::Off => None,
    };
    view! {
        <span class="box" aria-hidden="true">
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round">
                {mark}
            </svg>
        </span>
    }
}

/// A `<select>` over `choices` (value, its key, its label).
fn choice_select<T: Copy + PartialEq + Send + Sync + 'static>(
    id: &'static str,
    choices: &'static [(T, &'static str, &'static str)],
    current: impl Fn() -> T + Send + Sync + 'static,
    choose: impl Fn(T) + 'static,
) -> impl IntoView {
    let key = move || {
        let current = current();
        choices
            .iter()
            .find(|(c, ..)| *c == current)
            .map_or("", |(_, key, _)| *key)
    };
    view! {
        <select
            id=id
            prop:value=key
            on:change=move |e| {
                let value = event_target_value(&e);
                if let Some((c, ..)) = choices.iter().find(|(_, key, _)| *key == value) {
                    choose(*c);
                }
            }
        >
            {choices.iter().map(|(_, key, label)| view! { <option value=*key>{*label}</option> }).collect_view()}
        </select>
    }
}

#[component]
fn Game() -> impl IntoView {
    // Settings that could not be read back take their defaults, and the
    // player is told. Without saved ones (or a storage to keep them), it is
    // a first run.
    let (session, problems): (Session, Vec<String>) = match load_settings() {
        Ok(Some(Restored { settings, problems })) => (
            Session::new(settings),
            problems.iter().map(js::chain).collect(),
        ),
        Ok(None) => (Session::first_run(), Vec::new()),
        Err(error) => (Session::first_run(), vec![js::chain(&error)]),
    };
    let app = App {
        session: RwSignal::new(session),
        store: StoredValue::new_local(data::Data::default()),
        loaded: StoredValue::new_local(None),
        load: RwSignal::new(Ok(())),
        voice: Notice::new(),
        ime_on: RwSignal::new(false),
        slip: Notice::new(),
        problem: Notice::new(),
        pointer: RwSignal::new(false),
        pointer_timer: StoredValue::new_local(None),
        options: RwSignal::new(false),
        starter_pending: StoredValue::new(true),
        start: Instant::now(),
        refs: Refs {
            sentence: NodeRef::new(),
            done: NodeRef::new(),
            audio: NodeRef::new(),
        },
    };
    for problem in &problems {
        leptos::logging::warn!("{problem}");
    }
    if !problems.is_empty() {
        app.problem.flash(problems.join("; "));
    }
    let session = app.session;
    // The translation languages; a failed list says why in the options.
    let languages: RwSignal<Result<Vec<Language>, Arc<data::DataError>>> =
        RwSignal::new(Ok(Vec::new()));

    // Answer the session's initial sentence request.
    let mut initial = Vec::new();
    session.update(|s| initial.extend(std::iter::from_fn(|| s.poll_effect())));
    initial.into_iter().for_each(|e| app.run(e));

    spawn_local(async move {
        match data::languages(app.store).await {
            Ok(list) => {
                app.dispatch(Event::LanguagesLoaded {
                    available: list.iter().map(|l| l.lang.clone()).collect(),
                    browser: tatoeba::browser_language(&browser_languages(), &list, html_lang)
                        .map(String::from),
                });
                languages.set(Ok(list));
            }
            Err(error) => {
                js::report("loading the translation languages", &error);
                languages.set(Err(Arc::new(error)));
            }
        }
    });

    let listener = window_event_listener(ev::keydown, move |e| app.on_keydown(e));
    on_cleanup(move || listener.remove());
    let listener = window_event_listener(ev::mousemove, move |_| app.on_pointer());
    on_cleanup(move || listener.remove());

    // Narrow memos: the session changes on every key, these only when what
    // they hold does. (Copying the settings shares their known readings.)
    let sentence = Memo::new(move |_| session.with(|s| s.sentence().cloned()));
    let segments = Memo::new(move |_| session.with(|s| s.segments()));
    let settings = Memo::new(move |_| session.with(|s| s.settings().clone()));
    let completed = Memo::new(move |_| session.with(Session::completed));
    let stats = Memo::new(move |_| session.with(Session::stats));
    let outcome = Memo::new(move |_| session.with(|s| s.outcome().cloned()));
    let shown = Memo::new(move |_| session.with(Session::helpers));
    let scoring = Memo::new(move |_| session.with(Session::scoring));
    let multiplier = Memo::new(move |_| session.with(Session::multiplier));
    let fade_pending = Memo::new(move |_| session.with(Session::fade_pending));
    let tier = Memo::new(move |_| settings.with(|s| s.ladder.tier));
    let numbers = Memo::new(move |_| Numbers {
        kanji: session.with(Session::kanji_numerals),
    });
    let panel = Memo::new(move |_| outcome.with(|o| panel(o.as_ref())));
    // The interface thins as the player climbs: English labels and live
    // stats go at 中級, helpers that faded and stayed off at 上級, and at
    // 無心 everything while typing (the mouse or Shift+Tab brings it back).
    let reached = move |level: Tier| scoring.get() && tier.get() >= level;
    let legends = Signal::derive(move || !reached(Tier::Chukyu));
    let quiet = Memo::new(move |_| {
        reached(Tier::Mushin) && matches!(panel.get(), Panel::Typing | Panel::Complete)
    });

    // Body classes: what shows (furigana, the hint, the translation), the
    // font, and when the controls step back: in Zen while typing and at
    // 無心, unless the mouse moves.
    Effect::new(move |_| {
        let helpers = shown.get();
        let zen = settings.with(|s| s.mode == Some(Mode::Zen));
        let classes = [
            ("furigana", helpers.furigana),
            ("hint", helpers.hint),
            ("translation", helpers.translation),
            ("mincho", settings.with(|s| s.mincho)),
            ("step-back", (zen && !completed.get()) || quiet.get()),
            ("pointer", app.pointer.get()),
        ];
        if let Some(body) = document().body() {
            let list = body.class_list();
            for (name, on) in classes {
                list.toggle_with_force(name, on)
                    .reported("setting a page class");
            }
        }
    });

    // Reveal the page once fonts are ready.
    spawn_local(async move {
        // Without them the page shows anyway, in fallback fonts.
        if let Some(ready) = document().fonts().ready().reported("waiting for the fonts") {
            JsFuture::from(ready)
                .await
                .reported("waiting for the fonts");
        }
        show_page();
    });

    // The key hint: a letter only with furigana and the hint on.
    let hint = Memo::new(move |_| {
        session
            .with(Session::hint)
            .filter(|_| shown.with(|h| h.hint))
    });
    // Built once per sentence; its squares follow the progress themselves.
    let sentence_body = move || {
        sentence.track();
        let lines = segments.with_untracked(|segs| genko::lines(segs, genko::WIDTH));
        genko::view(lines, genko::WIDTH, segments, hint.into())
    };
    // The next key, for tests and assistive tools; shown or not.
    let next_key = move || session.with(|s| s.hint().map(String::from).unwrap_or_default());
    // The translation shown, in the chosen language or else in English,
    // with its `lang`.
    let language = Memo::new(move |_| settings.with(|s| s.language.clone()));
    let shown_translation = Memo::new(move |_| {
        let language = language.read();
        if language.is_empty() {
            return None;
        }
        sentence.with(|s| {
            let (lang, text) = s.as_ref()?.translation(&language)?;
            Some((html_lang(lang), text.to_string()))
        })
    });
    let translation = move || {
        shown_translation.with(|t| t.as_ref().map(|(_, text)| text.clone()).unwrap_or_default())
    };
    let translation_lang =
        move || shown_translation.with(|t| t.as_ref().map(|(lang, _)| lang.clone()));
    let tatoeba_id = move || sentence.with(|s| s.as_ref().and_then(|s| s.id.clone()));
    let refs = app.refs;

    let stats_text = move || {
        let n = numbers.get();
        stats.with(|s| match (s.wpm, s.accuracy()) {
            (Some(wpm), Some(accuracy)) => n.stats(wpm, accuracy),
            _ => String::new(),
        })
    };

    // THE PLAY ROW

    let helper_row = move || {
        // From 上級, helpers that are off leave the row.
        let away = reached(Tier::Jokyu);
        let listed: Vec<Helper> = Helper::ALL
            .into_iter()
            .rev()
            .filter(|&h| !away || settings.with(|s| s.helper(h)))
            .collect();
        let all_away = listed.is_empty();
        let buttons = listed
            .into_iter()
            .map(|h| view! { <HelperButton app helper=h scoring legends /> })
            .collect_view();
        view! {
            <div class="helpers">
                {buttons}
                {all_away.then(|| view! {
                    <button type="button" class="button helper faded" aria-label="Helpers"
                        on:click=move |_| app.open_options(true)>
                        <ToggleBox state=Signal::stored(HelperState::Faded) />
                        <span lang="ja">"補助"</span>
                    </button>
                })}
            </div>
        }
    };

    let complete_row = move |o: Outcome| {
        let n = numbers.get();
        let r = o.recorded;
        let (gain, sub, progress) = match scoring.get() {
            false => (
                "完了".to_string(),
                n.stats(o.wpm, o.accuracy),
                "No score in Zen. Next one when you are ready.".to_string(),
            ),
            true => (
                format!("+{}", n.int(r.gain)),
                format!(
                    "{} · {}",
                    n.stats(o.wpm, o.accuracy),
                    n.multiplier(r.multiplier)
                ),
                match o.counted {
                    true => settings.with(|s| text::progress(&s.ladder, s.numerals)),
                    false => "A retry is not scored again.".to_string(),
                },
            ),
        };
        view! {
            <div class="result">
                <span class="result-line">
                    <span class="gain">{gain}</span>
                    <span class="sub">{sub}</span>
                </span>
                <span class="progress">{progress}</span>
            </div>
        }
    };

    let level_up_row = move |o: Outcome| {
        let n = numbers.get();
        let tier = tier.get();
        let pending = fade_pending.get();
        // Read when the row redraws: the helpers and the pending fade are
        // tracked through `multiplier` and `fade_pending`.
        multiplier.track();
        let with = |h, on| session.with_untracked(|s| s.multiplier_with(h, on));
        // This level is the first written in kanji numbers.
        let numerals_now = Tier::ALL.into_iter().find(|t| t.kanji_numerals()) == Some(tier)
            && settings.with(|s| s.numerals == Numerals::Auto);
        let line = match (tier.is_mushin(), pending, tier.fades()) {
            (true, ..) => "From the next sentence nothing is shown. Move the mouse or press \
                           Shift+Tab to see the controls."
                .to_string(),
            (false, Some(h), _) => format!(
                "{} fades from the next sentence{} · score now {}",
                text::helper_label(h).0,
                if numerals_now {
                    " · numbers are now 漢数字"
                } else {
                    ""
                },
                n.multiplier(with(h, false))
            ),
            (false, None, Some(h)) => format!(
                "{} stays on · score {}",
                text::helper_label(h).0,
                n.multiplier(multiplier.get())
            ),
            (false, None, None) => String::new(),
        };
        view! {
            <div class="result">
                <span class="headline">
                    "Reached " <span lang="ja">{tier.japanese()}</span> " " {tier.english()}
                    <span class="sub">{format!("+{} · tier bonus included", n.int(o.recorded.gain))}</span>
                </span>
                <span class="progress">
                    {line}
                    {pending.map(|h| view! {
                        " · "
                        <button type="button" class="inline-button"
                            on:click=move |_| app.dispatch(Event::KeepHelper)>
                            <span lang="ja">{format!("{}を残す", text::helper_label(h).1)}</span>
                            {format!(" (keep {})", text::helper_name(h))}
                        </button>
                    })}
                </span>
            </div>
        }
    };

    let drop_back_row = move |o: Outcome, offer: Helper| {
        let n = numbers.get();
        let back = session.with_untracked(|s| s.multiplier_with(offer, true));
        view! {
            <div class="result">
                <span class="headline">
                    {format!("{}, a rough one", n.percent(o.accuracy))}
                    <span class="sub">{format!("+{} · streak reset", n.int(o.recorded.gain))}</span>
                </span>
                <span class="progress">
                    {format!(
                        "{} can come back for a while · {} → {} while it is on",
                        text::helper_label(offer).0,
                        n.multiplier(multiplier.get()),
                        n.multiplier(back)
                    )}
                </span>
            </div>
            <div class="offer">
                <button type="button" class="button compact accept"
                    on:click=move |_| app.dispatch(Event::TakeOffer(true))>
                    <span class="legend">{format!("Show {}", text::helper_name(offer))}</span>
                    <span lang="ja">{format!("{}を戻す", text::helper_label(offer).1)}</span>
                </button>
                <button type="button" class="button compact"
                    on:click=move |_| app.dispatch(Event::TakeOffer(false))>
                    <span class="legend">"Not now"</span>
                    <span lang="ja">"今はいい"</span>
                </button>
            </div>
        }
    };

    let play_main = move || match (panel.get(), outcome.get()) {
        (Panel::Complete, Some(o)) => complete_row(o).into_any(),
        (Panel::LevelUp, Some(o)) => level_up_row(o).into_any(),
        (Panel::DropBack, Some(o)) => match o.recorded.offer {
            Some(offer) => drop_back_row(o, offer).into_any(),
            None => ().into_any(),
        },
        _ => helper_row().into_any(),
    };

    // THE STATUS ROW

    let pips = move || {
        let filled = settings.with(|s| s.ladder.pips);
        (0..wantype_core::ladder::PIPS)
            .map(|i| view! { <span class="pip" class:full=i < filled></span> })
            .collect_view()
    };
    let level = move || match scoring.get() {
        true => view! {
            <span class="tier-name">
                <span lang="ja">{move || tier.get().japanese()}</span>
                " "
                <span class="tier-english">{move || tier.get().english()}</span>
            </span>
            <span
                class="pips"
                role="img"
                aria-label=move || settings.with(|s| format!(
                    "{} of {} clean sentences toward the next level",
                    s.ladder.pips,
                    wantype_core::ladder::PIPS
                ))
            >
                {pips}
            </span>
        }
        .into_any(),
        false => view! {
            <span class="tier-name"><span lang="ja">"禅"</span> " Zen"</span>
            <span class="aside">"no score · no tiers"</span>
        }
        .into_any(),
    };
    let score = move || {
        scoring.get().then(|| {
            view! {
                <span class="score-line">
                    {move || numbers.get().score(settings.with(|s| s.ladder.score))}
                    <span class="chip">{move || numbers.get().multiplier(multiplier.get())}</span>
                </span>
                <span class="aside">
                    {move || numbers.get().streak(settings.with(|s| s.ladder.streak))}
                </span>
            }
        })
    };
    // Its line stays, hidden when there is nothing to show, so the score
    // column never moves.
    let live_stats = move || {
        view! {
            <span class="aside" class:unseen=move || completed.get() || !legends.get()>
                {stats_text}
            </span>
        }
    };

    // Buttons say what they do in Japanese, as verbs (SmartHR); the English
    // is the legend above, like the helpers'.
    let next_legend = move || {
        if completed.get() {
            "Next sentence"
        } else {
            "Skip"
        }
    };

    let voice_on = move || settings.with(|s| s.audio);

    view! {
        <main>
            <div id="imeNotice" class="notice" role="status" class:hidden=move || !app.ime_on.get()>
                <span lang="ja">"日本語入力がオンです。半角英数に切り替えてください"</span>
                <span lang="en">"Japanese input (IME) is on: switch to direct input"</span>
            </div>
            <div id="slipNote" class="notice" role="status" lang="en" class:hidden=move || !app.slip.shown.get()>
                {move || app.slip.text.get()}
            </div>
            <div id="problemNote" class="notice" role="alert" lang="en" class:hidden=move || !app.problem.shown.get()>
                {move || app.problem.text.get()}
            </div>

            // A sentence that could not load says why in its place, until
            // the next one does (Enter tries again).
            <ErrorBoundary fallback=|errors| view! {
                <div class="load-error" role="alert" lang="en">
                    <span class="load-error-title">"The next sentence could not load"</span>
                    {move || errors
                        .get()
                        .into_iter()
                        .map(|(_, error)| view! { <span class="faint">{js::chain(&**error)}</span> })
                        .collect_view()}
                    <span class="faint">"Press Enter to try again."</span>
                </div>
            }>
                {move || app.load.get()}
                <div id="sentenceContainer" class:complete=completed data-next=next_key>
                    <div id="sentenceElement" node_ref=refs.sentence>{sentence_body}</div>
                    <div id="translationElement" dir="auto" lang=translation_lang>
                        {translation}
                    </div>
                </div>
            </ErrorBoundary>

            // The recordings: the voice actor shows while one plays
            // (upstream behaviour).
            <audio
                node_ref=refs.audio
                on:play=move |_| {
                    let author = app.loaded.read_value().as_ref().map(|a| a.author.clone());
                    app.voice.show(author.unwrap_or_default());
                }
                on:pause=move |_| app.voice.shown.set(false)
                on:ended=move |_| app.voice.shown.set(false)
            ></audio>

            {move || (quiet.get() && completed.get()).then(|| view! {
                <div class="seal" role="img"
                    aria-label=move || format!("無心 · {}", settings.with(|s| s.ladder.score))>
                    <span lang="ja" class="seal-mark">"無心"</span>
                    <span lang="ja" class="seal-score">
                        {move || Numbers { kanji: true }.score(settings.with(|s| s.ladder.score))}
                    </span>
                </div>
            })}

            <div class="controls" lang="en">
                <div class="play-row">
                    <div class="play-main">{play_main}</div>
                    <div class="play-buttons">
                        <button
                            id="restartButton"
                            type="button"
                            class="button"
                            on:click=move |_| app.dispatch(Event::Restart)
                        >
                            {move || legends.get().then(|| view! { <span class="legend">"Retry"</span> })}
                            <span lang="ja">"やり直す"</span>
                            <kbd class="key">"escape"</kbd>
                        </button>
                        <button
                            id="nextSentenceButton"
                            type="button"
                            class="button"
                            class:ready=completed
                            on:click=move |_| app.dispatch(Event::Next)
                        >
                            {move || legends.get().then(|| view! { <span class="legend">{next_legend}</span> })}
                            // Both labels in one slot: the button never changes width.
                            <span lang="ja" class="slot">
                                <span class:shown=move || !completed.get()>"飛ばす"</span>
                                <span class:shown=completed>"次へ"</span>
                            </span>
                            <kbd class="key">"enter"</kbd>
                        </button>
                    </div>
                </div>

                <div class="status-row">
                    <button
                        id="optionsButton"
                        type="button"
                        class="button icon-button"
                        aria-label="Options"
                        aria-haspopup="dialog"
                        on:click=move |_| app.open_options(true)
                    >
                        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                            stroke-width="1.8" stroke-linecap="round" aria-hidden="true">
                            <path d="M4 7h9M17 7h3M4 17h3M11 17h9"></path>
                            <circle cx="15" cy="7" r="2"></circle>
                            <circle cx="9" cy="17" r="2"></circle>
                        </svg>
                    </button>
                    <button
                        id="voiceButton"
                        type="button"
                        class="button helper"
                        class:on=voice_on
                        // Not a toggle: once on, a press plays the recording
                        // again, so the label says the state instead.
                        aria-label=move || if voice_on() { "Voice, on: play again" } else { "Voice, off: turn on" }
                        title="Space plays the recording again; Shift turns the voice off"
                        on:click=move |e: ev::MouseEvent| app.dispatch(voice_click(voice_on(), e.shift_key()))
                    >
                        {move || legends.get().then(|| view! { <span class="legend">"Voice"</span> })}
                        <div id="sentenceVoiceContainer" class:hidden=move || !app.voice.shown.get()>
                            // The speaker: a bust and the sound of a voice.
                            <svg class="speaker" width="16" height="16" viewBox="0 0 24 24" fill="none"
                                stroke="currentColor" stroke-width="1.8" stroke-linecap="round"
                                stroke-linejoin="round" aria-hidden="true">
                                <circle cx="9" cy="8" r="3.5"></circle>
                                <path d="M2.5 20.5a6.5 6.5 0 0 1 13 0"></path>
                                <path d="M17.5 6.5a4 4 0 0 1 0 5"></path>
                                <path d="M20 4a7.5 7.5 0 0 1 0 10"></path>
                            </svg>
                            <span id="sentenceVoiceElement">{move || app.voice.text.get()}</span>
                        </div>
                        <svg class="mark" width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                            stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                            <path d="M4 9.5h3.5L12 6v12l-4.5-3.5H4z"></path>
                            <path d="M15.5 9a4 4 0 0 1 0 6"></path>
                            <path class="wave" d="M18 6.5a7.5 7.5 0 0 1 0 11"></path>
                        </svg>
                        <span lang="ja">"音声"</span>
                        <span class="note slot">
                            <span class:shown=voice_on>"on"</span>
                            <span class:shown=move || !voice_on()>"off"</span>
                        </span>
                        <kbd class="key">"space"</kbd>
                    </button>
                    <div class="tier">{level}</div>
                    <div class="score">{score}{live_stats}</div>
                </div>
            </div>

            <Options app settings scoring numbers languages tatoeba_id=Signal::derive(tatoeba_id) />
            <Welcome app />
        </main>
    }
}

const FONTS: &[(bool, &str, &str)] = &[
    (false, "gothic", "ゴシック Gothic"),
    (true, "mincho", "明朝 Mincho"),
];
const NUMERALS: &[(Numerals, &str, &str)] = &[
    (Numerals::Auto, "auto", "漢数字 from 上級"),
    (Numerals::Arabic, "arabic", "Always 1 2 3"),
    (Numerals::Kanji, "kanji", "Always 一 二 三"),
];
const NEW_KANJI: &[(Option<u8>, &str, &str)] = &[
    (Some(0), "0", "None"),
    (Some(1), "1", "One"),
    (None, "any", "Any"),
];

/// The options sheet: progress, helpers, display and play.
#[component]
fn Options(
    app: App,
    settings: Memo<Settings>,
    scoring: Memo<bool>,
    numbers: Memo<Numbers>,
    languages: RwSignal<Result<Vec<Language>, Arc<data::DataError>>>,
    tatoeba_id: Signal<Option<String>>,
) -> impl IntoView {
    let session = app.session;
    let zen = move || settings.with(|s| s.mode == Some(Mode::Zen));
    let fixed = move || settings.with(|s| s.ladder.fixed);

    let tier_rows = Tier::ALL
        .into_iter()
        .map(|t| {
            let current = move || settings.with(|s| s.ladder.tier == t);
            let multiplier = move || {
                numbers
                    .get()
                    .multiplier(wantype_core::ladder::multiplier(t.helpers(), t))
            };
            view! {
                <label class="tier-row" class:current=current>
                    <input
                        type="radio"
                        name="tier"
                        prop:checked=current
                        on:change=move |_| app.dispatch(Event::SetTier(Some(t)))
                    />
                    <span>
                        <span lang="ja">{t.japanese()}</span>
                        " "
                        <span class="faint">{t.english()}</span>
                    </span>
                    <span class="faint">{text::tier_description(t)} " · " {multiplier}</span>
                </label>
            }
        })
        .collect_view();

    let helper_rows = Helper::ALL
        .into_iter()
        .rev()
        .map(|h| {
            let (label, japanese) = text::helper_label(h);
            let on = move || settings.with(|s| s.helper(h));
            let note = move || {
                let tier = settings.with(|s| s.ladder.tier);
                match (scoring.get(), on(), h.faded(tier)) {
                    (false, ..) => "Stays as you set it".to_string(),
                    (true, true, false) => format!("On at {}", tier.japanese()),
                    (true, true, true) => "Brought back · −0.5倍".to_string(),
                    (true, false, true) => format!("Faded at {}", h.fades_at().japanese()),
                    (true, false, false) => "Off by choice · +0.5倍".to_string(),
                }
            };
            view! {
                <label class="choice">
                    <input
                        type="checkbox"
                        prop:checked=on
                        on:change=move |e| app.dispatch(Event::SetHelper(h, event_target_checked(&e)))
                    />
                    <span>
                        <span lang="ja">{japanese}</span> " " {label}
                        <br />
                        <span class="faint">{note}</span>
                    </span>
                </label>
            }
        })
        .collect_view();

    view! {
        <div
            class="sheet"
            role="dialog"
            aria-modal="true"
            aria-labelledby="optionsTitle"
            lang="en"
            hidden=move || !app.options.get()
        >
            <div class="sheet-head">
                <h2 id="optionsTitle"><span lang="ja">"設定"</span> " " <span class="faint">"Options"</span></h2>
                <button type="button" class="button" node_ref=app.refs.done
                    on:click=move |_| app.open_options(false)>
                    <span lang="ja">"閉じる"</span>
                    <kbd class="key">"escape"</kbd>
                </button>
            </div>
            <div class="sheet-columns">
                <div class="group">
                    <span class="group-title"><span lang="ja">"進み方"</span> " Progress"</span>
                    <label class="choice">
                        <input type="radio" name="progress" prop:checked=move || !fixed()
                            on:change=move |_| app.dispatch(Event::SetTier(None)) />
                        <span>
                            "Auto"
                            <br />
                            <span class="faint">"Helpers fade as you improve and come back after rough patches"</span>
                        </span>
                    </label>
                    <label class="choice">
                        <input type="radio" name="progress" prop:checked=fixed
                            on:change=move |_| {
                                let tier = session.with_untracked(|s| s.settings().ladder.tier);
                                app.dispatch(Event::SetTier(Some(tier)));
                            } />
                        <span>"Fixed level"</span>
                    </label>
                    <div class="tier-rows">{tier_rows}</div>
                </div>

                <div class="group">
                    <span class="group-title"><span lang="ja">"補助"</span> " Helpers · scored"</span>
                    {helper_rows}
                    <span class="faint">"A faded helper you turn back on costs its 0.5倍 while it stays on."</span>
                    <span class="group-title"><span lang="ja">"音"</span> " Sound · not scored"</span>
                    <label class="choice">
                        <input
                            type="checkbox"
                            id="audioCheckbox"
                            prop:checked=move || settings.with(|s| s.audio)
                            on:change=move |e| app.dispatch(Event::SetAudio(event_target_checked(&e)))
                        />
                        <span>
                            <span lang="ja">"音声"</span> " Voice"
                            <br />
                            <span class="faint">"Reads each sentence aloud. Space switches it any time."</span>
                        </span>
                    </label>
                </div>

                <div class="group">
                    <span class="group-title"><span lang="ja">"表示"</span> " Display"</span>
                    <label class="select-row">
                        "Font"
                        {choice_select("fontSelect", FONTS, move || settings.with(|s| s.mincho),
                            move |mincho| app.dispatch(Event::SetMincho(mincho)))}
                    </label>
                    <ErrorBoundary fallback=|errors| view! {
                        <span class="faint" role="alert">
                            "The translation languages could not load: "
                            {move || errors
                                .get()
                                .into_iter()
                                .map(|(_, error)| js::chain(&**error))
                                .collect::<Vec<_>>()
                                .join("; ")}
                        </span>
                    }>
                    {move || languages.with(|l| l.as_ref().map(|_| ()).map_err(Arc::clone))}
                    <label class="select-row">
                        "Translate into"
                        <select
                            id="languageSelect"
                            prop:value=move || {
                                languages.track();
                                settings.with(|s| s.language.clone())
                            }
                            on:change=move |e| app.dispatch(Event::SetLanguage(event_target_value(&e)))
                        >
                            <option value="">"None"</option>
                            <For
                                each=move || languages.get().into_iter().flatten()
                                key=|l| l.lang.clone()
                                children=|l| view! { <option value=l.lang.clone()>{l.name.clone()}</option> }
                            />
                        </select>
                    </label>
                    </ErrorBoundary>
                    <label class="select-row">
                        "Numbers"
                        {choice_select("numeralsSelect", NUMERALS, move || settings.with(|s| s.numerals),
                            move |n| app.dispatch(Event::SetNumerals(n)))}
                    </label>

                    <span class="group-title"><span lang="ja">"遊び方"</span> " Play"</span>
                    <label class="select-row">
                        "New kanji per sentence"
                        {choice_select("newKanjiSelect", NEW_KANJI, move || settings.with(|s| s.unknown_readings),
                            move |n| app.dispatch(Event::SetUnknownReadings(n)))}
                    </label>
                    <span class="faint">"A kanji counts as known once you type it with furigana off."</span>
                    <span class="group-subtitle"><span lang="ja">"モード"</span> " Mode"</span>
                    <label class="choice">
                        <input type="radio" name="mode" prop:checked=move || scoring.get()
                            on:change=move |_| app.dispatch(Event::SetMode(Mode::Score)) />
                        <span><span lang="ja">"級"</span> " Levels and score"</span>
                    </label>
                    <label class="choice">
                        <input type="radio" name="mode" id="zenRadio" prop:checked=zen
                            on:change=move |_| app.dispatch(Event::SetMode(Mode::Zen)) />
                        <span>
                            <span lang="ja">"禅"</span> " Zen"
                            <br />
                            <span class="faint">"No score, no levels; helpers stay as you set them"</span>
                        </span>
                    </label>
                </div>
            </div>
            <nav class="links">
                <span id="logo">"WanType" <span class="faint" lang="ja">"ワンタイプ"</span></span>
                <a class="pochi" target="_blank" href=REPO title="WanType on GitHub" aria-label="WanType on GitHub">
                    <Pochi />
                </a>
                <a
                    id="sentenceOriginLink"
                    class="button"
                    target="_blank"
                    href=move || tatoeba_id.get().map(|id| format!("https://tatoeba.org/en/sentences/show/{id}"))
                >
                    <span>"この文を" <span class="bold">"Tatoeba"</span> "で見る"</span>
                </a>
                <p class="credits" lang="en">
                    "A Rust rewrite of "
                    <a target="_blank" href="https://github.com/entibo/taipingu">"Taipingu"</a>
                    " by Thibaut Guénédal · "
                    <a target="_blank" href="LICENSE.txt">"MIT licence"</a>
                    " · Sentences from "
                    <a target="_blank" href="https://tatoeba.org">"Tatoeba"</a>
                    ", "
                    <a target="_blank" href="https://creativecommons.org/licenses/by/2.0/fr/">"CC BY 2.0 FR"</a>
                    "; recordings under their speakers' licences"
                </p>
            </nav>
        </div>
    }
}

/// The first run: how to play, kept on the device.
#[component]
fn Welcome(app: App) -> impl IntoView {
    let unpicked = move || app.session.with(|s| s.settings().mode.is_none());
    view! {
        <div class="sheet welcome" role="dialog" aria-labelledby="welcomeTitle" lang="en" hidden=move || !unpicked()>
            <div class="welcome-head">
                <h2 id="welcomeTitle" lang="ja">"どう遊びますか"</h2>
                <span class="faint">"How do you want to play?"</span>
            </div>
            <div class="welcome-cards">
                <button type="button" class="welcome-card" on:click=move |_| app.dispatch(Event::SetMode(Mode::Score))>
                    <span lang="ja" class="welcome-mark shu">"級"</span>
                    <span class="welcome-title">"Levels and score"</span>
                    <span class="faint">
                        "Helpers fade as you get better and come back after a rough patch. "
                        "Score, streak and a multiplier."
                    </span>
                    // No-break spaces before the separators: a line may end
                    // with one, never start with one (行頭禁則).
                    <span lang="ja" class="welcome-path">"入門\u{a0}→ 初級\u{a0}→ 中級\u{a0}→ 上級"</span>
                    <kbd class="key">"1"</kbd>
                </button>
                <button type="button" class="welcome-card" on:click=move |_| app.dispatch(Event::SetMode(Mode::Zen))>
                    <span lang="ja" class="welcome-mark">"禅"</span>
                    <span class="welcome-title">"Zen"</span>
                    <span class="faint">
                        "Just type. No score, no levels. Helpers stay exactly as you set them, "
                        "and the controls step back while you type."
                    </span>
                    <span lang="ja" class="welcome-path">"ふりがな\u{a0}· 翻訳\u{a0}· ヒント\u{a0}· 音声"</span>
                    <kbd class="key">"2"</kbd>
                </button>
            </div>
            <span class="faint welcome-foot">"Saved on this device. Change it any time in Options."</span>
        </div>
    }
}

fn main() {
    console_error_panic_hook_lite();
    if framed() {
        show_page();
        leptos::mount::mount_to_body(Framed);
    } else {
        // Both layouts; the stylesheets show the one for the screen.
        leptos::mount::mount_to_body(|| view! { <Game /> <mobile::Phone /> });
    }
}

/// Take the page out of `loading` (`index.html`), which hides it.
fn show_page() {
    if let Some(body) = document().body() {
        body.class_list()
            .remove_1("loading")
            .reported("showing the page");
    }
}

/// The page is inside another page's frame. GitHub Pages cannot send
/// `frame-ancestors` (a `<meta>` policy ignores it), so the game refuses to
/// run there: a page framing it could lay its own over the controls
/// (clickjacking). Without a top window to compare, it counts as framed.
fn framed() -> bool {
    let Some(window) = web_sys::window() else {
        return true;
    };
    let top = window.top().reported("finding the top window").flatten();
    top.as_ref() != Some(&window)
}

/// Shown instead of the game inside a frame ([`framed`]): a link that
/// opens it on its own.
#[component]
fn Framed() -> impl IntoView {
    view! {
        <div class="framed" lang="en">
            <p lang="ja">"WanType はほかのページの中では動きません。"</p>
            <span class="faint">"WanType does not run inside another page."</span>
            <a id="openOnItsOwn" class="button" href="./" target="_top">
                <span lang="ja">"開く"</span>
                <span class="faint">"Open"</span>
            </a>
        </div>
    }
}

/// Log panics to the browser console instead of failing silently.
fn console_error_panic_hook_lite() {
    std::panic::set_hook(Box::new(|info| leptos::logging::error!("{info}")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom_key_values_map_to_core_keys() {
        assert_eq!(to_core_key("Enter"), Key::Enter);
        assert_eq!(to_core_key("Tab"), Key::Tab);
        assert_eq!(to_core_key("Escape"), Key::Escape);
        assert_eq!(to_core_key("Backspace"), Key::Backspace);
        assert_eq!(to_core_key("Delete"), Key::Delete);
        assert_eq!(to_core_key(" ").normalize(), Key::Space);
        assert_eq!(to_core_key("k"), Key::Char('k'));
        assert_eq!(to_core_key("か"), Key::Char('か'));
        // Key names must never be typed into the sentence.
        assert_eq!(to_core_key("ArrowLeft"), Key::Other);
        assert_eq!(to_core_key("Shift"), Key::Other);
        assert_eq!(to_core_key("Unidentified"), Key::Other);
        assert_eq!(to_core_key("Dead"), Key::Other);
    }

    #[test]
    fn ime_keys_are_detected() {
        assert!(is_ime_key("Process", false, 229));
        assert!(is_ime_key("a", true, 65));
        assert!(is_ime_key("Unidentified", false, 229));
        assert!(!is_ime_key("a", false, 65));
    }

    #[test]
    fn slip_notes_name_the_hinted_spelling() {
        let note = |text: &str, typed: &str, key: char| {
            let mut m = wantype_core::Matcher::new(text);
            typed.chars().for_each(|c| assert!(m.input(c)));
            assert!(!m.input(key));
            m.slip(key).map(|slip| slip_note(&slip))
        };
        assert_eq!(
            note("これは", "kore", 'w').as_deref(),
            Some("は is typed ha, even where it is said wa")
        );
        assert_eq!(
            note("ゲーム", "ge", 'e').as_deref(),
            Some("ー (long vowel) is typed -")
        );
        assert_eq!(
            note("ねぇ", "ne", 'e').as_deref(),
            Some("Small ぇ is typed xe")
        );
    }

    #[test]
    fn the_voice_button_replays_once_on() {
        assert_eq!(voice_click(false, false), Event::SetAudio(true));
        assert_eq!(voice_click(true, false), Event::PlayAudio);
        assert_eq!(voice_click(true, true), Event::SetAudio(false));
    }

    #[test]
    fn the_play_row_follows_the_outcome() {
        let outcome = |tier_up, offer| Outcome {
            accuracy: 1.0,
            wpm: 40.0,
            counted: true,
            recorded: wantype_core::ladder::Recorded {
                tier_up,
                offer,
                ..wantype_core::ladder::Recorded::unscored(
                    1.0,
                    wantype_core::Helpers::default(),
                    Tier::Nyumon,
                )
            },
        };
        assert_eq!(panel(None), Panel::Typing);
        assert_eq!(panel(Some(&outcome(None, None))), Panel::Complete);
        assert_eq!(
            panel(Some(&outcome(Some(Tier::Shokyu), None))),
            Panel::LevelUp
        );
        assert_eq!(
            panel(Some(&outcome(None, Some(Helper::Hint)))),
            Panel::DropBack
        );
    }
}
