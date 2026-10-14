//! The phone layout (`mobile.css`), apart from the keyboard one (`Game`,
//! `style.css`) that desktops and tablets play. A phone's own keyboard
//! sends no keys, so the game brings a flick pad (`wantype_core::flick`).
//!
//! The sentence is one column of manuscript squares seen through a lens:
//! the square to type stays where it is and the column slides up past it,
//! a square at a time.

use leptos::ev::PointerEvent;
use leptos::prelude::*;
use wantype_core::flick::{Flick, Gesture, Pad, FLICKS, PAD};
use wantype_core::{Event, Session};
use wasm_bindgen::JsCast;

use crate::genko;
use crate::js::Reported;
use crate::{App, Options, OptionsButton, Shown, Welcome};

/// Phones get this layout: a screen narrower than a portrait tablet, or a
/// touch screen lower than one (a phone turned sideways).
pub fn phone() -> bool {
    window()
        .match_media("(max-width: 599px), (pointer: coarse) and (max-height: 599px)")
        .reported("asking the screen's width")
        .flatten()
        .is_some_and(|query| query.matches())
}

/// A flick's name, as the stylesheet knows it.
fn way(flick: Flick) -> &'static str {
    match flick {
        Flick::Tap => "tap",
        Flick::Left => "left",
        Flick::Up => "up",
        Flick::Right => "right",
        Flick::Down => "down",
    }
}

/// The game on a phone: the lens, the translation, the flick pad.
#[component]
pub fn Phone(
    /// The game, which outlives the layout showing it ([`crate::Layout`]).
    app: App,
) -> impl IntoView {
    let session = app.session;
    let Shown {
        sentence,
        segments,
        helpers,
        completed,
        ..
    } = app.view;
    let at = Memo::new(move |_| segments.with(|s| genko::active_square(s)));

    // The whole sentence in one column, built once per sentence.
    let column = move || {
        sentence.track();
        let (lines, width) = segments.with_untracked(|s| {
            let width = genko::squares(s);
            (genko::lines(s, width), width)
        });
        genko::view(lines, width, segments, Signal::stored(None))
    };

    // The gesture to make next, with the key hint on.
    let hint = Memo::new(move |_| {
        helpers
            .with(|h| h.hint)
            .then(|| session.with(Session::flick_hint))
            .flatten()
    });
    let on_flick = Callback::new(move |gesture: Gesture| {
        // The first run's choice comes first, as with the keyboard.
        if session.with_untracked(|s| s.settings().mode.is_some()) {
            app.dispatch(Event::Flick(gesture));
        }
    });

    view! {
        <div class="phone">
            <div class="phone-bar">
                <OptionsButton app />
                <button type="button" class="button" on:click=move |_| app.dispatch(Event::Restart)>
                    <span lang="ja">"やり直す"</span>
                </button>
                <button
                    type="button"
                    class="button"
                    class:ready=completed
                    on:click=move |_| app.dispatch(Event::Next)
                >
                    // Both labels in one slot: the button never changes width.
                    <span lang="ja" class="slot">
                        <span class:shown=move || !completed.get()>"飛ばす"</span>
                        <span class:shown=completed>"次へ"</span>
                    </span>
                </button>
            </div>
            <div class="lens" node_ref=app.refs.sentence style:--at=move || at.get().to_string()>
                {column}
                <span class="lens-held" lang="ja">
                    {move || session.with(Session::held).map(String::from)}
                </span>
            </div>
            <div class="phone-translation" dir="auto" lang=app.view.translation_lang()>
                {app.view.translation_text()}
            </div>
            <div class="pad" lang="ja" role="group" aria-label="Flick keyboard">
                {PAD.map(|pad| view! { <PadKey pad hint on_flick /> }).collect_view()}
            </div>
            <Options app />
            <Welcome app />
        </div>
    }
}

/// One key of the pad. A touch is a tap, or a flick the way it moves
/// ([`Flick::of_motion`]); while pressed, the key shows which.
#[component]
fn PadKey(
    pad: Pad,
    /// The gesture to make next, if hinted.
    hint: Memo<Option<Gesture>>,
    on_flick: Callback<Gesture>,
) -> impl IntoView {
    // Where the touch began, and where it is going.
    let start = StoredValue::new(None::<(f64, f64)>);
    let toward = RwSignal::new(None::<Flick>);
    let motion = move |e: &PointerEvent| {
        start.get_value().map(|(x, y)| {
            Flick::of_motion(f64::from(e.client_x()) - x, f64::from(e.client_y()) - y)
        })
    };
    let release = move || {
        start.set_value(None);
        toward.set(None);
    };
    let hinted = move || {
        hint.get()
            .filter(|hinted| hinted.pad == pad)
            .map(|hinted| way(hinted.flick))
    };
    let labels = FLICKS
        .into_iter()
        .zip(pad.chars())
        .filter_map(|(flick, ch)| {
            let ch = ch?;
            Some(view! { <span class=format!("pad-char {}", way(flick))>{ch.to_string()}</span> })
        })
        .collect_view();
    let turn = (pad == Pad::Turn).then(|| view! { <span class="pad-char tap">"゛゜小"</span> });
    view! {
        <button
            type="button"
            class="pad-key"
            data-hint=hinted
            data-toward=move || toward.get().map(way)
            on:pointerdown=move |e: PointerEvent| {
                // The key keeps the touch while it flicks off its edge.
                let target = e.current_target();
                if let Some(key) = target.as_ref().and_then(|t| t.dyn_ref::<web_sys::Element>()) {
                    key.set_pointer_capture(e.pointer_id()).reported("holding the touch");
                }
                start.set_value(Some((f64::from(e.client_x()), f64::from(e.client_y()))));
                toward.set(Some(Flick::Tap));
            }
            on:pointermove=move |e: PointerEvent| {
                if let Some(flick) = motion(&e) {
                    if toward.get_untracked() != Some(flick) {
                        toward.set(Some(flick));
                    }
                }
            }
            on:pointerup=move |e: PointerEvent| {
                if let Some(flick) = motion(&e) {
                    on_flick.run(pad.flick(flick));
                }
                release();
            }
            on:pointercancel=move |_| release()
        >
            {labels}
            {turn}
        </button>
    }
}
