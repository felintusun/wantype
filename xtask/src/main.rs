//! Project chores: `cargo xtask <task>` (alias in `.cargo/config.toml`).
//!
//!   data   build the app's sentence data from Tatoeba's exports
//!   csp    add the Content-Security-Policy to a release build (Trunk hook)
//!   icons  render the PNG icons from the SVG

mod csp;
mod data;
mod icons;

use std::error::Error as _;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("unexpected argument `{0}`")]
    Argument(String),
    #[error("unknown task `{0}`")]
    UnknownTask(String),
    #[error("no task given")]
    NoTask,
    #[error("data")]
    Data(#[from] data::Error),
    #[error("csp")]
    Csp(#[from] csp::Error),
    #[error("icons")]
    Icons(#[from] icons::Error),
}

/// An I/O error, and the path it is about.
#[derive(Debug, thiserror::Error)]
#[error("{}", path.display())]
pub struct IoError {
    path: PathBuf,
    #[source]
    source: io::Error,
}

const USAGE: &str = "\
usage: cargo xtask <task> [options]

tasks:
  data [--update]
               build the sentence data (Tatoeba, CC BY 2.0 FR) into
               app/public/sentences, unless it is already there.
               Downloads Tatoeba's exports (about 200 MB) into
               target/tatoeba once; --update fetches newer exports
               and rebuilds.
  csp          add the Content-Security-Policy to the page of a release
               build; run by Trunk as a post-build hook (app/Trunk.toml).
  icons        render app/public/favicon.png (32 px) and
               apple-touch-icon.png (180 px) from favicon.svg.";

fn main() -> ExitCode {
    let mut update = false;
    let mut task = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--update" => update = true,
            "-h" | "--help" | "help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ if task.is_none() => task = Some(arg),
            _ => return fail(Error::Argument(arg)),
        }
    }
    let result = match task.as_deref() {
        Some("data") => data::data(update).map_err(Error::from),
        Some("csp") => csp::csp().map_err(Error::from),
        Some("icons") => icons::icons().map_err(Error::from),
        Some(other) => Err(Error::UnknownTask(other.into())),
        None => Err(Error::NoTask),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}

/// Report `error` with its causes, `a: b: c`.
fn fail(error: Error) -> ExitCode {
    let mut message = error.to_string();
    let mut cause = error.source();
    while let Some(e) = cause {
        message.push_str(&format!(": {e}"));
        cause = e.source();
    }
    eprintln!("error: {message}");
    if matches!(
        error,
        Error::Argument(_) | Error::UnknownTask(_) | Error::NoTask
    ) {
        eprintln!("\n{USAGE}");
    }
    ExitCode::FAILURE
}

/// The workspace root.
fn root() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).to_path_buf()
}

/// Name the path an I/O error is about.
fn at<T>(path: &Path, result: io::Result<T>) -> Result<T, IoError> {
    result.map_err(|source| IoError {
        path: path.to_path_buf(),
        source,
    })
}
