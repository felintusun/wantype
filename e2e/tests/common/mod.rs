//! What the browser tests share: the built site served on localhost,
//! geckodriver, and helpers to drive the page.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use fantoccini::actions::{InputSource, PointerAction, TouchActions, MOUSE_BUTTON_LEFT};
use fantoccini::error::{CmdError, NewSessionError};
use fantoccini::{Client, ClientBuilder};
use hyper_util::client::legacy::connect::HttpConnector;
use serde_json::json;

pub fn workspace() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
}

pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        // Required for WebAssembly.instantiateStreaming.
        Some("wasm") => "application/wasm",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

/// The app's localStorage key for its settings (`STORAGE_KEY` there).
pub const STORAGE_KEY: &str = "wantype";

/// Where the site is served, as GitHub Pages serves a project page
/// (`/<repo>/`): the build must not use absolute paths.
pub const BASE: &str = "/repo/";

/// The languages a browser prefers unless a test says otherwise.
pub const ENGLISH: &str = "en-US, en";

/// Answer one request: the file under [`BASE`], 404 if there is none, 500
/// if it cannot be read.
pub fn respond(mut stream: TcpStream, root: &Path) -> std::io::Result<()> {
    let mut request_line = String::new();
    BufReader::new(&stream).read_line(&mut request_line)?;
    let target = request_line.split_whitespace().nth(1).unwrap_or("/");
    let target = target.split('?').next().unwrap_or("/");
    let path = match target.strip_prefix(BASE) {
        Some("") => root.join("index.html"),
        Some(t) if !t.split('/').any(|part| part == "..") => root.join(t),
        _ => root.join("missing"),
    };
    let response = match std::fs::read(&path) {
        Ok(body) => [
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                content_type(&path),
                body.len()
            )
            .into_bytes(),
            body,
        ]
        .concat(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }
        Err(e) => {
            eprintln!("test server: {}: {e}", path.display());
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec()
        }
    };
    stream.write_all(&response)
}

/// What failed in a browser test.
#[derive(Debug, thiserror::Error)]
pub enum E2eError {
    #[error("no {0:?}: run `just build` first")]
    NoBuild(PathBuf),
    #[error("the test server")]
    Serve(#[source] std::io::Error),
    #[error("geckodriver (on PATH? `just smoke` installs it)")]
    Spawn(#[source] std::io::Error),
    #[error("geckodriver did not start on port {port}")]
    Connect {
        port: u16,
        #[source]
        source: NewSessionError,
    },
    #[error("{what}")]
    Browser {
        what: String,
        #[source]
        source: CmdError,
    },
    #[error("{expression}: {value} is not a {kind}")]
    Type {
        expression: String,
        value: serde_json::Value,
        kind: &'static str,
    },
    #[error("timed out waiting for {0}")]
    Timeout(String),
    #[error("the sentence was not completed: {0:?}")]
    NotCompleted(String),
    #[error("the hints never led to the square {0}")]
    NotReached(char),
    #[error("{0:?}")]
    Read(PathBuf, #[source] std::io::Error),
    #[error("{0:?}")]
    Write(PathBuf, #[source] std::io::Error),
    #[error("a screen as JSON")]
    Json(#[source] serde_json::Error),
}

pub type E2eResult<T = ()> = Result<T, E2eError>;

/// Name the browser command a failure is about.
fn browser(what: impl Into<String>) -> impl FnOnce(CmdError) -> E2eError {
    let what = what.into();
    move |source| E2eError::Browser { what, source }
}

/// Serves `root` on a free localhost port for the rest of the test run.
pub fn serve(root: PathBuf) -> E2eResult<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(E2eError::Serve)?;
    let port = listener.local_addr().map_err(E2eError::Serve)?.port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(stream) => stream,
                Err(e) => {
                    eprintln!("test server: accepting a connection: {e}");
                    continue;
                }
            };
            let root = root.clone();
            std::thread::spawn(move || {
                if let Err(e) = respond(stream, &root) {
                    eprintln!("test server: answering a request: {e}");
                }
            });
        }
    });
    Ok(port)
}

pub fn free_port() -> E2eResult<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(E2eError::Serve)?;
    Ok(listener.local_addr().map_err(E2eError::Serve)?.port())
}

/// geckodriver, killed when dropped.
pub struct Driver(Child);

impl Drop for Driver {
    fn drop(&mut self) {
        // A drop cannot fail the test; a driver left running is said.
        if let Err(e) = self.0.kill().and_then(|()| self.0.wait().map(drop)) {
            eprintln!("geckodriver did not stop: {e}");
        }
    }
}

/// A headless Firefox through geckodriver on `port`, preferring
/// `languages` (`intl.accept_languages`, as in `de-AT, en`).
pub async fn connect(port: u16, languages: &str) -> E2eResult<Client> {
    let mut capabilities = serde_json::Map::new();
    // Loads finish at DOMContentLoaded: the test waits for the app itself,
    // and the full load event waits for recordings from tatoeba.org.
    capabilities.insert("pageLoadStrategy".into(), json!("eager"));
    capabilities.insert(
        "moz:firefoxOptions".into(),
        json!({
            "args": ["-headless"],
            "prefs": { "media.autoplay.default": 0, "intl.accept_languages": languages },
        }),
    );
    let mut builder = ClientBuilder::new(HttpConnector::new());
    builder.capabilities(capabilities);
    let url = format!("http://127.0.0.1:{port}");
    // geckodriver takes a moment to listen: retry while it starts, and say
    // why the last attempt failed if it never does.
    let mut attempts = 0;
    loop {
        match builder.connect(&url).await {
            Ok(client) => return Ok(client),
            Err(source) if attempts == 50 => return Err(E2eError::Connect { port, source }),
            Err(_starting) => attempts += 1,
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

pub async fn js(client: &Client, expression: &str) -> E2eResult<serde_json::Value> {
    js_with(client, expression, &[]).await
}

/// `expression`'s value, `args` given to it as `arguments[0]`,
/// `arguments[1]`…: values travel as JSON, never spliced into the script.
pub async fn js_with(
    client: &Client,
    expression: &str,
    args: &[serde_json::Value],
) -> E2eResult<serde_json::Value> {
    client
        .execute(&format!("return {expression}"), args.to_vec())
        .await
        .map_err(browser(expression))
}

pub async fn js_str(client: &Client, expression: &str) -> E2eResult<String> {
    match js(client, expression).await? {
        serde_json::Value::String(text) => Ok(text),
        value => Err(E2eError::Type {
            expression: expression.into(),
            value,
            kind: "string",
        }),
    }
}

pub async fn js_bool(client: &Client, expression: &str) -> E2eResult<bool> {
    js_bool_with(client, expression, &[]).await
}

/// [`js_bool`] of `expression`, given `args` ([`js_with`]).
pub async fn js_bool_with(
    client: &Client,
    expression: &str,
    args: &[serde_json::Value],
) -> E2eResult<bool> {
    match js_with(client, expression, args).await? {
        serde_json::Value::Bool(value) => Ok(value),
        value => Err(E2eError::Type {
            expression: expression.into(),
            value,
            kind: "boolean",
        }),
    }
}

pub async fn press(client: &Client, keys: &str) -> E2eResult {
    client
        .active_element()
        .await
        .map_err(browser("the focused element"))?
        .send_keys(keys)
        .await
        .map_err(browser(format!("pressing {keys:?}")))
}

/// The next key to type, shown as the hint or not.
pub async fn next_key(client: &Client) -> E2eResult<String> {
    js_str(
        client,
        "document.getElementById('sentenceContainer').dataset.next",
    )
    .await
}

/// Type the next key until `done` (a JS condition, given `args`) holds:
/// whether it did before the sentence ended. One script per key gives the
/// next key, or `null` once `done`.
async fn type_until(client: &Client, done: &str, args: &[serde_json::Value]) -> E2eResult<bool> {
    let step =
        format!("({done}) ? null : document.getElementById('sentenceContainer').dataset.next");
    for _ in 0..500 {
        match js_with(client, &step, args).await? {
            serde_json::Value::Null => return Ok(true),
            serde_json::Value::String(key) if !key.is_empty() => press(client, &key).await?,
            _ => return Ok(false),
        }
    }
    Ok(false)
}

/// Type the next key until the square to type holds `ch`.
pub async fn type_to(client: &Client, ch: char) -> E2eResult {
    let at =
        "document.querySelector('.genko-square.active')?.firstChild?.nodeValue === arguments[0]";
    match type_until(client, at, &[json!(ch.to_string())]).await? {
        true => Ok(()),
        false => Err(E2eError::NotReached(ch)),
    }
}

/// Type the next `n` keys, or up to the end of the sentence.
pub async fn type_keys(client: &Client, n: usize) -> E2eResult {
    for _ in 0..n {
        let letter = next_key(client).await?;
        if letter.is_empty() {
            break;
        }
        press(client, &letter).await?;
    }
    Ok(())
}

/// Type the next key until the sentence is complete.
pub async fn type_sentence(client: &Client) -> E2eResult {
    type_until(client, "false", &[]).await?;
    let class = js_str(
        client,
        "document.getElementById('sentenceContainer').className",
    )
    .await?;
    match class.contains("complete") {
        true => Ok(()),
        false => Err(E2eError::NotCompleted(class)),
    }
}

/// Close the browser.
pub async fn close(client: Client) -> E2eResult {
    client.close().await.map_err(browser("closing the browser"))
}

pub async fn wait_for(client: &Client, what: &str, expression: &str) -> E2eResult {
    wait_for_with(client, what, expression, &[]).await
}

/// [`wait_for`] `expression`, given `args` ([`js_with`]).
pub async fn wait_for_with(
    client: &Client,
    what: &str,
    expression: &str,
    args: &[serde_json::Value],
) -> E2eResult {
    for _ in 0..100 {
        if js_with(client, expression, args).await? == json!(true) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(E2eError::Timeout(what.into()))
}

/// Replace the saved settings and reload, as a returning player would
/// come back.
pub async fn reload_with(client: &Client, settings: serde_json::Value) -> E2eResult {
    js_with(
        client,
        "(localStorage.setItem(arguments[0], arguments[1]), true)",
        &[json!(STORAGE_KEY), json!(settings.to_string())],
    )
    .await?;
    reload(client).await
}

pub async fn reload(client: &Client) -> E2eResult {
    client.refresh().await.map_err(browser("reloading"))?;
    wait_for(
        client,
        "the reloaded page",
        "!document.body.classList.contains('loading')",
    )
    .await
}

pub async fn body_has(client: &Client, class: &str) -> E2eResult<bool> {
    js_bool_with(
        client,
        "document.body.classList.contains(arguments[0])",
        &[json!(class)],
    )
    .await
}

/// A finger on the element `css`: down at its centre, moved `dx` right and
/// `dy` down, lifted (a tap when it does not move).
pub async fn touch(client: &Client, css: &str, dx: f64, dy: f64) -> E2eResult {
    let element = client
        .find(fantoccini::Locator::Css(css))
        .await
        .map_err(browser(css))?;
    let finger = TouchActions::new("finger".into())
        .then(PointerAction::MoveToElement {
            element,
            duration: None,
            x: 0.0,
            y: 0.0,
        })
        .then(PointerAction::Down {
            button: MOUSE_BUTTON_LEFT,
        })
        .then(PointerAction::MoveBy {
            duration: Some(Duration::from_millis(40)),
            x: dx,
            y: dy,
        })
        .then(PointerAction::Up {
            button: MOUSE_BUTTON_LEFT,
        });
    client
        .perform_actions(finger)
        .await
        .map_err(browser(format!("touching {css}")))
}

pub async fn click(client: &Client, css: &str) -> E2eResult {
    client
        .find(fantoccini::Locator::Css(css))
        .await
        .map_err(browser(css))?
        .click()
        .await
        .map_err(browser(format!("clicking {css}")))
}

/// Wait until no transition or animation runs (finite ones: the page has
/// no endless one).
pub async fn settled(client: &Client) -> E2eResult {
    wait_for(
        client,
        "the page to settle",
        "document.getAnimations().every((a) => a.playState !== 'running')",
    )
    .await
}

pub async fn screenshot(client: &Client, out: &Path, name: &str) -> E2eResult {
    settled(client).await?;
    let png = client.screenshot().await.map_err(browser("a screenshot"))?;
    let path = out.join(format!("{name}.png"));
    std::fs::write(&path, png).map_err(|e| E2eError::Write(path, e))
}

/// The release build (`target/site`) open in headless Firefox at 1100×700,
/// loaded, in an English browser. Keep the `Driver` alive as long as the
/// client.
pub async fn open() -> E2eResult<(Client, Driver)> {
    open_with_languages(ENGLISH).await
}

/// `target/<name>`, made if missing: where a test leaves screenshots and
/// screens.
pub fn out_dir(name: &str) -> E2eResult<PathBuf> {
    let out = workspace().join("target").join(name);
    std::fs::create_dir_all(&out).map_err(|e| E2eError::Write(out.clone(), e))?;
    Ok(out)
}

/// [`open`], in a browser preferring `languages`.
pub async fn open_with_languages(languages: &str) -> E2eResult<(Client, Driver)> {
    let (client, driver, port) = launch(languages).await?;
    visit(&client, port).await?;
    Ok((client, driver))
}

/// Open the site served on `port` and wait until it shows.
pub async fn visit(client: &Client, port: u16) -> E2eResult {
    goto(client, &format!("http://127.0.0.1:{port}{BASE}")).await?;
    wait_for(
        client,
        "the page",
        "!document.body.classList.contains('loading')",
    )
    .await
}

/// The window resized to `width`×`height`.
pub async fn resize(client: &Client, width: u32, height: u32) -> E2eResult {
    client
        .set_window_size(width, height)
        .await
        .map_err(browser(format!("sizing the window to {width}×{height}")))
}

/// The release build served, and headless Firefox at 1100×700 preferring
/// `languages`, on a blank page; the port the site is served on.
pub async fn launch(languages: &str) -> E2eResult<(Client, Driver, u16)> {
    let dist = workspace().join("target/site");
    if !dist.join("index.html").exists() {
        return Err(E2eError::NoBuild(dist));
    }
    let port = serve(dist)?;
    let driver_port = free_port()?;
    let driver = Driver(
        Command::new("geckodriver")
            .args(["--port", &driver_port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(E2eError::Spawn)?,
    );
    let client = connect(driver_port, languages).await?;
    resize(&client, 1100, 700).await?;
    Ok((client, driver, port))
}

pub async fn goto(client: &Client, url: &str) -> E2eResult {
    client
        .goto(url)
        .await
        .map_err(browser(format!("opening {url}")))
}

/// Into the page's `n`th frame, where scripts then run.
pub async fn enter_frame(client: &Client, n: u16) -> E2eResult {
    client
        .enter_frame(n)
        .await
        .map_err(browser(format!("entering frame {n}")))
}

/// Back out of a frame to the page around it.
pub async fn leave_frame(client: &Client) -> E2eResult {
    client
        .enter_parent_frame()
        .await
        .map_err(browser("leaving the frame"))
}
