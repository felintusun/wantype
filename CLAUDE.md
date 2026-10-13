# Working on WanType

Rust workspace: `core/` (sans-IO engine), `app/` (Leptos 0.8, client-side
only, Trunk), `xtask/` (data build, CSP), `e2e/` (Firefox via geckodriver).
`just` lists the tasks; `just check`, `just smoke` and `just screens` must
pass before a commit.

## Errors

- Never swallow an error: no `let _ =` on a `Result`, no `.ok()`,
  `unwrap_or_default()` or `Err(_) =>` that drops a failure. Each one is
  returned, shown to the player, or reported to the console
  (`js::report`, `Reported::reported`) saying what failed.
- No `unwrap`, `expect`, `panic!` or `unreachable!`, in code or tests.
  Restructure so the case cannot occur, or return an error. Tests return
  `Result` (`TestResult` in `core`, `E2eResult` in `e2e`) and use `?`;
  assertions stay assertions.
- Errors are `thiserror` enums, one per module or crate, whose variants
  say what failed and keep the cause as `#[source]`. Never `String`
  errors, never `Box<dyn Error>`. Messages do not repeat their source;
  `js::chain` and the xtask's `fail` print the whole chain.
- An `Option` is not an error: a missing translation or an empty list
  defaulting is fine. A `Result` is.

## Tests

- No fixed sleeps, in tests as in code: wait on a condition
  (`common::wait_for`, `common::settled`).
- Game rules live in `core` and are unit-tested there; the browser tests
  check what only a browser can (rendering, keys, storage, the CSP).

## Design

- Only the sentence and the square to type move. Controls never slide,
  grow, change width or shift rows: text that changes sits in a `.slot` as
  wide as its widest variant, rows have fixed heights, hidden lines keep
  their space (`visibility`). Nothing seeks attention: no filled primary
  button, faint ruling at rest, more on hover.
- Japanese practice (デジタル庁, SmartHR): Japanese labels first, as verbs
  (やり直す, 飛ばす, 次へ), English as the small legend; Japanese text at
  14 px or more with 2% letter spacing, 1.7 line height in prose; no
  italics; a toggle is on with a maru (〇), never a ✓ (in Japan it marks
  what is wrong); multipliers in 倍, never × (batsu).
- Toggles are buttons with `aria-pressed` and a constant label; their
  state shows in shape (the box), not colour alone.

## Leptos (from the Leptos book)

Reactivity
- A component runs once: it is setup, not a render function. Only signals
  and closures that read them are reactive in a view; `{x.get()}` renders
  once, `{move || x.get()}` or `{x}` updates.
- Derive, do not synchronise: a value computed from signals is a closure
  (cheap) or a `Memo` (expensive, or read in many places, or to stop
  propagation when the result does not change). Never write a signal from
  an `Effect` to mirror another one; update related signals together in
  the event handler instead.
- Effects are only for the outside world (the body's classes, focus,
  timers, storage). They track what they read; `Effect::watch` makes the
  trigger explicit.
- Read without cloning: `.read()` or `.with()` instead of `.get()` on
  large values; mutate in place with `.write()`/`.update()`. Never hold a
  read guard while writing, or a write guard while reading.
- Memos compare before notifying, signals always notify: memoize the
  final value before an expensive effect, not every step. Keep memos
  narrow, and share large values (`Arc`) so copying and comparing them is
  cheap (the known readings in `Settings`).
- Browser (`!Send`) values go in local storage: `StoredValue::new_local`,
  `RwSignal::new_local`, `LocalResource`.
- Arena signals (`RwSignal`, `StoredValue`) are `Copy` handles owned by
  the reactive owner that created them; using one after its owner is
  disposed panics. Create signals where they live; for collections of
  rows, use `ArcRwSignal`.

Views
- Control flow is Rust (`if`, `match`, `Option`, `bool::then`) inside
  `move ||`. Branches of different types join with `Either` or
  `.into_any()`. For expensive branches that toggle, use `<Show/>` (it
  memoizes the condition and builds each branch once); a cheap text or
  class toggle stays a closure or `class:`.
- Lists: a plain `Vec` (`collect_view`) when the list does not change;
  `<For/>` with a stable, unique key (never an index) when rows are
  added, removed or moved.
- Errors in views: a `Result` renders its `Ok` value; wrap fallible parts
  in `<ErrorBoundary/>` with a fallback that lists what failed, instead of
  defaulting the value. Keep failures in `Result` signals.
- Inputs: `prop:value` / `prop:checked` (not the `value` attribute) for
  controlled inputs, `on:input` for live updates, `on:change` for
  `<select>`; `bind:` is the shorthand.
- Attributes and styles: `class:name=`, `style:prop=`, and the tuple forms
  for CSS variables. This app runs under a strict CSP: no inline `style`
  attributes in markup; dynamic styles go through `style:` (the CSSOM).
- `NodeRef::get` is `None` until mounted: read it in effects or event
  handlers, and handle `None`. Prefer markup and Leptos' `on:` handlers to
  building DOM or `Closure`s by hand.

Components and data flow
- Props that change are `Signal<T>` (with `#[prop(into)]`, and
  `Signal::derive` for closures); plain props are read once.
  `#[prop(optional)]` / `#[prop(default = …)]` for optional ones; document
  props with `///`.
- Child to parent: a callback prop, or `on:` on the component for DOM
  events; pass a `WriteSignal` only for small, local cases. Shared state
  deep in the tree: `provide_context` with a newtype.
- `Children` is called once; use `ChildrenFn` to render more than once.

Async
- Data that loads with the page or a signal is a `LocalResource` read
  under `<Suspense/>` (or `Suspend`); one-off work in response to an event
  is an `Action` (it tracks pending and the result); `spawn_local` plus a
  signal is the fallback when neither fits.

Testing
- Keep logic out of components: plain types with methods, unit-tested.
  Test components end to end in a browser.
