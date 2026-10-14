//! `cargo xtask csp`: adds the Content-Security-Policy to the built page.
//! Run by Trunk after a release build (`app/Trunk.toml`): the policy names
//! the inline script Trunk injects (its wasm loader) by hash, and that
//! changes with every build. Debug builds (`trunk serve`) are left alone,
//! since Trunk's live reload injects scripts of its own.

use std::fs;
use std::path::PathBuf;

use base64::Engine;
use sha2::{Digest, Sha256};
use wantype_core::AudioId;

use crate::{at, IoError};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("run me as a Trunk hook (TRUNK_STAGING_DIR is not set)")]
    NotAHook,
    #[error(transparent)]
    Io(#[from] IoError),
    #[error("the page has a policy already")]
    HasPolicy,
    #[error("unclosed <script>")]
    UnclosedScript,
    #[error("the page has no <head>")]
    NoHead,
}

pub fn csp() -> Result<(), Error> {
    if std::env::var("TRUNK_PROFILE").as_deref() != Ok("release") {
        println!("csp: not a release build, no policy added");
        return Ok(());
    }
    let staging = std::env::var_os("TRUNK_STAGING_DIR").ok_or(Error::NotAHook)?;
    let page = PathBuf::from(staging).join("index.html");
    let html = at(&page, fs::read_to_string(&page))?;
    Ok(at(&page, fs::write(&page, with_policy(&html)?))?)
}

/// Everything the page loads, and nothing else:
/// - scripts: its own, wasm, and Trunk's inline loader by hash;
/// - the sentence data from its own origin; styles and images likewise
///   (system fonts need no source);
/// - recordings from wherever [`AudioId::urls`] points.
///
/// No Trusted Types: Leptos' `view!` builds static elements from the HTML
/// it generates at compile time, through `innerHTML` on a `<template>`.
///
/// `frame-ancestors` cannot be set from a `<meta>`, and GitHub Pages sends
/// no headers of ours: the app checks for a frame itself (`framed`).
fn policy(inline_scripts: &[&str]) -> String {
    let hashes: String = inline_scripts
        .iter()
        .map(|script| {
            let digest = Sha256::digest(script.as_bytes());
            let hash = base64::engine::general_purpose::STANDARD.encode(digest);
            format!(" 'sha256-{hash}'")
        })
        .collect();
    // Each URL's origin: `https://host`.
    let mut media: Vec<String> = Vec::new();
    for url in AudioId(0).urls() {
        let origin = url.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
        if !media.contains(&origin) {
            media.push(origin);
        }
    }
    format!(
        "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'{hashes}; \
         connect-src 'self'; style-src 'self'; img-src 'self'; media-src {}; \
         base-uri 'none'; form-action 'none'",
        media.join(" ")
    )
}

/// `html` with the policy as its first `<head>` element.
fn with_policy(html: &str) -> Result<String, Error> {
    if html.contains("Content-Security-Policy") {
        return Err(Error::HasPolicy);
    }
    // Inline scripts: `<script …>` without `src`, up to `</script>`.
    let mut inline = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<script") {
        let tag_end = rest[start..].find('>').ok_or(Error::UnclosedScript)? + start + 1;
        let end = rest[tag_end..]
            .find("</script>")
            .ok_or(Error::UnclosedScript)?
            + tag_end;
        if !rest[start..tag_end].contains(" src=") {
            inline.push(&rest[tag_end..end]);
        }
        rest = &rest[end..];
    }
    let head = html.find("<head>").ok_or(Error::NoHead)? + "<head>".len();
    let meta = format!(
        "\n    <meta http-equiv=\"Content-Security-Policy\" content=\"{}\" />",
        policy(&inline)
    );
    Ok([&html[..head], &meta, &html[head..]].concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_inline_scripts_and_allows_the_recordings() -> Result<(), Error> {
        let html = "<html><head><meta charset=\"utf-8\" />\
            <script type=\"module\">init();</script>\
            <script src=\"/a.js\"></script></head><body></body></html>";
        let out = with_policy(html)?;
        // sha256("init();"), base64.
        let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(b"init();"));
        assert!(out.contains(&format!("'sha256-{hash}'")), "{out}");
        assert_eq!(
            out.matches("'sha256-").count(),
            1,
            "external scripts are not hashed"
        );
        assert!(out.contains("media-src https://tatoeba.org https://api.tatoeba.org;"));
        assert!(out.starts_with("<html><head>\n    <meta http-equiv=\"Content-Security-Policy\""));
        assert!(
            matches!(with_policy(&out), Err(Error::HasPolicy)),
            "applied twice"
        );
        Ok(())
    }
}
