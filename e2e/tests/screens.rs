//! Captures the game's screens from the release build, as markup for the
//! design canvas: each screen's `<main>` with its live state written out
//! (selected options, checked boxes, finished animations), the body's
//! classes, and the stylesheet with `body` rules moved onto an
//! `.app` root so a screen renders inside any page. Output in
//! `target/screens/`.
//!
//! Opt-in: `just screens`, or
//!
//!     cargo test -p e2e --test screens -- --ignored

mod common;

use common::*;
use serde_json::{Value, json};

/// `<main>` as markup, with what lives in properties and animations
/// written into attributes, and the body's classes.
const CAPTURE: &str = "(() => {
  const main = document.querySelector('main');
  for (const o of main.querySelectorAll('option')) o.toggleAttribute('selected', o.selected);
  for (const c of main.querySelectorAll('input[type=checkbox]')) c.toggleAttribute('checked', c.checked);
  for (const el of main.querySelectorAll('*')) {
    const animations = el.getAnimations().filter((a) => a.effect.getComputedTiming().endTime !== Infinity);
    if (!animations.length) continue;
    for (const a of animations) a.finish();
    const style = getComputedStyle(el);
    for (const p of ['background-color', 'color'])
      el.style.setProperty(p, style.getPropertyValue(p));
    for (const a of animations) a.cancel();
  }
  const copy = main.cloneNode(true);
  const walker = document.createTreeWalker(copy, NodeFilter.SHOW_COMMENT);
  const comments = [];
  while (walker.nextNode()) comments.push(walker.currentNode);
  comments.forEach((c) => c.remove());
  return { classes: document.body.className, html: copy.outerHTML };
})()";

async fn capture(client: &fantoccini::Client, name: &str) -> E2eResult {
    let screen: Value = js(client, CAPTURE).await?;
    let path = workspace().join(format!("target/screens/{name}.json"));
    let json = serde_json::to_string_pretty(&screen).map_err(E2eError::Json)?;
    std::fs::write(&path, json).map_err(|e| E2eError::Write(path, e))?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn captures_the_screens() -> E2eResult {
    let out = out_dir("screens")?;
    let style = workspace().join("app/style.css");
    let css = std::fs::read_to_string(&style).map_err(|e| E2eError::Read(style, e))?;
    // Root variables stay global, so a shared part renders on its own too.
    let css = css.replace("html {", ":root {").replace("body", ".app");
    let path = out.join("style.css");
    std::fs::write(&path, css).map_err(|e| E2eError::Write(path, e))?;

    let (client, _driver) = open().await?;
    capture(&client, "welcome").await?;
    press(&client, "1").await?;
    // Mid-sentence on the starter: up to the な of 何度, with one key
    // spelled by sound (w at は) and its note showing.
    type_to(&client, 'は').await?;
    press(&client, "w").await?;
    type_to(&client, '何').await?;
    type_keys(&client, 2).await?;
    wait_for(
        &client,
        "the note on は",
        "!document.getElementById('slipNote').classList.contains('hidden')",
    )
    .await?;
    settled(&client).await?;
    capture(&client, "typing").await?;
    type_sentence(&client).await?;
    settled(&client).await?;
    capture(&client, "complete").await?;
    click(&client, "#optionsButton").await?;
    capture(&client, "options").await?;

    // A new level, from a clean fifth sentence.
    reload_with(
        &client,
        json!({ "mode": "score", "ladder": { "pips": 4, "score": 900 } }),
    )
    .await?;
    type_sentence(&client).await?;
    capture(&client, "level-up").await?;

    // Zen while typing, the mouse at rest.
    reload_with(&client, json!({ "mode": "zen" })).await?;
    type_keys(&client, 4).await?;
    wait_for(
        &client,
        "the controls to step back",
        "getComputedStyle(document.querySelector('.controls')).opacity === '0'",
    )
    .await?;
    capture(&client, "zen").await?;

    // 無心, complete: the seal.
    reload_with(
        &client,
        json!({
            "mode": "score", "ladder": { "tier": "mushin", "score": 11250 },
            "furigana": false, "hint": false, "translationEnabled": false
        }),
    )
    .await?;
    type_sentence(&client).await?;
    capture(&client, "mushin-complete").await?;
    close(client).await
}
