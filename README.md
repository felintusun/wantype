# WanType ワンタイプ

▼・ᴥ・▼✧ A Japanese typing game, with Pochi. It is a Rust rewrite of
[Taipingu](https://github.com/entibo/taipingu) by Thibaut Guénédal: type
Tatoeba sentences in romaji or kana, with furigana, translations in many
languages and native-speaker recordings.
Like upstream it is a static site (no backend), built with
[Leptos](https://leptos.dev) on a sans-IO Rust engine. Text uses the reader's
own Japanese system fonts (Hiragino, Yu Gothic/Meiryo, Noto CJK).

| Path | What |
| --- | --- |
| `core/` (`wantype-core`) | The engine, no I/O: upstream's matcher with the romaji rules as Rust data, the sentence model (furigana, readings, translations), sentence selection from upstream's data, levels and score, and the game as a sans-IO state machine. |
| `app/` (`wantype`) | The Leptos site: draws the sentence on manuscript paper, wires keys and buttons to the session, fetches sentences, plays audio, saves settings. |
| `e2e/` | Browser test of the built site (WebDriver via `fantoccini` + `geckodriver`). |
| `xtask/` | `cargo xtask data`: builds the sentence data from Tatoeba's exports, checking every sentence with `core`. `cargo xtask csp`: adds the Content-Security-Policy to release builds. |

Keys as upstream: type the sentence, **Tab** furigana, **Space** play audio
(turns it on; Shift+Space off), **Enter** next sentence, **Esc** restart. On
the first run, **1** or **2** picks the mode.

## The game

The sentence is written down a sheet of manuscript paper (原稿用紙,
縦書き), one char per square, furigana in the strip beside its kanji. Untyped
text is faint and typed text full ink; a miss leaves a vermilion dot in its
square and the key hint (ヒント) sits just beside the square to type. Under
a light red rule are the play row (the helpers while typing, the result
once done; retry and next) and the status row (options, voice, level and
score), kept small so the sentence stays what the eye is on.

The first run asks how to play, and remembers it:

- **Levels and score (級).** Five clean sentences (95% accuracy or more)
  reach the next level, and each level fades one helper from the next
  sentence: 入門 shows everything, 初級 drops the key hint, 中級 the
  translation, 上級 furigana (numbers turn to 漢数字 there), and at 無心
  nothing shows while typing, not even the controls, until the mouse moves
  or Shift+Tab; a completed sentence leaves only a seal with the score.
  Points are five per correct key, weighed by accuracy and a multiplier:
  ×1, plus ×0.5 per helper off (a faded helper turned back on gives its
  share back), plus ×0.5 at 無心. A helper shown at any point of a sentence
  counts as on. A rough sentence (under 80%) resets the streak and the
  level's progress and offers the last faded helper back; a retry is not
  scored again. A level can also be fixed in the options.
- **Zen (禅).** No score, no levels; helpers stay as set, and the controls
  fade while a sentence is typed and the mouse rests.

The options sheet holds the rest: progress (auto or a fixed level), the
helpers and voice, font, translation language, numbers, new kanji per
sentence and the mode.

## Develop

Needs Rust with the `wasm32-unknown-unknown` target, [just](https://just.systems)
and [Trunk](https://trunkrs.dev) (on PATH or in `.tools/bin/`; Trunk fetches
`wasm-bindgen` and `wasm-opt` itself). Everything else is Rust; the recipes
that need `geckodriver` install it into `.tools/` on first use. Use Trunk's
release binary: building it from source fails with GCC 16 and clang 23,
which reject the AVX-512 code in its bundled `libdeflate`.

```sh
just              # list the tasks
just serve        # http://127.0.0.1:8080, rebuilds and reloads on change
just check        # lint (clippy, zizmor), audit, tests, full-data test
just smoke        # release build (target/site) played in headless Firefox
just icons        # PNG icons rendered from favicon.svg
```

The first `serve`, `build` or data test builds the sentence data (29 MB
into `app/public/sentences`, not committed) from Tatoeba's per-language
exports, downloaded once into `target/tatoeba` (about 170 MB, half a
minute). `just update-data` fetches newer exports, which Tatoeba publishes
weekly, and rebuilds.

## Design

- **Leptos, client-side only.** The browser renders the text as upstream
  does (ruby, system fonts), and its wasm was the smallest of the four
  ports compared before (Leptos, Dioxus, Slint, Bevy; see
  `EVALUATION.md` at the `framework-comparison` tag). Native apps
  are out of scope.
- **Sans-IO engine.** `wantype_core::Session` takes events with the current
  time and returns effects (load a sentence, play audio, save settings),
  so every game rule is tested without a browser or a clock.
- **Upstream's settings format.** Settings use upstream's `localStorage`
  fields (under the key `wantype`: a GitHub Pages origin is shared by all of
  an account's projects), so upstream's saved settings and known kanji read
  as they are; the mode, level and score are added fields upstream ignores.
- **Our own sentence data, built like upstream's.** `cargo xtask data`
  follows upstream's scripts (reviewed furigana only, the same length and
  character limits), and adds two things. It checks every sentence with
  this project's parser and matcher, so stale furigana or untypeable text
  never ships. Its format is made for what a player downloads: index
  lists stored as gaps between numbers, readings as a table plus numbers,
  and sentences shuffled across chunks so the app plays a fetched chunk
  whole. A version directory named by the content keeps caches from
  mixing two builds.
- **Romaji rules as Rust data, held to standard IME input.** Google's IME
  table (Mozc, vendored in `core/tests/data`) is the reference, tested both
  ways: every Mozc spelling is accepted, and every accepted spelling
  converts back to the same kana. Hints teach Hepburn (`shi`, `tsu`, `nn`).
- **System fonts** (Hiragino, Yu Gothic or Meiryo, Noto CJK), as most
  Japanese sites use: native look, no download. Only Linux has been checked
  visually.
- **Small wasm.** The release profile optimises for size (`opt-level = "z"`,
  LTO, one codegen unit, `wasm-opt -Oz`; `"s"` measured 6% bigger). The
  Unicode compatibility forms the game folds (fullwidth, halfwidth, the
  symbols that stand for kana) are tables generated from NFKC in
  `core/src/compat.rs` instead of the normalization library, which was a
  fifth of the download; tests check the tables against it, and against
  every char of the data.

## Differences from upstream

Fixes:

- A reading counts as known only if furigana stayed hidden for the whole
  sentence (upstream marks every reading known when furigana stay on).
- Sentences with empty reading fields (`[田舎道|いなか||みち]`, `[㌔|]`) can
  be completed; fullwidth and halfwidth text is typed with plain keys.
- No overrun after the end of a sentence, no crash on sentences ending in
  kanji, and a notice when no sentence matches the settings.
- A malformed field in the saved settings takes its default instead of
  resetting them all (known kanji included); a saved translation language
  the data lacks falls back to English. The second audio source moved to
  Tatoeba's stable API (upstream's now answers 404).
- Sentences are picked from all of them whatever the translation
  language: one without a translation in it shows the English one
  (97.5% have one) instead of upstream's placeholder, so a language with
  few translations still plays the whole set. A first run translates
  into the browser's language (`navigator.languages`) when Tatoeba has it.

Input follows standard IME romaji instead of upstream's own table:

- Every standard spelling works: `jya`, `whi`, `ci`, the `q`/`kw`/`gw` rows,
  `t'i`, small kana and っ typed on their own (`kixyo`, `kixtute`), `tch`
  for っち, Mozc's `z` keys (`z.` for …).
- Spellings no IME accepts are gone: `wya`, `dzu`, `tsya`, `nee` for ねぇ
  (type `nexe`), lenient brackets like `(` for 「. Punctuation uses the IME
  keys (`,` 、 `.` 。 `/` ・ `[` 「 `]` 」); dashes and curly quotes take the
  nearest key (`-`, `"`).

Data:

- Built from current Tatoeba exports: 100,308 sentences in 189 translation
  languages (upstream's copy: 92,449 in 185), each checked to be typeable.
- Far less to download: about 10 KB before the first sentence instead of
  about 190 KB (gzipped), one request per ~50 sentences instead of one per
  sentence, and 581 KB instead of 988 KB for "New kanji".

Usability and accessibility:

- With a Japanese IME on, a notice asks to switch to direct input (upstream
  ignores the keys silently).
- A key that spells a kana by its sound (`wa` for the particle は, `e` for
  へ, `o` for を, the vowel again for ー, `nee` for ねぇ) still counts as a
  miss, as in an IME, but a note says how the kana is typed.
- Shift+Tab moves into the controls, Escape returns to the game; focus is
  visible.
- Accuracy is shown next to WPM.
- The key hint (ヒント) can be turned off while furigana stay on, to read
  the kana and kanji rather than the next letter.
- Levels and score, or Zen (see [The game](#the-game)), instead of
  upstream's free play; the design follows Japanese print instead of
  upstream's pink card. Long readings shrink to fit their squares.
- Translations carry their language (`lang`), so Chinese uses Chinese
  glyphs.
- "Reduce motion" turns the animations off, and furigana, translations and
  the controls meet WCAG AA contrast.

Left out: upstream's analytics, community links and social meta tags (they
belong to its deployment).

## Deploy

The page carries a strict Content-Security-Policy (`cargo xtask csp`, run
by Trunk after release builds): scripts, styles, images and data only from
the site itself, recordings only from Tatoeba. Headers a `<meta>` cannot
set (HSTS, `frame-ancestors`, `X-Content-Type-Options`) belong in the
host's configuration.

`just build` writes the whole site to `target/site/` (≈ 582 KB wasm, 241 KB
gzipped, plus the sentence data): upload it to any static host. Its paths
are relative, so it runs at any path; the browser tests serve it under
`/repo/`, as GitHub Pages serves a project page.

`.github/workflows/pages.yml` runs the unit tests, builds and publishes on
GitHub Pages on every push to `main` (Settings → Pages → Source: GitHub
Actions), and monthly with Tatoeba's newer exports. The built data is
cached for the month (rebuilt sooner when the code that builds it
changes), the exports too. Pages compresses with gzip itself (it neither makes nor
serves brotli, 194 KB here), so nothing is precompressed.

## Licences

Code: MIT (`LICENSE.txt`, shipped with the site and linked from the
options), keeping upstream's notice (the stylesheet, icons markup, animations and
sentence-selection logic are ported from it; `xtask/data/language-names.json`
is upstream's). Sentences: Tatoeba, CC BY 2.0 FR. Icons and Pochi: the app's own. Recordings:
streamed from Tatoeba, licences per speaker.
