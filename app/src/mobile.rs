//! The phone layout (`mobile.css`), apart from the keyboard one (`Game`,
//! `style.css`) that desktops and tablets play. A phone's own keyboard
//! sends no keys, so for now it says what WanType is; the game with a
//! keyboard of its own comes here.

use leptos::prelude::*;

/// Shown on a phone instead of the game.
#[component]
pub fn Phone() -> impl IntoView {
    view! {
        <div class="phone" lang="en">
            <em>"WanType"</em> " is a simple web-based " <em>"Japanese typing game"</em>
            " featuring thousands of sentences from Tatoeba, including furigana, "
            "translation in many languages and recordings from native speakers."
            <br /><br />
            "If you're on mobile, switch to a computer to get started!"
        </div>
    }
}
