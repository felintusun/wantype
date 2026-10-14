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
use crate::js::{self, Reported};
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

/// The lens as a page of the whole sentence is chosen for
/// ([`genko::page`]): its size, and its squares' and their strip's
/// (mobile.css's `--square`, `--strip`), in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Lens {
    height: f64,
    width: f64,
    cell: f64,
    strip: f64,
}

/// Why the lens could not be measured.
#[derive(Debug, thiserror::Error)]
enum LensError {
    #[error("{property} is {value:?}, not in pixels")]
    NotPixels {
        property: &'static str,
        value: String,
    },
    #[error("{property} is {value:?}")]
    Number {
        property: &'static str,
        value: String,
        #[source]
        source: std::num::ParseFloatError,
    },
}

impl Lens {
    /// Measure `lens`; `None`, reported, if the browser cannot say.
    fn of(lens: &web_sys::Element) -> Option<Lens> {
        let style = window()
            .get_computed_style(lens)
            .reported("reading the lens's style")
            .flatten()?;
        let pixels = |property: &'static str| -> Option<f64> {
            let value = style
                .get_property_value(property)
                .reported("reading the lens's squares")?;
            let parsed = match value.trim().strip_suffix("px") {
                Some(number) => number.parse().map_err(|source| LensError::Number {
                    property,
                    value: value.clone(),
                    source,
                }),
                None => Err(LensError::NotPixels {
                    property,
                    value: value.clone(),
                }),
            };
            match parsed {
                Ok(pixels) => Some(pixels),
                Err(error) => {
                    js::report("measuring the lens", &error);
                    None
                }
            }
        };
        Some(Lens {
            height: f64::from(lens.client_height()),
            width: f64::from(lens.client_width()),
            cell: pixels("--square")?,
            strip: pixels("--strip")?,
        })
    }
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

    // The whole sentence through the lens, in one column; or tapped, on a
    // page of columns chosen for the lens's shape, its squares sized by the
    // CSS to the lens as it is. Built once per sentence, and page.
    let paged = RwSignal::new(None::<Lens>);
    let page = Memo::new(move |_| {
        sentence.track();
        let lens = paged.get()?;
        Some(
            segments
                .with_untracked(|s| genko::page(s, lens.height, lens.width, lens.cell, lens.strip)),
        )
    });
    let measure = move || {
        app.refs
            .sentence
            .get_untracked()
            .and_then(|lens| Lens::of(&lens))
    };
    // A turned phone keeps the page, with columns for its new shape.
    let listener = window_event_listener(leptos::ev::resize, move |_| {
        if paged.with_untracked(Option::is_some) {
            paged.set(measure());
        }
    });
    on_cleanup(move || listener.remove());
    // Typing, another sentence or a retry goes back to the lens.
    let close_page = move || {
        if paged.with_untracked(Option::is_some) {
            paged.set(None);
        }
    };
    let column = move || {
        sentence.track();
        let page = page.get();
        let (lines, width) = segments.with_untracked(|s| {
            let width = page.unwrap_or_else(|| genko::squares(s));
            (genko::lines(s, width), width)
        });
        genko::view(lines, width, segments, Signal::stored(None))
    };
    let paging = move || paged.with(Option::is_some);
    let toggle_page = move |_| {
        let lens = if paged.with_untracked(Option::is_some) {
            None
        } else {
            measure()
        };
        paged.set(lens);
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
            close_page();
            app.dispatch(Event::Flick(gesture));
        }
    });

    view! {
        <div class="phone">
            <div class="phone-bar">
                <OptionsButton app />
                <button
                    type="button"
                    class="button"
                    on:click=move |_| {
                        close_page();
                        app.dispatch(Event::Restart)
                    }
                >
                    <span lang="ja">"やり直す"</span>
                </button>
                <button
                    type="button"
                    class="button"
                    class:ready=completed
                    on:click=move |_| {
                        close_page();
                        app.dispatch(Event::Next)
                    }
                >
                    // Both labels in one slot: the button never changes width.
                    <span lang="ja" class="slot">
                        <span class:shown=move || !completed.get()>"飛ばす"</span>
                        <span class:shown=completed>"次へ"</span>
                    </span>
                </button>
            </div>
            <div
                class="lens"
                class:page=paging
                node_ref=app.refs.sentence
                style:--at=move || at.get().to_string()
            >
                {column}
                <span class="lens-held" lang="ja">
                    {move || session.with(Session::held).map(String::from)}
                </span>
                // Over the lens: a tap shows the whole sentence, or the lens
                // again.
                <button
                    type="button"
                    class="lens-toggle"
                    aria-label="Whole sentence"
                    aria-pressed=move || paging().to_string()
                    on:click=toggle_page
                ></button>
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
