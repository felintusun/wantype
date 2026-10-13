//! Failures from browser APIs, and reporting the ones no one in the page
//! can act on.

use std::error::Error;

use wasm_bindgen::{JsCast, JsValue};

/// An exception a browser API threw, by its message.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct JsFailure(String);

impl From<JsValue> for JsFailure {
    fn from(value: JsValue) -> Self {
        let message = match (value.dyn_ref::<js_sys::Error>(), value.as_string()) {
            (Some(error), _) => format!("{}: {}", error.name(), error.message()),
            (None, Some(text)) => text,
            (None, None) => format!("{value:?}"),
        };
        JsFailure(message)
    }
}

/// `error` with its causes: `a: b: c`.
pub fn chain<E: Error + ?Sized>(error: &E) -> String {
    let mut message = error.to_string();
    let mut cause = error.source();
    while let Some(e) = cause {
        message.push_str(": ");
        message.push_str(&e.to_string());
        cause = e.source();
    }
    message
}

/// Log `error` to the console, saying what failed.
pub fn report(what: &str, error: &impl Error) {
    leptos::logging::error!("{what}: {}", chain(error));
}

/// A browser call's result, its failure reported to the console. For
/// calls whose failure the player cannot do anything about (focus, a
/// class, a listener); `what` names the call.
pub trait Reported<T> {
    fn reported(self, what: &str) -> Option<T>;
}

impl<T> Reported<T> for Result<T, JsValue> {
    fn reported(self, what: &str) -> Option<T> {
        match self {
            Ok(value) => Some(value),
            Err(error) => {
                report(what, &JsFailure::from(error));
                None
            }
        }
    }
}
