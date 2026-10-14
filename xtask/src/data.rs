//! `cargo xtask data`: the app's sentence data, built from Tatoeba's weekly
//! exports (sentences CC BY 2.0 FR) as upstream's `sentences/tatoeba`
//! scripts build theirs, but checked with this project's own parser and
//! matcher: sentences whose furigana no longer match the text, or that
//! cannot be typed on a romaji keyboard, are left out. The files are
//! described in `wantype_core::tatoeba`.
//!
//! Downloads (Tatoeba's per-language files, about 200 MB) are cached in
//! `target/tatoeba/`; `--update` asks Tatoeba for newer exports.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use bzip2::read::MultiBzDecoder;
use wantype_core::tatoeba::{
    translated_languages, with_audio, DataFile, Index, Language, RawSentence, Readings, CHUNK_SIZE,
    FORMAT,
};
use wantype_core::{Audio, AudioId, Lang, Matcher, Sentence, SentenceId};

use crate::{at, root, IoError};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error("{url}")]
    Http {
        url: String,
        #[source]
        source: ureq::Error,
    },
    #[error("{url}: HTTP {status}")]
    Status { url: String, status: u16 },
    #[error("{url}: its Last-Modified header is not text")]
    LastModified {
        url: String,
        #[source]
        source: ureq::http::header::ToStrError,
    },
    #[error("upstream's language names")]
    LanguageNames(#[source] serde_json::Error),
    #[error("{}:{line}", file.display())]
    Row {
        file: PathBuf,
        line: usize,
        #[source]
        source: RowError,
    },
    #[error("a download worker panicked (its message is above)")]
    WorkerPanicked,
    #[error("{0} as JSON")]
    Json(&'static str, #[source] serde_json::Error),
    #[error("more sentences than the index can count")]
    TooMany(#[source] std::num::TryFromIntError),
}

/// A line of a Tatoeba export that is not what its file holds.
#[derive(Debug, thiserror::Error)]
pub enum RowError {
    #[error("only {0} fields")]
    Short(usize),
    #[error("id {field:?}")]
    Id {
        field: String,
        #[source]
        source: std::num::ParseIntError,
    },
}

type Result<T = (), E = Error> = std::result::Result<T, E>;

const EXPORTS: &str = "https://downloads.tatoeba.org/exports/per_language";
const USER_AGENT: &str = concat!("wantype-data/", env!("CARGO_PKG_VERSION"));
/// Parallel downloads (one translation language each).
const WORKERS: usize = 6;
/// Bounds on what Tatoeba sends, well above its largest export (English,
/// tens of MB compressed): a broken or hostile server fails the build
/// instead of filling the disk or memory.
const MAX_DOWNLOAD: u64 = 256 << 20;
const MAX_DECOMPRESSED: u64 = 2 << 30;

/// Upstream's `language-names.json` (MIT): the translation languages
/// offered, by Tatoeba code.
const LANGUAGE_NAMES: &str = include_str!("../data/language-names.json");

/// Upstream's limits: longer sentences do not fit the card.
const MAX_LENGTH: usize = 30;
/// Longer translations are left out (the longest is 176 chars): they would
/// not fit under the sentence either.
const MAX_TRANSLATION: usize = 200;
/// Upstream leaves out sentences with these: ヶ and ヵ are read か/が, which
/// their spelling does not say, and ゝ repeats the kana before it.
const UNSUPPORTED: [char; 3] = ['ヶ', 'ヵ', 'ゝ'];

/// Where the app serves the data from (`app/index.html` copies it).
fn out_dir() -> PathBuf {
    root().join("app/public/sentences")
}

pub fn data(update: bool) -> Result {
    let out = out_dir();
    if !update {
        if let Some(index) = current(&out)? {
            println!(
                "sentence data is up to date ({} sentences, exported {}; --update checks for newer)",
                index.sentences, index.exported
            );
            return Ok(());
        }
    }
    let downloads = Downloads::new(update);
    let exports = Exports::download(&downloads)?;
    let (sentences, skipped) = build(&exports);
    println!("kept {} sentences", sentences.len());
    for (skip, (count, example)) in &skipped {
        println!("left out {count:>5} {skip:?}, e.g. {example}");
    }
    write(&out, &sentences, &exports)
}

/// The data in `out`, if complete and in the current format. An index
/// this version cannot read (an older format) is rebuilt, and said so.
fn current(out: &Path) -> Result<Option<Index>> {
    let path = out.join(DataFile::Index.path(""));
    let Some(text) = read_if_exists(&path)? else {
        return Ok(None);
    };
    let index: Index = match serde_json::from_str(&text) {
        Ok(index) => index,
        Err(e) => {
            println!("{}: {e}; rebuilding", path.display());
            return Ok(None);
        }
    };
    Ok((index.format == FORMAT && out.join(&index.version).is_dir()).then_some(index))
}

/// A file's text, `None` if there is no such file.
fn read_if_exists(path: &Path) -> Result<Option<String>, IoError> {
    match fs::read_to_string(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        text => at(path, text).map(Some),
    }
}

/// Tatoeba's files, cached in `target/tatoeba/` under their path.
struct Downloads {
    agent: ureq::Agent,
    update: bool,
    cache: PathBuf,
}

impl Downloads {
    fn new(update: bool) -> Self {
        let agent = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            // Redirects included.
            .https_only(true)
            .build()
            .into();
        Downloads {
            agent,
            update,
            cache: root().join("target/tatoeba"),
        }
    }

    /// `path` under [`EXPORTS`] as a local file: the cached copy, unless
    /// updating and Tatoeba has a newer one.
    fn get(&self, path: &str) -> Result<PathBuf> {
        // Directory listings are cached as their `index.html`.
        let file = match path.strip_suffix('/') {
            Some(dir) => self.cache.join(dir).join("index.html"),
            None => self.cache.join(path),
        };
        let stamp = file.with_added_extension("last-modified");
        if file.exists() && !self.update {
            return Ok(file);
        }
        let url = format!("{EXPORTS}/{path}");
        let mut request = self.agent.get(&url);
        if file.exists() {
            if let Some(since) = read_if_exists(&stamp)? {
                request = request.header("If-Modified-Since", since.trim());
            }
        }
        let mut response = request.call().map_err(|source| Error::Http {
            url: url.clone(),
            source,
        })?;
        match response.status().as_u16() {
            304 => return Ok(file),
            200 => {}
            status => return Err(Error::Status { url, status }),
        }
        println!("downloading {path}");
        // Kept to ask for newer copies with; a server may not send one.
        let modified = match response.headers().get("last-modified") {
            Some(value) => value
                .to_str()
                .map_err(|source| Error::LastModified {
                    url: url.clone(),
                    source,
                })?
                .to_string(),
            None => String::new(),
        };
        if let Some(parent) = file.parent() {
            at(parent, fs::create_dir_all(parent))?;
        }
        // Into a side file first, so an interrupted download is never
        // taken for a complete one.
        let part = file.with_added_extension("part");
        let mut out = at(&part, File::create(&part))?;
        at(
            &part,
            io::copy(
                &mut response
                    .body_mut()
                    .with_config()
                    .limit(MAX_DOWNLOAD)
                    .reader(),
                &mut out,
            ),
        )?;
        at(&file, fs::rename(&part, &file))?;
        at(&stamp, fs::write(&stamp, modified))?;
        Ok(file)
    }

    /// When Tatoeba exported `path` (its `Last-Modified`), once fetched.
    fn modified(&self, path: &str) -> Result<String, IoError> {
        let stamp = self.cache.join(path).with_added_extension("last-modified");
        at(&stamp, fs::read_to_string(&stamp))
    }
}

/// Each line of a bzip2-compressed Tatoeba TSV file, split into fields.
/// (Tatoeba's fields hold no tabs or newlines, and quotes are literal.) A
/// line `row` cannot read stops the build: the export's format changed.
fn tsv(file: &Path, mut row: impl FnMut(&[&str]) -> Result<(), RowError>) -> Result {
    let reader = BufReader::new(Capped {
        inner: MultiBzDecoder::new(BufReader::new(at(file, File::open(file))?)),
        left: MAX_DECOMPRESSED,
    });
    for (n, line) in reader.lines().enumerate() {
        let line = at(file, line)?;
        row(&line.split('\t').collect::<Vec<_>>()).map_err(|source| Error::Row {
            file: file.to_path_buf(),
            line: n + 1,
            source,
        })?;
    }
    Ok(())
}

/// More than [`MAX_DECOMPRESSED`] bytes out of a compressed file.
#[derive(Debug, thiserror::Error)]
#[error("more than {MAX_DECOMPRESSED} bytes once decompressed")]
struct TooLarge;

/// A reader that fails once it has given `left` bytes and has more:
/// unlike `Read::take`, a cut file is never taken for a whole one.
struct Capped<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        let read = u64::try_from(n).map_err(io::Error::other)?;
        self.left = self
            .left
            .checked_sub(read)
            .ok_or_else(|| io::Error::other(TooLarge))?;
        Ok(n)
    }
}

/// A Tatoeba id field: a sentence's or a recording's.
fn parse_id<Id>(field: &str, id: fn(u32) -> Id) -> Result<Id, RowError> {
    field.parse().map(id).map_err(|source| RowError::Id {
        field: field.into(),
        source,
    })
}

/// What the exports say about the Japanese sentences.
#[derive(Debug, Default)]
struct Exports {
    /// Tatoeba id → text.
    texts: HashMap<SentenceId, String>,
    /// Tatoeba id → reviewed furigana transcription.
    transcriptions: HashMap<SentenceId, String>,
    audio: HashMap<SentenceId, Audio>,
    /// Tatoeba id → language → translation.
    translations: HashMap<SentenceId, BTreeMap<Lang, String>>,
    /// Translation languages offered: code → English name.
    names: BTreeMap<Lang, String>,
    /// When the transcriptions were exported.
    exported: String,
}

impl Exports {
    fn download(downloads: &Downloads) -> Result<Self> {
        let names: BTreeMap<Lang, String> =
            serde_json::from_str(LANGUAGE_NAMES).map_err(Error::LanguageNames)?;
        let mut exports = Exports {
            names,
            ..Exports::default()
        };

        tsv(&downloads.get("jpn/jpn_sentences.tsv.bz2")?, |f| {
            let [id, _lang, text, ..] = f else {
                return Err(RowError::Short(f.len()));
            };
            exports
                .texts
                .insert(parse_id(id, SentenceId)?, text.to_string());
            Ok(())
        })?;
        let transcriptions = "jpn/jpn_transcriptions.tsv.bz2";
        tsv(&downloads.get(transcriptions)?, |f| {
            let [id, _lang, _script, user, transcription, ..] = f else {
                return Err(RowError::Short(f.len()));
            };
            // "A transcription without a username has not been marked as
            // reviewed" (tatoeba.org/en/downloads).
            if !(user.is_empty() || *user == "\\N") {
                let id = parse_id(id, SentenceId)?;
                exports.transcriptions.insert(id, transcription.to_string());
            }
            Ok(())
        })?;
        exports.exported = downloads.modified(transcriptions)?;
        tsv(
            &downloads.get("jpn/jpn_sentences_with_audio.tsv.bz2")?,
            |f| {
                let [id, audio, author, ..] = f else {
                    return Err(RowError::Short(f.len()));
                };
                let audio = Audio {
                    id: parse_id(audio, AudioId)?,
                    author: author.to_string(),
                };
                exports.audio.insert(parse_id(id, SentenceId)?, audio);
                Ok(())
            },
        )?;

        let listing = at(
            Path::new("jpn/"),
            fs::read_to_string(downloads.get("jpn/")?),
        )?;
        let languages: Vec<&str> = linked_languages(&listing)
            .into_iter()
            .filter(|code| exports.names.contains_key(*code))
            .collect();
        println!(
            "{} translation languages; reading their files",
            languages.len()
        );

        // Workers take the next language by index; each returns what it
        // found.
        let next = AtomicUsize::new(0);
        let wanted: HashSet<SentenceId> = exports.transcriptions.keys().copied().collect();
        let mut found = Vec::new();
        thread::scope(|scope| {
            let workers: Vec<_> = (0..WORKERS)
                .map(|_| {
                    scope.spawn(|| {
                        let mut found = Vec::new();
                        while let Some(&lang) = languages.get(next.fetch_add(1, Ordering::Relaxed))
                        {
                            found.push((lang, translations(downloads, lang, &wanted)));
                        }
                        found
                    })
                })
                .collect();
            for worker in workers {
                found.extend(worker.join().map_err(|_| Error::WorkerPanicked)?);
            }
            Ok::<_, Error>(())
        })?;
        found.sort_by(|a, b| a.0.cmp(b.0));
        for (lang, result) in found {
            for (id, text) in result? {
                // Several translations into one language: the last wins, as
                // upstream.
                let entry = exports.translations.entry(id).or_default();
                entry.insert(Lang::new(lang), text);
            }
        }
        Ok(exports)
    }
}

/// The languages Tatoeba has `jpn-<code>_links` files for, from the
/// directory listing.
fn linked_languages(listing: &str) -> Vec<&str> {
    let mut codes: Vec<&str> = listing
        .split("href=\"jpn-")
        .skip(1)
        .filter_map(|rest| rest.split_once("_links.tsv.bz2\"").map(|(code, _)| code))
        .filter(|code| {
            !code.is_empty() && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .collect();
    codes.sort_unstable();
    codes.dedup();
    codes
}

/// (Japanese sentence id, translation) for `lang`, in Tatoeba id order.
fn translations(
    downloads: &Downloads,
    lang: &str,
    wanted: &HashSet<SentenceId>,
) -> Result<Vec<(SentenceId, String)>> {
    // Translation id → the Japanese sentences it translates.
    let mut links: HashMap<SentenceId, Vec<SentenceId>> = HashMap::new();
    tsv(
        &downloads.get(&format!("jpn/jpn-{lang}_links.tsv.bz2"))?,
        |f| {
            let [jpn, other, ..] = f else {
                return Err(RowError::Short(f.len()));
            };
            let jpn = parse_id(jpn, SentenceId)?;
            if wanted.contains(&jpn) {
                links
                    .entry(parse_id(other, SentenceId)?)
                    .or_default()
                    .push(jpn);
            }
            Ok(())
        },
    )?;
    let mut found = Vec::new();
    tsv(
        &downloads.get(&format!("{lang}/{lang}_sentences.tsv.bz2"))?,
        |f| {
            let [id, _lang, text, ..] = f else {
                return Err(RowError::Short(f.len()));
            };
            for &jpn in links.get(&parse_id(id, SentenceId)?).into_iter().flatten() {
                found.push((jpn, text.to_string()));
            }
            Ok(())
        },
    )?;
    Ok(found)
}

/// Why a sentence was left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Skip {
    /// Transcribed, but no longer among the sentences.
    Deleted,
    TooLong,
    Spaces,
    UnsupportedKana,
    Untranslated,
    /// Its translations are all longer than [`MAX_TRANSLATION`].
    LongTranslations,
    /// The transcription does not parse.
    Unparsable,
    /// The transcription's text differs from the sentence's (edited since).
    Stale,
    /// No way to type it to the end on a romaji keyboard.
    Untypeable,
}

/// Left-out sentences by reason: how many, and the one with the lowest
/// Tatoeba id as an example.
type Skipped = BTreeMap<Skip, (usize, String)>;

/// The sentences to ship with their readings, in shuffled order.
fn build(exports: &Exports) -> (Vec<(RawSentence, Vec<String>)>, Skipped) {
    let mut kept: Vec<(RawSentence, Vec<String>)> = Vec::new();
    let mut skipped: BTreeMap<Skip, (usize, SentenceId, String)> = BTreeMap::new();
    for (&id, transcription) in &exports.transcriptions {
        match check(exports, id, transcription) {
            Ok(entry) => kept.push(entry),
            Err((skip, why)) => {
                let entry = skipped.entry(skip).or_insert((0, id, why.clone()));
                entry.0 += 1;
                if id < entry.1 {
                    (entry.1, entry.2) = (id, why);
                }
            }
        }
    }
    // Shuffled (by a hash of the Tatoeba id), so a chunk holds unrelated
    // sentences rather than one contributor's consecutive ones.
    kept.sort_by_key(|(raw, _)| {
        let mut hasher = DefaultHasher::new();
        raw.id.hash(&mut hasher);
        (hasher.finish(), raw.id)
    });
    let skipped = skipped
        .into_iter()
        .map(|(skip, (count, id, why))| (skip, (count, format!("{id}: {why}"))))
        .collect();
    (kept, skipped)
}

fn check(
    exports: &Exports,
    id: SentenceId,
    transcription: &str,
) -> std::result::Result<(RawSentence, Vec<String>), (Skip, String)> {
    let text = exports
        .texts
        .get(&id)
        .ok_or((Skip::Deleted, String::new()))?;
    // Upstream's `text.length`: UTF-16 units.
    if text.encode_utf16().count() > MAX_LENGTH {
        return Err((Skip::TooLong, text.clone()));
    }
    if text.chars().any(char::is_whitespace) {
        return Err((Skip::Spaces, text.clone()));
    }
    if text.contains(UNSUPPORTED) {
        return Err((Skip::UnsupportedKana, text.clone()));
    }
    let (translations, too_long): (BTreeMap<Lang, String>, BTreeMap<Lang, String>) = exports
        .translations
        .get(&id)
        .into_iter()
        .flatten()
        .filter(|(lang, _)| exports.names.contains_key(*lang))
        .map(|(lang, t)| (lang.clone(), t.clone()))
        .partition(|(_, t)| t.chars().count() <= MAX_TRANSLATION);
    if translations.is_empty() {
        let skip = match too_long.is_empty() {
            true => Skip::Untranslated,
            false => Skip::LongTranslations,
        };
        return Err((skip, text.clone()));
    }
    let sentence = Sentence::parse(None, transcription, BTreeMap::new(), None)
        .map_err(|e| (Skip::Unparsable, format!("{transcription}: {e}")))?;
    if sentence.text() != *text {
        return Err((Skip::Stale, format!("{transcription} is not {text}")));
    }
    Matcher::romaji(&sentence.kana()).map_err(|e| (Skip::Untypeable, format!("{text}: {e}")))?;
    let raw = RawSentence {
        id,
        transcription: transcription.to_string(),
        translations,
        audio: exports.audio.get(&id).cloned(),
    };
    Ok((raw, sentence.readings()))
}

/// Replace `out` with the data: the files under a directory named by a
/// hash of their content, then the index last, naming it.
fn write(out: &Path, sentences: &[(RawSentence, Vec<String>)], exports: &Exports) -> Result {
    let mut files: Vec<(DataFile, Vec<u8>)> = Vec::new();

    for (n, chunk) in (0..).zip(sentences.chunks(CHUNK_SIZE as usize)) {
        let raws: Vec<&RawSentence> = chunk.iter().map(|(raw, _)| raw).collect();
        files.push((DataFile::Chunk(n), json("a chunk", &raws)?));
    }
    let audio = with_audio(sentences.iter().map(|(raw, _)| raw));
    let readings = Readings::new(sentences.iter().map(|(_, r)| r.as_slice()));
    files.push((DataFile::Readings, json("the readings", &readings)?));

    let mut hasher = DefaultHasher::new();
    files.hash(&mut hasher);
    let version = format!("{:016x}", hasher.finish());

    let mut languages: Vec<Language> = translated_languages(sentences.iter().map(|(raw, _)| raw))
        .into_iter()
        .map(|lang| Language {
            lang: lang.clone(),
            name: exports.names[lang].clone(),
        })
        .collect();
    languages.sort_by(|a, b| a.name.cmp(&b.name));
    let index = Index {
        format: FORMAT,
        version: version.clone(),
        exported: exports.exported.clone(),
        sentences: u32::try_from(sentences.len()).map_err(Error::TooMany)?,
        languages,
        audio,
    };

    if out.exists() {
        at(out, fs::remove_dir_all(out))?;
    }
    let mut bytes = 0;
    for (file, content) in &files {
        let file = out.join(file.path(&version));
        if let Some(parent) = file.parent() {
            at(parent, fs::create_dir_all(parent))?;
        }
        at(&file, fs::write(&file, content))?;
        bytes += content.len();
    }
    let index_file = out.join(DataFile::Index.path(&version));
    at(
        &index_file,
        fs::write(&index_file, json("the index", &index)?),
    )?;
    println!(
        "wrote {} files ({:.1} MB) to {}",
        files.len() + 1,
        bytes as f64 / 1e6,
        out.display()
    );
    Ok(())
}

/// `value` as JSON; `what` names it in an error.
fn json(what: &'static str, value: &impl serde::Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|e| Error::Json(what, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exports() -> Exports {
        let mut e = Exports {
            names: [(Lang::english(), "English".to_string())].into(),
            ..Exports::default()
        };
        let mut add = |id: SentenceId, text: &str, transcription: &str, translated: bool| {
            e.texts.insert(id, text.into());
            e.transcriptions.insert(id, transcription.into());
            if translated {
                let t = [(Lang::english(), "…".to_string())].into();
                e.translations.insert(id, t);
            }
        };
        add(SentenceId(1), "日本です。", "[日本|に|ほん]です。", true);
        add(SentenceId(2), "猫です。", "[猫|ねこ]です。", false);
        add(SentenceId(3), "犬です。", "[猫|ねこ]です。", true);
        add(SentenceId(4), "一ヶ月。", "[一|いっ]ヶ[月|げつ]。", true);
        add(SentenceId(5), "はい、そうです。", "はい、そうです。", true);
        add(SentenceId(6), "ふ ふ", "ふ ふ", true);
        add(SentenceId(7), "♪", "♪", true);
        add(SentenceId(9), "長いです。", "[長|なが]いです。", false);
        e.transcriptions
            .insert(SentenceId(8), "[消|け]した。".into());
        let long = "x".repeat(MAX_TRANSLATION + 1);
        e.translations
            .insert(SentenceId(9), [(Lang::english(), long)].into());
        e
    }

    #[test]
    fn keeps_checked_sentences_and_says_why_not() {
        let e = exports();
        let (kept, skipped) = build(&e);
        let ids: Vec<SentenceId> = kept.iter().map(|(raw, _)| raw.id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&SentenceId(1)) && ids.contains(&SentenceId(5)));
        let first = kept.iter().find(|(raw, _)| raw.id == SentenceId(1));
        assert_eq!(
            first.map(|(raw, readings)| (raw.translations["eng"].as_str(), readings.as_slice())),
            Some(("…", ["日本|にほん".to_string()].as_slice()))
        );
        let skipped: BTreeMap<Skip, usize> = skipped
            .iter()
            .map(|(skip, (count, _))| (*skip, *count))
            .collect();
        assert_eq!(
            skipped,
            [
                (Skip::Deleted, 1),
                (Skip::Spaces, 1),
                (Skip::UnsupportedKana, 1),
                (Skip::Untranslated, 1),
                (Skip::LongTranslations, 1),
                (Skip::Stale, 1),
                (Skip::Untypeable, 1),
            ]
            .into()
        );
    }

    #[test]
    fn finds_linked_languages_in_the_listing() {
        let listing = r#"<a href="../">../</a>
            <a href="jpn-eng_links.tsv.bz2">jpn-eng_links.tsv.bz2</a>
            <a href="jpn_sentences.tsv.bz2">jpn_sentences.tsv.bz2</a>
            <a href="jpn-deu_links.tsv.bz2">jpn-deu_links.tsv.bz2</a>"#;
        assert_eq!(linked_languages(listing), ["deu", "eng"]);
    }

    #[test]
    fn a_capped_reader_fails_past_its_limit() -> Result<(), io::Error> {
        let mut text = String::new();
        let mut at_limit = Capped {
            inner: "1234".as_bytes(),
            left: 4,
        };
        at_limit.read_to_string(&mut text)?;
        assert_eq!(text, "1234");
        let mut over = Capped {
            inner: "12345".as_bytes(),
            left: 4,
        };
        let read = over.read_to_string(&mut String::new());
        assert!(
            matches!(&read, Err(e) if e.get_ref().is_some_and(|e| e.is::<TooLarge>())),
            "a cut file is an error: {read:?}"
        );
        Ok(())
    }
}
