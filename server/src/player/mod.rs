//! The external player. `POST /api/play` resolves a stream and hands it to
//! `Player::play`; everything after that (spawning mpv, its IPC, progress,
//! 85% watched, 75% preload, the playlist advance) happens here.

mod aniskip;
mod ipc;
mod mpv;
mod policy;
pub mod prefs;
pub mod routes;
mod session;
mod upscale;

use std::sync::{Arc, Mutex};

use anicat_core::ffi::{AnicatEngine, FfiCatalog, StreamHandle, WatchProgress};
use serde::Serialize;

use crate::writer::Writer;
use prefs::{Prefs, PrefsPatch, PrefsStore};

/// Past this fraction a recorded position replays from the start. Same 85%
/// the watched mark uses; resuming at 23:40 of 24:00 put a rewatch into the
/// credits and straight into auto-next (`AppModel.resolveAndPlay`).
const REPLAY_FROM_START_FRACTION: f64 = 0.85;

/// Where a play of an episode starts, and its recorded duration: shared by
/// `POST /api/play` and the next and previous keys in mpv.
pub fn resume_point(recorded: Option<&WatchProgress>) -> (f64, Option<f64>) {
    match recorded.filter(|p| p.duration > 0) {
        Some(p) => {
            let fraction = p.stop_time as f64 / p.duration as f64;
            let start = if fraction < REPLAY_FROM_START_FRACTION { p.stop_time.max(0) as f64 } else { 0.0 };
            (start, Some(p.duration as f64))
        }
        None => (0.0, None),
    }
}

/// Only from a known duration. It tells the pre-buffer gate which part of
/// the file mpv's --start will read; without it the gate proves byte 0
/// healthy and hands off to a resume seek on an unprioritized piece that
/// stalls.
pub fn resume_fraction(start: f64, duration: Option<f64>) -> Option<f64> {
    match duration {
        Some(d) if start > 0.0 && d > 0.0 => Some(start / d),
        _ => None,
    }
}

/// What the route knew when it asked for a play, beyond the stream itself.
#[derive(Debug, Clone)]
pub struct PlayRequest {
    pub catalog: FfiCatalog,
    pub catalog_id: i64,
    pub episode: i64,
    /// For mpv's window title.
    pub title: Option<String>,
    pub prefer_dub: bool,
    /// Where to start, in seconds; 0 for the beginning.
    pub start_seconds: f64,
    /// The recorded duration, when there is one.
    #[allow(dead_code)] // mpv reports the real one as soon as the file loads
    pub duration_seconds: Option<f64>,
}

/// `GET /api/player`. The page codes against these exact field names.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub active: bool,
    pub catalog: Option<FfiCatalog>,
    pub catalog_id: Option<i64>,
    pub episode: Option<i64>,
    pub title: Option<String>,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub paused: Option<bool>,
    pub next_ready: bool,
}

pub struct Player {
    engine: Arc<AnicatEngine>,
    writer: Writer,
    /// Held across a whole play or stop: two plays racing would each find no
    /// mpv and start one, and both would claim the same pipe name.
    session: tokio::sync::Mutex<Option<session::Handle>>,
    /// Read by `GET /api/player` without touching the session, so a stalled
    /// mpv cannot hang the page's polling.
    snapshot: Arc<Mutex<Snapshot>>,
    prefs: Arc<PrefsStore>,
    /// The running session's command sender, outside `session`: that lock is
    /// held for a whole play request, resolve included, and a settings
    /// toggle must reach mpv without waiting on one.
    live_cmd: Mutex<Option<tokio::sync::mpsc::UnboundedSender<session::Cmd>>>,
    /// For AniSkip.
    http: reqwest::Client,
}

impl Player {
    pub fn new(engine: Arc<AnicatEngine>, writer: Writer, prefs: Arc<PrefsStore>, http: reqwest::Client) -> Self {
        Self {
            engine,
            writer,
            session: tokio::sync::Mutex::new(None),
            snapshot: Arc::default(),
            prefs,
            live_cmd: Mutex::new(None),
            http,
        }
    }

    pub fn prefs(&self) -> Prefs {
        self.prefs.get()
    }

    /// Persists a change from the page and hands it to a running mpv, which
    /// applies it at once. Not through the session lock: that is held for a
    /// whole play, and a toggle must not wait on a resolve.
    pub async fn set_prefs(&self, patch: PrefsPatch) -> Prefs {
        let store = self.prefs.clone();
        let updated = tokio::task::spawn_blocking(move || store.update(|p| patch.apply(p)))
            .await
            .unwrap_or_else(|_| self.prefs.get());
        let live = self.live_cmd.lock().ok().and_then(|c| c.clone());
        if let Some(cmd) = live {
            let _ = cmd.send(session::Cmd::Prefs);
        }
        updated
    }

    /// Plays a resolved stream: into the running mpv when there is one, in a
    /// new one otherwise. One mpv, one playing file, one pin.
    pub async fn play(&self, handle: &StreamHandle, request: &PlayRequest) -> Result<(), String> {
        let mut slot = self.session.lock().await;
        if let Some(live) = slot.as_ref().filter(|h| !h.join.is_finished()) {
            let (reply, answer) = tokio::sync::oneshot::channel();
            let sent = live.cmd.send(session::Cmd::Replace {
                url: handle.url.clone(),
                request: request.clone(),
                reply,
            });
            if sent.is_ok() {
                match answer.await {
                    Ok(result) => return result,
                    // The session ended between the check and the command:
                    // mpv was closed a moment ago. Start a new one below.
                    Err(_) => log::info!("[player] mpv exited while taking a new file, starting another"),
                }
            }
        }
        if let Some(old) = slot.take() {
            // Its finish (final write, then pause) has to land before the new
            // mpv starts reading, or the pause lands on the new stream.
            let _ = old.join.await;
        }
        log::info!(
            "[player] play {:?}:{} episode {} at {:.0}s",
            request.catalog,
            request.catalog_id,
            request.episode,
            request.start_seconds
        );
        let started = session::start(
            self.engine.clone(),
            self.writer.clone(),
            self.snapshot.clone(),
            self.prefs.clone(),
            self.http.clone(),
            handle.url.clone(),
            request.clone(),
        )
        .await?;
        if let Ok(mut c) = self.live_cmd.lock() {
            *c = Some(started.cmd.clone());
        }
        *slot = Some(started);
        Ok(())
    }

    /// Quits mpv, if one is running, and waits for its session to write the
    /// final position. `playback_stopped` then releases the playing-file pin
    /// and pauses the torrent session; without it librqbit keeps pulling the
    /// episode, and any preload, at full speed with nobody watching.
    pub async fn stop(&self) {
        log::info!("[player] stop");
        let mut slot = self.session.lock().await;
        if let Some(live) = slot.take() {
            let _ = live.cmd.send(session::Cmd::Quit);
            let _ = live.join.await;
        }
        self.engine.playback_stopped().await;
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_point_replays_past_85_percent() {
        let p = |stop_time, duration| WatchProgress { episode_number: 1, stop_time, duration };
        assert_eq!(resume_point(None), (0.0, None));
        assert_eq!(resume_point(Some(&p(600, 1440))), (600.0, Some(1440.0)));
        assert_eq!(resume_point(Some(&p(1300, 1440))), (0.0, Some(1440.0)));
        assert_eq!(resume_point(Some(&p(600, 0))), (0.0, None));
        assert_eq!(resume_fraction(720.0, Some(1440.0)), Some(0.5));
        assert_eq!(resume_fraction(0.0, Some(1440.0)), None);
    }
}
