//! Fetching the sentence data (`sentences/`, built by `cargo xtask data`)
//! and answering the session's sentence requests from it. What to pick is
//! the core's job (`wantype_core::tatoeba`); this module only does the I/O.
//! Big files are fetched only when a setting needs them.

use std::collections::{BTreeSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use futures::lock::Mutex;
use serde::de::DeserializeOwned;
use wantype_core::tatoeba::{
    candidates, locate, DataFile, Filter, Index, Indices, Language, Picker, RawSentence, Readings,
};
use wantype_core::{ParseError, Sentence, SentenceId, Settings};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::js::JsFailure;

const DATA: &str = "sentences";

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("no browser window")]
    NoWindow,
    #[error("{url}")]
    Fetch {
        url: String,
        #[source]
        source: JsFailure,
    },
    #[error("{url}: HTTP {status}")]
    Status { url: String, status: u16 },
    #[error("{url}: the response is not text")]
    NotText { url: String },
    #[error("{url}")]
    Json {
        url: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("No sentence matches these settings")]
    NoMatch,
    #[error("sentence {pick} is missing from chunk {chunk}")]
    Missing { pick: u32, chunk: u32 },
    #[error("sentence {id}")]
    Sentence {
        id: SentenceId,
        #[source]
        source: ParseError,
    },
}

/// `file` of data `version`, parsed. Files that change in place (the index)
/// are revalidated; the rest live under a directory that changes with
/// their content, so normal caching is safe.
async fn fetch_json<T: DeserializeOwned>(file: &DataFile, version: &str) -> Result<T, DataError> {
    let window = web_sys::window().ok_or(DataError::NoWindow)?;
    let url = format!("{DATA}/{}", file.path(version));
    let fetch_error = |e: wasm_bindgen::JsValue| DataError::Fetch {
        url: url.clone(),
        source: e.into(),
    };
    let init = web_sys::RequestInit::new();
    if file.changes_in_place() {
        init.set_cache(web_sys::RequestCache::NoCache);
    }
    let response: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init(&url, &init))
        .await
        .map_err(fetch_error)?
        .unchecked_into();
    if !response.ok() {
        let status = response.status();
        return Err(DataError::Status { url, status });
    }
    let text = JsFuture::from(response.text().map_err(fetch_error)?)
        .await
        .map_err(fetch_error)?
        .as_string()
        .ok_or_else(|| DataError::NotText { url: url.clone() })?;
    serde_json::from_str(&text).map_err(|source| DataError::Json { url, source })
}

/// A file, fetched once: callers at the same time wait for the first one's
/// fetch (the language menu and the first sentence both need the index),
/// later ones get the parsed value. A failed fetch leaves it empty, so the
/// next call tries again.
type Slot<T> = Rc<Mutex<Option<Rc<T>>>>;

async fn cached<T: DeserializeOwned>(
    slot: Slot<T>,
    file: DataFile,
    version: &str,
) -> Result<Rc<T>, DataError> {
    let mut held = slot.lock().await;
    if let Some(value) = &*held {
        return Ok(value.clone());
    }
    let value = Rc::new(fetch_json::<T>(&file, version).await?);
    *held = Some(value.clone());
    Ok(value)
}

async fn index(store: Store) -> Result<Rc<Index>, DataError> {
    let slot = with(store, |d| d.index.clone());
    cached(slot, DataFile::Index, "").await
}

pub async fn languages(store: Store) -> Result<Vec<Language>, DataError> {
    Ok(index(store).await?.languages.clone())
}

/// The settings that decide which sentences are candidates (not the
/// translation language: a sentence without one shows the English one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    audio: bool,
    unknown_readings: Option<u8>,
    known_readings: Arc<BTreeSet<String>>,
}

impl Selection {
    pub fn new(settings: &Settings) -> Self {
        Selection {
            audio: settings.audio,
            unknown_readings: settings.unknown_readings,
            // Known readings only matter in "new kanji" mode.
            known_readings: match settings.unknown_readings {
                Some(_) => Arc::clone(&settings.known_readings),
                None => Arc::default(),
            },
        }
    }
}

/// Sentences kept fetched ahead so "next" is instant, as upstream does.
const PRELOAD: usize = 5;

/// Fetched files and the current selection's sentences.
#[derive(Default)]
pub struct Data {
    index: Slot<Index>,
    readings: Slot<Readings>,
    pool: Option<Pool>,
    /// The selection a background preload is running for.
    preloading: Option<Selection>,
}

/// One selection's random picker and the sentences fetched ahead for it.
struct Pool {
    selection: Selection,
    picker: Picker,
    ready: VecDeque<Sentence>,
}

/// Handle to the shared cache (lives for the whole app, single-threaded).
pub type Store = leptos::prelude::StoredValue<Data, leptos::prelude::LocalStorage>;

/// Work on the store, which lives as long as the app.
fn with<R>(store: Store, f: impl FnOnce(&mut Data) -> R) -> R {
    use leptos::prelude::*;
    f(&mut store.write_value())
}

/// `selection`'s pool, if it is the current one.
fn pool<'a>(data: &'a mut Data, selection: &Selection) -> Option<&'a mut Pool> {
    data.pool.as_mut().filter(|p| p.selection == *selection)
}

/// A random sentence matching `selection`, like upstream's sentence
/// provider: one fetched ahead if there is one, then the buffer is refilled
/// in the background.
pub async fn next_sentence(store: Store, selection: Selection) -> Result<Sentence, DataError> {
    let ready = with(store, |d| pool(d, &selection)?.ready.pop_front());
    let sentence = match ready {
        Some(sentence) => sentence,
        None => {
            let mut batch = fetch_batch(store, &selection).await?.into_iter();
            let first = batch.next().ok_or(DataError::NoMatch)?;
            queue(store, &selection, batch);
            first
        }
    };
    preload(store, selection);
    Ok(sentence)
}

/// Keep `sentences` ready, if `selection` is still the current one.
fn queue(store: Store, selection: &Selection, sentences: impl IntoIterator<Item = Sentence>) {
    with(store, |d| {
        if let Some(pool) = pool(d, selection) {
            pool.ready.extend(sentences);
        }
    });
}

/// Fill the buffer for `selection` in the background. A running preload
/// for another selection stops at its next step; a failed one is reported
/// and stops (the next sentence request fetches, and says what failed).
pub fn preload(store: Store, selection: Selection) {
    if with(store, |d| d.preloading.as_ref() == Some(&selection)) {
        return;
    }
    with(store, |d| d.preloading = Some(selection.clone()));
    leptos::task::spawn_local(async move {
        let current = || with(store, |d| d.preloading.as_ref() == Some(&selection));
        while current() {
            let ready = with(store, |d| pool(d, &selection).map_or(0, |p| p.ready.len()));
            if ready >= PRELOAD {
                break;
            }
            match fetch_batch(store, &selection).await {
                Ok(batch) => queue(store, &selection, batch),
                Err(error) => {
                    crate::js::report("preloading sentences", &error);
                    break;
                }
            }
        }
        if current() {
            with(store, |d| d.preloading = None);
        }
    });
}

/// Pick a sentence and fetch its chunk: the pick first, then the chunk's
/// other candidates ([`Picker::next_batch`]). The candidates are rebuilt
/// only when the selection changes.
async fn fetch_batch(store: Store, selection: &Selection) -> Result<Vec<Sentence>, DataError> {
    let fresh = with(store, |d| pool(d, selection).is_some());
    let index = index(store).await?;
    if !fresh {
        let version = index.version.clone();
        let readings = match selection.unknown_readings {
            Some(_) => {
                let slot = with(store, |d| d.readings.clone());
                Some(cached(slot, DataFile::Readings, &version).await?)
            }
            None => None,
        };
        let ids = candidates(
            Indices {
                index: &index,
                readings: readings.as_deref(),
            },
            Filter {
                audio: selection.audio,
                unknown_readings: selection.unknown_readings,
                known_readings: &selection.known_readings,
            },
        );
        with(store, |d| {
            d.pool = Some(Pool {
                selection: selection.clone(),
                picker: Picker::new(ids),
                ready: VecDeque::new(),
            })
        });
    }
    let (n, picks) = with(store, |d| {
        pool(d, selection)?.picker.next_batch(js_sys::Math::random)
    })
    .ok_or(DataError::NoMatch)?;
    let chunk: Vec<RawSentence> = fetch_json(&DataFile::Chunk(n), &index.version).await?;
    let mut chunk: Vec<Option<RawSentence>> = chunk.into_iter().map(Some).collect();
    picks
        .into_iter()
        .map(|pick| {
            let raw = chunk
                .get_mut(locate(pick).1)
                .and_then(Option::take)
                .ok_or(DataError::Missing { pick, chunk: n })?;
            let id = raw.id;
            raw.into_sentence()
                .map_err(|source| DataError::Sentence { id, source })
        })
        .collect()
}
