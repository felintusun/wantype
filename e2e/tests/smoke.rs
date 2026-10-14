//! Plays the release build in headless Firefox: the first run's choice of
//! mode, sentences typed key by key, the helpers and voice, the options,
//! a new level, Zen and 無心, a reload keeping it all. Screenshots land in
//! `target/smoke/`.
//!
//! Opt-in (needs `target/site` and `geckodriver` on PATH): `just smoke`, or
//!
//!     cargo test -p e2e --test smoke -- --ignored

mod common;

use common::*;
use fantoccini::{Client, Locator};
use serde_json::json;

/// Record Content-Security-Policy violations from here on.
async fn watch_policy(client: &Client) -> E2eResult {
    js(
        client,
        "(window.blocked = [], document.addEventListener('securitypolicyviolation', \
           e => blocked.push(e.effectiveDirective + ' ' + e.blockedURI)), true)",
    )
    .await?;
    Ok(())
}

/// The translation's `lang`.
const TRANSLATION_LANG: &str = "document.getElementById('translationElement').lang";
/// Its text.
const TRANSLATION_TEXT: &str = "document.getElementById('translationElement').textContent";

const CONTROLS_FADED: &str =
    "getComputedStyle(document.querySelector('.controls')).opacity === '0'";
const CONTROLS_SHOWN: &str =
    "getComputedStyle(document.querySelector('.controls')).opacity === '1'";

/// The options sheet is closed, or open.
const OPTIONS_CLOSED: &str = "document.querySelector('.options').hidden";
const OPTIONS_OPEN: &str = "!document.querySelector('.options').hidden";

/// The phone's lens shows the whole sentence as a page.
const PAGED: &str = "document.querySelector('.lens.page') !== null";
/// Every square of the sentence lies within the lens.
const WHOLE: &str = "(lens => { \
    const b = lens.getBoundingClientRect(); \
    return [...lens.querySelectorAll('.genko-square')].every(s => { \
      const r = s.getBoundingClientRect(); \
      return r.top >= b.top - 0.5 && r.bottom <= b.bottom + 0.5 \
        && r.left >= b.left - 0.5 && r.right <= b.right + 0.5 }) \
  })(document.querySelector('.lens'))";

/// What in `arguments[0]` (a selector) leaves the window, or sticks out of
/// a row too narrow for what it holds, by name.
const CUT_OFF: &str = "(root => { \
    const name = e => e.tagName.toLowerCase() + (e.id ? '#' + e.id : '') \
      + [...e.classList].map(c => '.' + c).join(''); \
    const out = [root, ...root.querySelectorAll('*')].filter(e => { \
      const r = e.getBoundingClientRect(); \
      return r.width > 0 && (r.left < 0 || r.right > innerWidth) }); \
    const tight = [...root.querySelectorAll('.play-row, .status-row, .welcome-cards')] \
      .flatMap(row => { \
        const r = row.getBoundingClientRect(); \
        return [...row.querySelectorAll('*')].filter(e => { \
          const c = e.getBoundingClientRect(); \
          return c.width > 0 && (c.left < r.left - 0.5 || c.right > r.right + 0.5) }) }); \
    return [...out, ...tight].map(name) })(document.querySelector(arguments[0]))";

#[tokio::test]
#[ignore]
async fn plays_sentences_in_firefox() -> E2eResult {
    let out = out_dir("smoke")?;
    let (client, _driver) = open().await?;
    // The release build runs under a strict Content-Security-Policy (its
    // loader got this far); from here on, nothing may be blocked.
    let policy = "!!document.querySelector('meta[http-equiv=Content-Security-Policy]')";
    assert_eq!(js(&client, policy).await?, json!(true), "no CSP");
    watch_policy(&client).await?;

    // The licence ships with the site, upstream's notice in it, and the
    // credits link to it from the sub-path the site is served at.
    js(
        &client,
        "(fetch(document.querySelector('.credits a[href=\"LICENSE.txt\"]').href) \
           .then(r => r.text()).then(t => window.licence = t), true)",
    )
    .await?;
    wait_for(
        &client,
        "the licence",
        "(window.licence || '').includes('Copyright (c) 2025 Thibaut Guénédal')",
    )
    .await?;
    // Pochi, beside the name in the options, links to the source.
    assert_eq!(
        js_str(&client, "document.querySelector('.links a.pochi').href").await?,
        env!("CARGO_PKG_REPOSITORY")
    );
    // So do the icons, each found where its link says.
    js(
        &client,
        "(Promise.all([...document.querySelectorAll('link[rel$=icon]')] \
           .map(l => fetch(l.href).then(r => r.ok && r.headers.get('Content-Type').startsWith('image/')))) \
           .then(found => window.icons = found), true)",
    )
    .await?;
    wait_for(
        &client,
        "the three icons",
        "Array.isArray(window.icons) && window.icons.length === 3 && window.icons.every(Boolean)",
    )
    .await?;

    // The first run asks how to play; 1 picks levels and score.
    let welcome = "!document.querySelector('.welcome').hidden";
    assert_eq!(
        js(&client, welcome).await?,
        json!(true),
        "no first-run choice"
    );
    screenshot(&client, &out, "welcome").await?;
    press(&client, "1").await?;
    wait_for(&client, "the choice to close", &format!("!({welcome})")).await?;
    screenshot(&client, &out, "start").await?;

    // チャンスは…: `w` spells the particle は by its sound, and a note says how
    // it is typed. The square to type flashes and keeps its miss mark.
    // The sheet is built once per sentence: keys change its squares'
    // classes, not the squares.
    js(
        &client,
        "(window.firstSquare = document.querySelector('.genko-square'), true)",
    )
    .await?;
    type_to(&client, 'は').await?;
    press(&client, "w").await?;
    assert!(
        js_bool(
            &client,
            "document.querySelector('.genko-square') === window.firstSquare && \
             window.firstSquare.classList.contains('done')",
        )
        .await?,
        "the sheet was rebuilt for a key"
    );
    wait_for(
        &client,
        "the note on は",
        "!document.getElementById('slipNote').classList.contains('hidden') && \
         document.getElementById('slipNote').textContent.includes('は is typed ha')",
    )
    .await?;
    wait_for(
        &client,
        "the square to flash",
        "document.querySelector('.genko-square.active').getAnimations().length > 0",
    )
    .await?;
    screenshot(&client, &out, "slip").await?;
    // Finishing the sentence changes what the rows show, but nothing moves:
    // the next button keeps its width, the rows their place.
    let layout = "[document.getElementById('nextSentenceButton').getBoundingClientRect().width, \
                  document.querySelector('.play-row').getBoundingClientRect().top, \
                  document.querySelector('.status-row').getBoundingClientRect().top].join()";
    let before = js_str(&client, layout).await?;
    type_sentence(&client).await?;
    assert_eq!(js_str(&client, layout).await?, before, "the controls moved");
    assert!(
        js_bool(
            &client,
            "!!document.querySelector('.genko-square.done.error')"
        )
        .await?,
        "the miss mark is gone"
    );
    // The result: points, speed and accuracy; the score counts them.
    let gain = js_str(
        &client,
        "document.querySelector('.result .gain').textContent",
    )
    .await?;
    assert!(gain.starts_with('+'), "gain {gain:?}");
    let sub = js_str(
        &client,
        "document.querySelector('.result .sub').textContent",
    )
    .await?;
    assert!(sub.contains("wpm") && sub.contains('%'), "result {sub:?}");
    screenshot(&client, &out, "done").await?;

    press(&client, "\u{E007}").await?; // Enter
    wait_for(
        &client,
        "a Tatoeba sentence",
        "!!document.getElementById('sentenceOriginLink').getAttribute('href')",
    )
    .await?;
    type_sentence(&client).await?;
    screenshot(&client, &out, "next").await?;

    // The translation is tagged and stays inside the card.
    let lang = js_str(&client, TRANSLATION_LANG).await?;
    assert_eq!(lang, "en");
    let fits = js(
        &client,
        "(() => { const t = document.getElementById('translationElement').getBoundingClientRect(); \
           const m = document.querySelector('main').getBoundingClientRect(); \
           return t.left >= m.left && t.bottom <= m.bottom })()",
    )
    .await?;
    assert_eq!(fits, json!(true), "the translation leaves the card");

    // A key that went to a Japanese IME shows the notice; the next real key
    // hides it.
    let notice_hidden = "document.getElementById('imeNotice').classList.contains('hidden')";
    js(
        &client,
        "(dispatchEvent(new KeyboardEvent('keydown', { key: 'Process', isComposing: true })), true)",
    )
    .await?;
    wait_for(&client, "the IME notice", &format!("!{notice_hidden}")).await?;
    js(
        &client,
        "(dispatchEvent(new KeyboardEvent('keydown', { key: 'Shift' })), true)",
    )
    .await?;
    wait_for(&client, "the IME notice to hide", notice_hidden).await?;

    // Shift+Tab moves keyboard focus into the controls; Escape returns to
    // the game.
    press(&client, "\u{E008}\u{E004}\u{E000}").await?; // Shift+Tab
    wait_for(
        &client,
        "a keyboard-focused control",
        "!!document.activeElement.closest('.controls') && document.activeElement.matches(':focus-visible')",
    )
    .await?;
    screenshot(&client, &out, "focus").await?;
    press(&client, "\u{E00C}").await?; // Escape
    wait_for(
        &client,
        "focus back on the game",
        "document.activeElement === document.body",
    )
    .await?;

    // Tab hides furigana; Space turns the voice on (and saves the setting).
    press(&client, "\u{E004}").await?; // Tab
    assert!(!body_has(&client, "furigana").await?);
    press(&client, " ").await?;
    wait_for(
        &client,
        "voice on",
        "document.getElementById('voiceButton').classList.contains('on')",
    )
    .await?;
    // Clicked once on, the 音声 button plays the recording again, as Space
    // does; it does not turn the voice off.
    click(&client, "#voiceButton").await?;
    assert!(
        js_bool(
            &client,
            "document.getElementById('voiceButton').classList.contains('on')"
        )
        .await?,
        "a click turned the voice off"
    );
    assert_eq!(js(&client, "blocked").await?, json!([]), "CSP violations");

    // Settings and the mode survive a reload.
    reload(&client).await?;
    watch_policy(&client).await?;
    let audio = client
        .find(Locator::Id("audioCheckbox"))
        .await
        .map_err(|source| E2eError::Browser {
            what: "#audioCheckbox".into(),
            source,
        })?
        .prop("checked")
        .await
        .map_err(|source| E2eError::Browser {
            what: "#audioCheckbox checked".into(),
            source,
        })?;
    assert_eq!(audio.as_deref(), Some("true"));
    assert!(!body_has(&client, "furigana").await?);
    assert_eq!(js(&client, welcome).await?, json!(false), "asked again");

    // The options: the button opens them, Escape closes them. Another
    // language there shows its translation where there is one.
    press(&client, "\u{E004}").await?; // Tab: furigana back
    click(&client, "#optionsButton").await?;
    wait_for(&client, "the options", OPTIONS_OPEN).await?;
    screenshot(&client, &out, "options").await?;
    js(
        &client,
        "(() => { const s = document.getElementById('languageSelect'); s.value = 'deu'; \
           s.dispatchEvent(new Event('change')); return true })()",
    )
    .await?;
    press(&client, "\u{E00C}").await?; // Escape
    wait_for(&client, "the options to close", OPTIONS_CLOSED).await?;
    // The sentence stays: without German it shows the English translation,
    // tagged so, and without either none. Skipping on comes to a German
    // one (a third of those with a recording).
    let mut lang = js_str(&client, TRANSLATION_LANG).await?;
    for _ in 0..40 {
        if lang.is_empty() {
            let text = js_str(&client, TRANSLATION_TEXT).await?;
            assert_eq!(text, "", "a translation without a language");
        }
        assert!(
            ["de", "en", ""].contains(&lang.as_str()),
            "a translation in {lang:?}"
        );
        if lang == "de" {
            break;
        }
        let id = js_str(
            &client,
            "document.getElementById('sentenceOriginLink').href",
        )
        .await?;
        press(&client, "\u{E007}").await?; // Enter: skip
        wait_for_with(
            &client,
            "the next sentence",
            "document.getElementById('sentenceOriginLink').href !== arguments[0]",
            &[json!(id)],
        )
        .await?;
        lang = js_str(&client, TRANSLATION_LANG).await?;
    }
    assert_eq!(lang, "de", "no German translation in 40 sentences");

    // The key hint turns off on its own, furigana staying on; the button
    // keeps its width (its note has a slot as wide as the widest).
    let width = "document.querySelector(\"button.helper[aria-label='Key hint']\").getBoundingClientRect().width";
    let before = js(&client, width).await?;
    click(&client, "button.helper[aria-label='Key hint']").await?;
    wait_for(
        &client,
        "the hint off",
        "!document.body.classList.contains('hint') && document.body.classList.contains('furigana')",
    )
    .await?;
    assert_eq!(
        js(&client, width).await?,
        before,
        "toggling the key hint resized its button"
    );
    screenshot(&client, &out, "helper-off").await?;
    assert_eq!(js(&client, "blocked").await?, json!([]), "CSP violations");

    // A fifth clean sentence reaches 初級, whose key hint fades from the
    // next sentence.
    reload_with(
        &client,
        json!({ "mode": "score", "ladder": { "pips": 4, "score": 900 }, "audio": false }),
    )
    .await?;
    type_sentence(&client).await?;
    wait_for(
        &client,
        "the new level",
        "document.querySelector('.result .headline').textContent.includes('初級')",
    )
    .await?;
    screenshot(&client, &out, "level-up").await?;
    assert!(
        body_has(&client, "hint").await?,
        "faded before the next sentence"
    );
    press(&client, "\u{E007}").await?; // Enter
    wait_for(
        &client,
        "the hint to fade",
        "!document.body.classList.contains('hint')",
    )
    .await?;
    let tier = js_str(&client, "document.querySelector('.tier-name').textContent").await?;
    assert!(tier.contains("初級"), "level {tier:?}");

    // Zen, picked in the options: no score, and the controls step back
    // while typing once the mouse rests; a mouse move brings them back.
    click(&client, "#optionsButton").await?;
    click(&client, "label:has(#zenRadio)").await?;
    press(&client, "\u{E00C}").await?; // Escape
    wait_for(
        &client,
        "Zen",
        "document.getElementById('zenRadio').checked",
    )
    .await?;
    press(&client, &next_key(&client).await?).await?;
    wait_for(&client, "the controls to fade", CONTROLS_FADED).await?;
    screenshot(&client, &out, "zen").await?;
    js(
        &client,
        "(dispatchEvent(new MouseEvent('mousemove', { clientX: 5, clientY: 5 })), true)",
    )
    .await?;
    wait_for(&client, "the controls to return", CONTROLS_SHOWN).await?;

    // 無心: nothing but the sentence while typing, numbers in kanji, and
    // only a seal once complete.
    reload_with(
        &client,
        json!({
            "mode": "score", "ladder": { "tier": "mushin", "score": 1240 },
            "furigana": false, "hint": false, "translationEnabled": false
        }),
    )
    .await?;
    watch_policy(&client).await?;
    assert!(body_has(&client, "step-back").await?, "not stepping back");
    wait_for(&client, "the controls to step back", CONTROLS_FADED).await?;
    type_sentence(&client).await?;
    wait_for(
        &client,
        "the seal",
        "!!document.querySelector('.seal') && document.querySelector('.seal-score').textContent.startsWith('一')",
    )
    .await?;
    screenshot(&client, &out, "mushin").await?;
    assert_eq!(js(&client, "blocked").await?, json!([]), "CSP violations");

    close(client).await
}

/// A first run takes the translation language from the browser, passing
/// over Japanese; the starter, without German, shows the English one.
#[tokio::test]
#[ignore]
async fn first_run_translates_into_the_browser_language() -> E2eResult {
    let (client, _driver) = open_with_languages("ja, de-AT, en").await?;
    wait_for_with(
        &client,
        "German chosen",
        "JSON.parse(localStorage.getItem(arguments[0]))?.language === 'deu'",
        &[json!(STORAGE_KEY)],
    )
    .await?;
    let lang = js_str(&client, TRANSLATION_LANG).await?;
    assert_eq!(lang, "en", "the starter's translation");
    close(client).await
}

/// Inside another site's frame the game does not run (GitHub Pages cannot
/// send `frame-ancestors`): only a link that opens it on its own shows.
#[tokio::test]
#[ignore]
async fn refuses_to_run_in_a_frame() -> E2eResult {
    let (client, _driver, port) = launch(ENGLISH).await?;
    // A blank page has an opaque origin: any other site's would do.
    goto(&client, "about:blank").await?;
    js_with(
        &client,
        "(f => (f.width = 800, f.height = 500, f.onload = () => window.framed = true, \
           f.src = arguments[0], document.body.append(f), true))(document.createElement('iframe'))",
        &[json!(format!("http://127.0.0.1:{port}{BASE}"))],
    )
    .await?;
    wait_for(&client, "the framed site", "window.framed === true").await?;
    enter_frame(&client, 0).await?;
    wait_for(
        &client,
        "the link to open it",
        "document.getElementById('openOnItsOwn') !== null",
    )
    .await?;
    let game = js(&client, "document.getElementById('sentenceContainer')").await?;
    assert_eq!(game, json!(null), "the game ran in a frame");
    leave_frame(&client).await?;
    screenshot(&client, &out_dir("smoke")?, "framed").await?;
    close(client).await
}

/// Whether the page mounted only `arguments[0]`: the keyboard layout
/// (`main`) or the phone's (`.phone`).
const LAYOUT: &str =
    "['main', '.phone'].filter(s => document.querySelector(s)).join() === arguments[0]";

/// Tablets play the keyboard layout, a keyboard of their own being up to
/// the player: in portrait the card narrows to the window with nothing cut
/// off.
#[tokio::test]
#[ignore]
async fn fits_a_tablet() -> E2eResult {
    let out = out_dir("smoke")?;
    let (client, _driver, port) = launch(ENGLISH).await?;
    for (width, height, name) in [(600, 960, "tablet-600"), (768, 1024, "tablet-768")] {
        resize(&client, width, height).await?;
        visit(&client, port).await?;
        let keyboard = js_bool_with(&client, LAYOUT, &[json!("main")]).await?;
        assert!(keyboard, "{name}: not the keyboard layout");
        screenshot(&client, &out, &format!("{name}-welcome")).await?;
        let cut = js_with(&client, CUT_OFF, &[json!("main")]).await?;
        assert_eq!(cut, json!([]), "{name}: the choice cut off");
        reload_with(&client, json!({ "mode": "score" })).await?;
        screenshot(&client, &out, name).await?;
        let cut = js_with(&client, CUT_OFF, &[json!("main")]).await?;
        assert_eq!(cut, json!([]), "{name}: the game cut off");
        js(&client, "(localStorage.clear(), true)").await?;
    }
    // Narrowed past a phone's width and back, as the browser's responsive
    // mode does, the layout follows without a reload.
    resize(&client, 500, 900).await?;
    wait_for_with(&client, "the phone layout", LAYOUT, &[json!(".phone")]).await?;
    resize(&client, 768, 1024).await?;
    wait_for_with(&client, "the keyboard layout", LAYOUT, &[json!("main")]).await?;
    close(client).await
}

/// A phone plays with the flick pad: following its hints (a key, a flick,
/// ゛゜小 for ャ, ド and デ) types the starter, the column sliding up past the
/// square to type.
#[tokio::test]
#[ignore]
async fn plays_on_a_phone() -> E2eResult {
    let out = out_dir("smoke")?;
    let (client, _driver, port) = launch(ENGLISH).await?;
    // Headless Firefox goes no narrower than 500 px: still a phone.
    resize(&client, 500, 900).await?;
    visit(&client, port).await?;
    let phone = js_bool_with(&client, LAYOUT, &[json!(".phone")]).await?;
    assert!(phone, "not the phone layout");
    watch_policy(&client).await?;
    screenshot(&client, &out, "phone-welcome").await?;
    click(&client, ".welcome-card").await?;
    wait_for(
        &client,
        "the choice made",
        "document.querySelector('.welcome').hidden",
    )
    .await?;
    screenshot(&client, &out, "phone").await?;

    // The hinted key, as its place on the pad and the way to flick.
    let hinted = "(k => k && [[...k.parentNode.children].indexOf(k) + 1, k.dataset.hint]) \
        (document.querySelector('.pad-key[data-hint]'))";
    let at = "getComputedStyle(document.querySelector('.lens')).getPropertyValue('--at')";
    let mut flicks = 0;
    while let serde_json::Value::Array(hint) = js(&client, hinted).await? {
        let [serde_json::Value::Number(key), serde_json::Value::String(way)] = hint.as_slice()
        else {
            return Err(E2eError::Type {
                expression: hinted.into(),
                value: serde_json::Value::Array(hint),
                kind: "[key, flick]",
            });
        };
        let (dx, dy) = match way.as_str() {
            "left" => (-40.0, 0.0),
            "up" => (0.0, -40.0),
            "right" => (40.0, 0.0),
            "down" => (0.0, 40.0),
            _ => (0.0, 0.0),
        };
        touch(&client, &format!(".pad-key:nth-child({key})"), dx, dy).await?;
        flicks += 1;
        if flicks == 6 {
            assert_eq!(
                js_str(&client, at).await?,
                "5",
                "the column slid past チャンスは"
            );
            screenshot(&client, &out, "phone-typing").await?;
            // Turned to the keyboard layout and back, as a phone turned by
            // mistake, the sentence is where it was.
            resize(&client, 768, 1024).await?;
            wait_for_with(&client, "the keyboard layout", LAYOUT, &[json!("main")]).await?;
            resize(&client, 500, 900).await?;
            wait_for_with(&client, "the phone layout", LAYOUT, &[json!(".phone")]).await?;
            assert_eq!(js_str(&client, at).await?, "5", "the place kept");
            // A tap on the lens shows the whole sentence, on a page within
            // it; the next flick brings the lens back.
            touch(&client, ".lens-toggle", 0.0, 0.0).await?;
            wait_for(&client, "the page", PAGED).await?;
            let columns = "document.querySelectorAll('.lens .genko-line').length";
            let columns = js(&client, columns).await?.as_u64();
            assert!(columns > Some(1), "the page in {columns:?} columns");
            assert!(js_bool(&client, WHOLE).await?, "the page cut off");
            screenshot(&client, &out, "phone-page").await?;
        }
        if flicks == 7 {
            assert!(!js_bool(&client, PAGED).await?, "the page stayed");
        }
        assert!(flicks < 40, "the hints went round in circles");
    }
    // チャンスは何度でもあるワン！: 15 kana and a mark, ャ, ド and デ turned.
    assert_eq!(flicks, 18, "flicks for the starter");
    let done = "document.querySelector('.phone-bar .ready') !== null";
    assert!(js_bool(&client, done).await?, "the starter not completed");
    screenshot(&client, &out, "phone-done").await?;

    // The options open from the bar, within the phone, and close again.
    click(&client, "#optionsButton").await?;
    wait_for(&client, "the options", OPTIONS_OPEN).await?;
    let cut = js_with(&client, CUT_OFF, &[json!(".options")]).await?;
    assert_eq!(cut, json!([]), "the options cut off");
    screenshot(&client, &out, "phone-options").await?;
    click(&client, ".options .sheet-head .button").await?;
    wait_for(&client, "the options to close", OPTIONS_CLOSED).await?;

    // The next sentence comes through the lens, not on the page.
    touch(&client, ".lens-toggle", 0.0, 0.0).await?;
    wait_for(&client, "the page", PAGED).await?;
    click(&client, ".phone-bar .ready").await?;
    assert!(
        !js_bool(&client, PAGED).await?,
        "the next sentence on the page"
    );
    assert_eq!(js(&client, "blocked").await?, json!([]), "CSP violations");
    close(client).await
}
