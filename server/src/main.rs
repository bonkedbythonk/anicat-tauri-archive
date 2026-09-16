//! Anicat for Windows (and, for development, macOS): the Rust engine behind
//! a local HTTP API and one HTML page. See docs/WINDOWS_PLAN.md.

// No console window in a release build. The console is the process's
// lifetime on Windows: closing it, which a public user does by reflex to
// something that looks like leftover noise, kills the engine and the stream
// mpv is reading mid-episode. Debug builds keep it for the log output.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;
mod error;
mod logging;
mod player;
mod routes;
mod state;
#[cfg(any(windows, target_os = "macos"))]
mod tray;
mod version;
mod writer;

use std::future::{Future, IntoFuture};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anicat_core::ffi::AnicatEngine;

use crate::player::Player;
use crate::state::AppState;
use crate::writer::Writer;

/// Fixed so a bookmark keeps working across launches.
const PREFERRED_PORT: u16 = 47111;
/// Successors tried when the preferred port is taken.
const PORT_FALLBACKS: u16 = 10;

/// Longest the single-instance probe waits on a port. `/api/health` answers
/// without touching the network, so this is a loopback round trip; a port
/// that accepts and never answers is something else's and must not stall
/// the launch.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// Cap on `player.stop()` during shutdown. It waits on mpv to exit, and a
/// wedged mpv otherwise holds Quit forever with the icon already gone.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// Cap on draining open HTTP requests after the stop. A `/api/play` still
/// resolving can run for tens of seconds, and nobody is waiting for it.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(3);

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    // Before the log is rotated and before the engine exists. Rotation
    // renames the running instance's `anicat.log` out from under it, and
    // `AnicatEngine::new` opens the same registry and torrent cache a second
    // time: two engines evict and pin files under each other. A second launch
    // is almost always someone who lost the tab, so hand them the page.
    if let Some(port) = runtime.block_on(running_instance()) {
        eprintln!("Anicat is already running on http://127.0.0.1:{port}/");
        if browser_wanted() {
            open_browser(port);
        }
        return;
    }

    let data_dir = config::data_dir();
    // First, before the engine: `AnicatEngine::new` installs env_logger, and
    // nothing said before the redirect reaches the file.
    let log_redirected = logging::start(&data_dir);

    let (state, listener) = runtime.block_on(async {
        let token = config::load_token(&data_dir);
        let proxy = config::tmdb_proxy();
        let engine = match AnicatEngine::new(
            data_dir.to_string_lossy().into_owned(),
            token.clone(),
            None,
            proxy.clone(),
        ) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("engine failed to start: {e}");
                std::process::exit(1);
            }
        };
        log::info!(
            "Anicat server {} on {} {}, data at {}, signed in: {}, TMDB proxy: {}",
            version::current(),
            std::env::consts::OS,
            std::env::consts::ARCH,
            data_dir.display(),
            token.is_some(),
            proxy.is_some(),
        );

        // Off the path to the first request: DHT bootstrap is seconds, and the
        // page should load while it runs. A first play otherwise pays for it.
        {
            let engine = engine.clone();
            tokio::spawn(async move { engine.warm_up().await });
        }

        let (listener, port) = match bind().await {
            Some(pair) => pair,
            None => {
                log::error!(
                    "no free port in 127.0.0.1:{}..={}",
                    PREFERRED_PORT,
                    PREFERRED_PORT + PORT_FALLBACKS
                );
                std::process::exit(1);
            }
        };

        let writer = Writer::spawn(engine.clone());
        let http = reqwest::Client::builder()
            // GitHub's API refuses requests with no User-Agent.
            .user_agent(format!("Anicat-Server/{} (+https://github.com/bonkedbythonk/anicat)", version::current()))
            .build()
            .expect("reqwest client");
        let prefs = Arc::new(player::prefs::PrefsStore::load(&data_dir));
        let player = Arc::new(Player::new(engine.clone(), writer.clone(), prefs, http.clone()));
        let state = AppState {
            engine,
            writer,
            player,
            data_dir: data_dir.clone(),
            port,
            log_redirected,
            token: Arc::new(Mutex::new(token)),
            http,
        };
        (state, listener)
    });

    let port = state.port;
    log::info!("listening on http://127.0.0.1:{port}/");
    // The listener is bound, so the page the browser asks for is answered
    // rather than refused.
    if browser_wanted() {
        open_browser(port);
    }

    #[cfg(any(windows, target_os = "macos"))]
    if std::env::var_os("ANICAT_NO_TRAY").is_none_or(|v| v != "1") && tray::available() {
        let tray = tray::Tray::new();
        let stopped = tray.stopped_notifier();
        let (quit_tx, quit_rx) = tokio::sync::oneshot::channel::<()>();
        std::thread::Builder::new()
            .name("anicat-server".into())
            .spawn(move || {
                runtime.block_on(serve(state, listener, async move {
                    tokio::select! {
                        _ = quit_rx => {}
                        _ = terminate() => {}
                    }
                }));
                stopped.notify();
            })
            .expect("server thread");
        tray.run(port, move || {
            let _ = quit_tx.send(());
        });
    }

    // Headless: `ANICAT_NO_TRAY=1`, a session with no display, or a platform
    // without a tray. Ctrl-C is the only way out.
    runtime.block_on(serve(state, listener, terminate()));
}

/// Ctrl-C, and on unix SIGTERM too. A SIGTERM used to end the process without
/// running `serve`'s shutdown, and `kill_on_drop` never fires when nothing is
/// dropped: mpv stayed open reading from a range server that was gone.
async fn terminate() {
    #[cfg(unix)]
    {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Serves until `stop` resolves, then stops the player and drains. The one
/// shutdown path for the tray's Quit and for Ctrl-C.
async fn serve(state: AppState, listener: tokio::net::TcpListener, stop: impl Future<Output = ()> + Send + 'static) {
    let player = state.player.clone();
    let stopped = Arc::new(tokio::sync::Notify::new());
    let stopped_signal = stopped.clone();
    let app = routes::router(state.clone()).merge(player::routes::router().with_state(state));
    let shutdown = async move {
        stop.await;
        log::info!("shutting down");
        // Ends mpv and pauses the torrent session; without it librqbit keeps
        // pulling the episode for however long the process takes to exit.
        if tokio::time::timeout(STOP_TIMEOUT, player.stop()).await.is_err() {
            log::warn!("player did not stop within {STOP_TIMEOUT:?}");
        }
        stopped_signal.notify_one();
    };
    let server = axum::serve(listener, app).with_graceful_shutdown(shutdown).into_future();
    tokio::select! {
        result = server => {
            if let Err(e) = result {
                log::error!("server stopped: {e}");
            }
        }
        _ = async {
            stopped.notified().await;
            tokio::time::sleep(DRAIN_TIMEOUT).await;
        } => {
            log::warn!("requests still open {DRAIN_TIMEOUT:?} after shutdown; dropping them");
        }
    }
}

/// `ANICAT_NO_BROWSER=1` is for tests and for development sessions, where a
/// tab per launch piles up in the developer's browser.
fn browser_wanted() -> bool {
    std::env::var_os("ANICAT_NO_BROWSER").is_none_or(|v| v != "1")
}

/// Opens the page in the default browser. Detached, so a launcher that waits
/// for the browser it started cannot block the tray loop or startup.
pub fn open_browser(port: u16) {
    let url = format!("http://127.0.0.1:{port}/");
    if let Err(e) = open::that_detached(&url) {
        log::warn!("could not open {url}: {e}");
    }
}

/// The port of an Anicat server already running in the fallback range, if
/// any. Every port is asked at once: a free port refuses immediately, and a
/// foreign one that hangs costs `PROBE_TIMEOUT` once rather than per port.
/// The answer must have our `/api/health` shape; a port that merely
/// accepts belongs to something else, and the scan in `bind` steps over it.
async fn running_instance() -> Option<u16> {
    let client = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .no_proxy()
        .build()
        .ok()?;
    let probes = (PREFERRED_PORT..=PREFERRED_PORT + PORT_FALLBACKS).map(|port| {
        let client = client.clone();
        async move {
            let url = format!("http://127.0.0.1:{port}/api/health");
            let body: serde_json::Value = client.get(url).send().await.ok()?.json().await.ok()?;
            is_our_health(&body).then_some(port)
        }
    });
    let found = futures_join_all(probes).await;
    // Lowest port wins: it is the one a bookmark points at.
    found.into_iter().flatten().min()
}

fn is_our_health(body: &serde_json::Value) -> bool {
    body.get("app").and_then(|v| v.as_str()) == Some("anicat")
}

/// A join over the probes without pulling in `futures` for one call.
async fn futures_join_all<F, T>(futures: impl Iterator<Item = F>) -> Vec<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handles: Vec<_> = futures.map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(v) = h.await {
            out.push(v);
        }
    }
    out
}

/// Loopback only. Anything wider publishes the viewer's watch history and
/// download cache to the LAN, which is why the range server binds the same.
async fn bind() -> Option<(tokio::net::TcpListener, u16)> {
    for port in PREFERRED_PORT..=PREFERRED_PORT + PORT_FALLBACKS {
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => return Some((l, port)),
            Err(e) => log::warn!("127.0.0.1:{port} unavailable: {e}"),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::is_our_health;
    use serde_json::json;

    #[test]
    fn health_shape_is_ours_only() {
        assert!(is_our_health(&json!({ "app": "anicat", "version": "6.0.1" })));
        assert!(!is_our_health(&json!({ "version": "1.0" })));
        assert!(!is_our_health(&json!({ "app": "something-else" })));
    }
}
