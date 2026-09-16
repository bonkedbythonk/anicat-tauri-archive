//! One running mpv and everything that happens while it plays: the IO half
//! of the player loop. The decisions are `policy::Tracker`'s and
//! `upscale::decide`'s.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anicat_core::ffi::{AnicatEngine, FfiCatalog, StreamRequest};
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use super::aniskip::{self, Segment};
use super::ipc::{self, IpcClient};
use super::policy::{self, Tracker};
use super::prefs::{Prefs, PrefsStore};
use super::upscale::{self, Decision};
use super::{mpv, PlayRequest, Snapshot};
use crate::writer::Writer;

/// After `quit`, how long mpv gets before it is killed. A quit it never
/// processes (stuck in a network read on a stalled piece) would otherwise
/// leave the stop request, and the tray's Quit, hanging.
const QUIT_GRACE: Duration = Duration::from_secs(3);

/// What the keys and skin buttons in mpv send. mpv delivers every
/// `script-message` to every IPC client as a `client-message` event, so these
/// reach the server with no HTTP and no port inside the Lua (the Tauri script
/// curled a hardcoded port, and every callback went to a stranger whenever
/// something else held it). Each name has exactly one owner: `anicat.lua`
/// registers none of these, or a toggle would flip twice.
const MSG_TOGGLE_SHADERS: &str = "anicat-toggle-shaders";
const MSG_TOGGLE_AUTOSKIP: &str = "anicat-toggle-autoskip";
const MSG_TOGGLE_AUTO_NEXT: &str = "anicat-toggle-auto-next";
const MSG_NEXT: &str = "anicat-next-episode";
const MSG_PREVIOUS: &str = "anicat-previous-episode";
const MSG_RELOAD: &str = "anicat-reload-episode";
const MSG_TRANSLATION: &str = "anicat-toggle-translation";

pub enum Cmd {
    Replace {
        url: String,
        request: PlayRequest,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// `prefs.json` changed from the page: re-read it and apply it.
    Prefs,
    Quit,
}

pub struct Handle {
    pub cmd: mpsc::UnboundedSender<Cmd>,
    pub join: JoinHandle<()>,
}

enum Internal {
    Episodes {
        catalog: FfiCatalog,
        catalog_id: i64,
        list: Vec<(i64, bool)>,
        mal_id: Option<i64>,
    },
    Preloaded {
        generation: u64,
        episode: i64,
        url: String,
    },
    SkipTimes {
        generation: u64,
        url: String,
        segments: Vec<Segment>,
    },
    /// A next, previous, reload or sub/dub resolve finished.
    Switched {
        generation: u64,
        request: PlayRequest,
        result: Result<String, String>,
    },
}

/// Registry writes, in the order the ticks asked for them. Spawning one
/// task per write would not keep that order, and a later tick landing
/// before an earlier one moves the resume point backwards (see `writer.rs`).
enum Write {
    Progress(FfiCatalog, i64, i64, i64, i64),
    Completed(FfiCatalog, i64, i64),
}

struct Session {
    engine: Arc<AnicatEngine>,
    writer: Writer,
    snapshot: Arc<Mutex<Snapshot>>,
    ipc: IpcClient,
    tracker: Tracker,
    title: Option<String>,
    prefer_dub: bool,
    internal_tx: mpsc::UnboundedSender<Internal>,
    writes: mpsc::UnboundedSender<Write>,
    prefs: Arc<PrefsStore>,
    http: reqwest::Client,
    /// `<exe dir>/mpv/shaders`, when the bundled folder exists.
    shader_dir: Option<PathBuf>,
    /// The `glsl-shaders` value last sent. Writing the property rebuilds the
    /// render graph even when the value is unchanged, and a rebuild stalls
    /// audio for a moment (`set_shader_profile` in the Tauri main.lua).
    applied_shaders: String,
    /// The last decision whose off message was shown, so a binge of a TV
    /// show does not repeat "live action" at every episode.
    announced: Option<Decision>,
    /// The playing file's video height, once mpv knows it.
    height: Option<i64>,
    /// `(catalog_id, mal_id)` from the detail fetch.
    mal: Option<(i64, Option<i64>)>,
    /// The generation AniSkip was last asked about.
    skips_requested: Option<u64>,
    /// A next, previous, reload or sub/dub resolve is running. A second
    /// press would race it, and whichever answered last would win.
    switching: bool,
}

pub async fn start(
    engine: Arc<AnicatEngine>,
    writer: Writer,
    snapshot: Arc<Mutex<Snapshot>>,
    prefs: Arc<PrefsStore>,
    http: reqwest::Client,
    url: String,
    request: PlayRequest,
) -> Result<Handle, String> {
    let binary = mpv::locate().ok_or_else(|| mpv::NOT_FOUND.to_string())?;
    let data_dir = crate::config::data_dir();
    let endpoint = ipc::endpoint(&data_dir);
    remove_socket(&endpoint);
    let config_dir = mpv::bundled_config_dir();
    let user_config = mpv::user_config();
    let extra = mpv::extra_args();
    // mpv does not create it, and with no directory it caches nothing.
    let shader_cache = data_dir.join("mpv-shader-cache");
    let shader_cache = std::fs::create_dir_all(&shader_cache).ok().map(|_| shader_cache);
    let media_title = media_title(&request);
    let args = mpv::args(&mpv::Launch {
        url: &url,
        start_seconds: request.start_seconds,
        media_title: &media_title,
        ipc: &endpoint,
        config_dir: config_dir.as_deref(),
        user_config: user_config.as_deref(),
        shader_cache: shader_cache.as_deref(),
        extra: &extra,
    });
    log::info!("[player] spawning {} {:?}", binary.display(), args);
    let mut child = Command::new(&binary)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        // The server exiting any way but through `stop` must not leave a
        // player behind reading from a range server that is gone.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("could not start mpv at {}: {e}", binary.display()))?;
    #[cfg(windows)]
    bind_to_server_lifetime(&child);

    let (ipc, mut events) = match IpcClient::connect(&endpoint, || matches!(child.try_wait(), Ok(None))).await {
        Ok(pair) => pair,
        Err(e) => {
            let _ = child.kill().await;
            remove_socket(&endpoint);
            return Err(e);
        }
    };
    for (id, name) in [(1, "time-pos"), (2, "duration"), (3, "pause"), (4, "playlist-pos"), (5, "height")] {
        if let Err(e) = ipc.command(json!(["observe_property", id, name])).await {
            log::warn!("[player] observe_property {name} failed: {e}");
        }
    }

    let mut tracker = Tracker::new(request.catalog, request.catalog_id, request.episode, url);
    tracker.auto_next = prefs.get().auto_next;
    // A local stream can finish loading before the connect retry lands, and
    // its `file-loaded` went to nobody. `time-pos` only answers once a file
    // is loaded, so a success here stands in for the missed event; without
    // it the episode waited for a file that had already arrived and nothing
    // was ever recorded.
    let loaded_at_connect = ipc.command(json!(["get_property", "time-pos"])).await.is_ok();
    if loaded_at_connect {
        tracker.file_loaded(Instant::now());
    }

    let (internal_tx, mut internal_rx) = mpsc::unbounded_channel();
    let (writes_tx, writes_rx) = mpsc::unbounded_channel();
    let write_task = tokio::spawn(run_writes(writer.clone(), writes_rx));
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();

    let mut session = Session {
        engine,
        writer,
        snapshot,
        ipc,
        tracker,
        title: request.title.clone(),
        prefer_dub: request.prefer_dub,
        internal_tx,
        writes: writes_tx,
        prefs,
        http,
        shader_dir: config_dir.as_ref().map(|d| d.join("shaders")),
        applied_shaders: String::new(),
        announced: None,
        height: None,
        mal: None,
        skips_requested: None,
        switching: false,
    };
    session.fetch_episodes();
    session.push_state().await;
    session.reset_skip_times().await;
    if loaded_at_connect {
        session.on_file_loaded().await;
    }
    session.publish();

    let join = tokio::spawn(async move {
        let mut events_open = true;
        let mut cmds_open = true;
        let mut kill_at: Option<tokio::time::Instant> = None;
        loop {
            let kill_sleep = async {
                match kill_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                status = child.wait() => {
                    log::info!("[player] mpv exited: {status:?}");
                    break;
                }
                ev = events.recv(), if events_open => match ev {
                    Some(ev) => session.on_event(ev).await,
                    None => events_open = false,
                },
                cmd = cmd_rx.recv(), if cmds_open => match cmd {
                    Some(Cmd::Replace { url, request, reply }) => {
                        let _ = reply.send(session.replace(url, request).await);
                    }
                    Some(Cmd::Prefs) => session.apply_prefs().await,
                    Some(Cmd::Quit) | None => {
                        cmds_open = false;
                        let ipc = session.ipc.clone();
                        tokio::spawn(async move { let _ = ipc.command(json!(["quit"])).await; });
                        kill_at = Some(tokio::time::Instant::now() + QUIT_GRACE);
                    }
                },
                Some(msg) = internal_rx.recv() => session.on_internal(msg).await,
                _ = kill_sleep => {
                    log::warn!("[player] mpv ignored quit, killing it");
                    let _ = child.kill().await;
                    kill_at = None;
                }
            }
        }
        session.finish(write_task).await;
        remove_socket(&endpoint);
    });
    Ok(Handle { cmd: cmd_tx, join })
}

impl Session {
    async fn on_event(&mut self, ev: Value) {
        match ev.get("event").and_then(Value::as_str) {
            Some("file-loaded") => {
                self.tracker.file_loaded(Instant::now());
                self.on_file_loaded().await;
            }
            Some("client-message") => {
                let name = ev.get("args").and_then(|a| a.get(0)).and_then(Value::as_str).unwrap_or("");
                self.on_message(name).await;
            }
            Some("property-change") => {
                let data = ev.get("data");
                match ev.get("name").and_then(Value::as_str) {
                    Some("time-pos") => {
                        if let Some(t) = data.and_then(Value::as_f64) {
                            let actions = self.tracker.time_pos(t, Instant::now());
                            self.perform(actions);
                        }
                    }
                    Some("duration") => {
                        self.tracker.set_duration(data.and_then(Value::as_f64));
                        self.maybe_fetch_skips();
                    }
                    // While awaiting a new file this is still the outgoing
                    // file's; `on_file_loaded` asks for the new one.
                    Some("height") if !self.tracker.awaiting_new_file() => {
                        self.height = data.and_then(Value::as_i64).filter(|h| *h > 0);
                        self.apply_shaders().await;
                    }
                    Some("pause") => {
                        if let Some(p) = data.and_then(Value::as_bool) {
                            self.tracker.set_paused(p);
                        }
                    }
                    Some("playlist-pos") => {
                        let pos = data.and_then(Value::as_i64).unwrap_or(-1);
                        // Record where the outgoing episode ended before the
                        // tracker forgets it.
                        let outgoing = (self.tracker.episode(), self.tracker.final_record());
                        if let Some(entry) = self.tracker.playlist_pos(pos) {
                            if let (ep, Some((stop, dur))) = outgoing {
                                let _ = self.writes.send(Write::Progress(
                                    self.tracker.catalog,
                                    self.tracker.catalog_id,
                                    ep,
                                    stop,
                                    dur,
                                ));
                            }
                            log::info!("[player] mpv advanced to episode {}", entry.episode);
                            self.claim_pin(entry.episode);
                            self.height = None;
                            self.reset_skip_times().await;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        self.publish();
    }

    fn perform(&mut self, actions: policy::Actions) {
        let (catalog, id, episode) = (self.tracker.catalog, self.tracker.catalog_id, self.tracker.episode());
        if let Some((stop, dur)) = actions.record {
            let _ = self.writes.send(Write::Progress(catalog, id, episode, stop, dur));
        }
        if actions.mark_watched {
            log::info!("[player] {catalog:?}:{id} episode {episode} passed 85%, marking watched");
            let _ = self.writes.send(Write::Completed(catalog, id, episode));
            if catalog == FfiCatalog::Anilist {
                tokio::spawn(advance_anilist(self.engine.clone(), self.writer.clone(), id, episode));
            }
        }
        if actions.advance {
            log::info!("[player] episode {episode} is at its end, advancing mpv's playlist");
            let ipc = self.ipc.clone();
            tokio::spawn(async move {
                if let Err(e) = ipc.command(json!(["playlist-next", "force"])).await {
                    log::warn!("[player] playlist-next failed: {e}");
                }
            });
        }
        if let Some(next) = actions.preload {
            let req = StreamRequest {
                catalog,
                catalog_id: id,
                episode: next,
                title: self.title.clone(),
                prefer_dub: self.prefer_dub,
                chosen_name: None,
                resume_fraction: None,
                preload: true,
            };
            let (engine, tx, generation) = (self.engine.clone(), self.internal_tx.clone(), self.tracker.generation);
            log::info!("[player] preloading {catalog:?}:{id} episode {next}");
            tokio::spawn(async move {
                match engine.resolve_stream(req).await {
                    Ok(handle) => {
                        let _ = tx.send(Internal::Preloaded { generation, episode: next, url: handle.url });
                    }
                    // Costs nothing visible: mpv ends at this episode, and
                    // the next play resolves cold as it would have anyway.
                    Err(e) => log::warn!("[player] episode {next} not preloaded: {e}"),
                }
            });
        }
    }

    async fn on_internal(&mut self, msg: Internal) {
        match msg {
            Internal::Episodes { catalog, catalog_id, list, mal_id } => {
                if self.tracker.catalog == catalog && self.tracker.catalog_id == catalog_id {
                    self.tracker.set_episodes(list);
                    self.mal = Some((catalog_id, mal_id));
                    self.maybe_fetch_skips();
                }
            }
            Internal::Preloaded { generation, episode, url } => {
                if !self.tracker.accept_preload(generation, episode, url.clone()) {
                    log::info!("[player] preload of episode {episode} arrived for an episode no longer playing");
                    return;
                }
                let options = json!({ "force-media-title": self.title_for(episode) });
                match self.ipc.command(json!(["loadfile", url, "append", -1, options])).await {
                    Ok(_) => log::info!("[player] episode {episode} appended to mpv's playlist"),
                    Err(e) => {
                        log::warn!("[player] mpv refused to append episode {episode}: {e}");
                        self.tracker.retract_append(episode);
                    }
                }
            }
            Internal::SkipTimes { generation, url, segments } => {
                if generation != self.tracker.generation {
                    return;
                }
                log::info!("[aniskip] {} segment(s) for episode {}", segments.len(), self.tracker.episode());
                self.set_skip_times(&url, &segments).await;
            }
            Internal::Switched { generation, request, result } => {
                self.switching = false;
                if generation != self.tracker.generation {
                    log::info!("[player] switch to episode {} landed after the episode changed; dropped", request.episode);
                    return;
                }
                match result {
                    Ok(url) => {
                        if let Err(e) = self.replace(url, request).await {
                            self.osd(&e, 4.0);
                        }
                    }
                    Err(e) => {
                        log::warn!("[player] could not switch to episode {}: {e}", request.episode);
                        self.osd(&format!("Could not load episode {}: {e}", request.episode), 5.0);
                    }
                }
            }
        }
        self.publish();
    }

    async fn replace(&mut self, url: String, request: PlayRequest) -> Result<(), String> {
        let mut options = json!({ "force-media-title": media_title(&request) });
        if request.start_seconds > 0.0 {
            options["start"] = json!(format!("{:.0}", request.start_seconds));
        }
        self.ipc
            .command(json!(["loadfile", url.clone(), "replace", -1, options]))
            .await
            .map_err(|e| format!("mpv did not take the new file: {e}"))?;
        // The outgoing episode's position, before the tracker forgets it.
        if let Some((stop, dur)) = self.tracker.final_record() {
            let _ = self.writes.send(Write::Progress(
                self.tracker.catalog,
                self.tracker.catalog_id,
                self.tracker.episode(),
                stop,
                dur,
            ));
        }
        let title_changed = self
            .tracker
            .replace(request.catalog, request.catalog_id, request.episode, url);
        self.title = request.title;
        self.prefer_dub = request.prefer_dub;
        self.height = None;
        // A play from the page supersedes a key's switch still resolving.
        self.switching = false;
        if title_changed {
            self.mal = None;
            self.fetch_episodes();
        }
        self.push_state().await;
        self.reset_skip_times().await;
        self.publish();
        Ok(())
    }

    /// A playlist advance is a play nobody asked the engine for: the pin
    /// still names the previous episode, and the next preload into the same
    /// pack could evict the file mpv is now reading. A non-preload resolve of
    /// an episode already in the reuse cache moves the pin and costs a lookup.
    fn claim_pin(&self, episode: i64) {
        let req = StreamRequest {
            catalog: self.tracker.catalog,
            catalog_id: self.tracker.catalog_id,
            episode,
            title: self.title.clone(),
            prefer_dub: self.prefer_dub,
            chosen_name: None,
            resume_fraction: None,
            preload: false,
        };
        let engine = self.engine.clone();
        tokio::spawn(async move {
            if let Err(e) = engine.resolve_stream(req).await {
                log::warn!("[player] could not move the playing-file pin to episode {episode}: {e}");
            }
        });
    }

    fn fetch_episodes(&self) {
        let (catalog, catalog_id) = (self.tracker.catalog, self.tracker.catalog_id);
        if !matches!(catalog, FfiCatalog::Anilist | FfiCatalog::TmdbTv) {
            return;
        }
        let (engine, tx) = (self.engine.clone(), self.internal_tx.clone());
        tokio::spawn(async move {
            let detail = if catalog == FfiCatalog::Anilist {
                engine.media_detail(catalog_id, false).await
            } else {
                engine.cinema_detail(catalog, catalog_id).await
            };
            match detail {
                Ok(d) => {
                    let list = d.episodes.iter().map(|e| (e.number as i64, e.is_aired)).collect();
                    let _ = tx.send(Internal::Episodes { catalog, catalog_id, list, mal_id: d.mal_id });
                }
                Err(e) => log::warn!("[player] no episode list for {catalog:?}:{catalog_id}, no auto-next: {e}"),
            }
        });
    }

    async fn on_file_loaded(&mut self) {
        // A file the same height as the one before sends no property change,
        // so the new file's is asked for rather than waited on.
        self.height = self
            .ipc
            .command(json!(["get_property", "height"]))
            .await
            .ok()
            .and_then(|v| v.as_i64())
            .filter(|h| *h > 0);
        self.apply_shaders().await;
        self.maybe_fetch_skips();
    }

    async fn on_message(&mut self, name: &str) {
        match name {
            MSG_TOGGLE_SHADERS => {
                let p = self.update_prefs(|p| p.upscaling = !p.upscaling).await;
                self.osd(if p.upscaling { "Upscaling: Enabled" } else { "Upscaling: Disabled" }, 2.0);
                self.apply_prefs().await;
            }
            MSG_TOGGLE_AUTOSKIP => {
                let p = self.update_prefs(|p| p.autoskip = !p.autoskip).await;
                self.osd(if p.autoskip { "Auto-skip intro: On" } else { "Auto-skip intro: Off" }, 1.5);
                self.apply_prefs().await;
            }
            MSG_TOGGLE_AUTO_NEXT => {
                let p = self.update_prefs(|p| p.auto_next = !p.auto_next).await;
                self.osd(if p.auto_next { "Auto-play next: On" } else { "Auto-play next: Off" }, 1.5);
                self.apply_prefs().await;
            }
            MSG_NEXT => self.next_episode().await,
            MSG_PREVIOUS => match self.tracker.previous_episode() {
                Some(ep) => self.switch(ep, self.prefer_dub, None, &format!("Loading episode {ep}...")).await,
                None => self.osd("Already at the first episode.", 3.0),
            },
            MSG_RELOAD => {
                let at = self.tracker.position;
                self.switch(self.tracker.episode(), self.prefer_dub, at, "Reloading episode...").await;
            }
            MSG_TRANSLATION => {
                if self.tracker.catalog != FfiCatalog::Anilist {
                    self.osd("Sub and dub apply to anime only.", 2.0);
                    return;
                }
                let dub = !self.prefer_dub;
                let at = self.tracker.position;
                let text = if dub { "Switching to dub..." } else { "Switching to sub..." };
                self.switch(self.tracker.episode(), dub, at, text).await;
            }
            _ => {}
        }
    }

    async fn next_episode(&mut self) {
        if self.tracker.catalog == FfiCatalog::TmdbMovie {
            self.osd("Films have no next episode.", 2.0);
            return;
        }
        // Already appended by the preload: mpv has the file, nothing to resolve.
        if self.tracker.next_ready() {
            if let Err(e) = self.ipc.command(json!(["playlist-next", "force"])).await {
                log::warn!("[player] playlist-next failed: {e}");
            }
            return;
        }
        let next = match self.tracker.next_episode() {
            Some(n) => Some(n),
            // No list yet (a slow or failed detail fetch): the number after
            // this one, and a resolve that finds nothing says so.
            None if !self.tracker.has_episode_list() => Some(self.tracker.episode() + 1),
            None => None,
        };
        match next {
            Some(ep) => self.switch(ep, self.prefer_dub, None, &format!("Loading episode {ep}...")).await,
            None => self.osd("Already at the last episode.", 3.0),
        }
    }

    /// Resolves `episode` in the background and replaces the playing file
    /// with it when it lands. `position` is where to start for a reload or a
    /// sub/dub flip; `None` resumes from the registry, as a play from the page
    /// does.
    async fn switch(&mut self, episode: i64, prefer_dub: bool, position: Option<f64>, message: &str) {
        if self.switching {
            self.osd("Still loading. One moment.", 2.0);
            return;
        }
        self.switching = true;
        // Long, because a cold resolve can take most of a minute; the new
        // file's own OSD replaces it the moment it opens.
        self.osd(message, 30.0);
        let (catalog, catalog_id, generation) = (self.tracker.catalog, self.tracker.catalog_id, self.tracker.generation);
        let (engine, tx, title, known_duration) =
            (self.engine.clone(), self.internal_tx.clone(), self.title.clone(), self.tracker.duration);
        log::info!("[player] switching to {catalog:?}:{catalog_id} episode {episode} (dub {prefer_dub})");
        tokio::spawn(async move {
            let (start, duration) = match position {
                Some(p) => (p.max(0.0).floor(), known_duration),
                None => {
                    let e = engine.clone();
                    let recorded = tokio::task::spawn_blocking(move || e.get_progress(catalog, catalog_id, episode))
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .flatten();
                    super::resume_point(recorded.as_ref())
                }
            };
            let req = StreamRequest {
                catalog,
                catalog_id,
                episode,
                title: title.clone(),
                prefer_dub,
                chosen_name: None,
                resume_fraction: super::resume_fraction(start, duration),
                preload: false,
            };
            let result = engine.resolve_stream(req).await.map(|h| h.url).map_err(|e| e.to_string());
            let request = PlayRequest {
                catalog,
                catalog_id,
                episode,
                title,
                prefer_dub,
                start_seconds: start,
                duration_seconds: duration,
            };
            let _ = tx.send(Internal::Switched { generation, request, result });
        });
    }

    async fn update_prefs(&self, f: impl FnOnce(&mut Prefs) + Send + 'static) -> Prefs {
        let store = self.prefs.clone();
        match tokio::task::spawn_blocking(move || store.update(f)).await {
            Ok(p) => p,
            Err(_) => self.prefs.get(),
        }
    }

    /// Brings mpv and the tracker in line with `prefs.json`.
    async fn apply_prefs(&mut self) {
        let prefs = self.prefs.get();
        self.tracker.auto_next = prefs.auto_next;
        if !prefs.auto_next {
            // mpv advances into an entry already appended on its own, at eof.
            if let Some((index, episode)) = self.tracker.appended_index() {
                match self.ipc.command(json!(["playlist-remove", index])).await {
                    Ok(_) => self.tracker.retract_append(episode),
                    Err(e) => log::warn!("[player] could not remove appended episode {episode}: {e}"),
                }
            }
        }
        self.apply_shaders().await;
        self.push_state().await;
        self.publish();
    }

    async fn apply_shaders(&mut self) {
        let prefs = self.prefs.get();
        let decision = upscale::decide(self.tracker.catalog, prefs.upscaling, self.height);
        let wanted = match decision {
            Decision::Pending => return,
            Decision::On => match &self.shader_dir {
                Some(dir) => upscale::shader_list(dir, upscale::PATH_LIST_SEP),
                None => String::new(),
            },
            _ => String::new(),
        };
        if decision == Decision::On && wanted.is_empty() && self.announced != Some(decision) {
            log::warn!("[player] upscaling is on but no Anime4K shaders were found in the bundled mpv folder");
        }
        if wanted != self.applied_shaders {
            let cmd = if wanted.is_empty() {
                json!(["change-list", "glsl-shaders", "clr", ""])
            } else {
                json!(["change-list", "glsl-shaders", "set", wanted])
            };
            match self.ipc.command(cmd).await {
                Ok(_) => {
                    log::info!("[player] Anime4K {decision:?}");
                    self.applied_shaders = wanted;
                }
                Err(e) => log::warn!("[player] could not set glsl-shaders: {e}"),
            }
        }
        if let Some(text) = decision.message().filter(|_| self.announced != Some(decision)) {
            self.osd(text, 2.5);
        }
        self.announced = Some(decision);
        self.set_user_data("upscaling-active", json!(!self.applied_shaders.is_empty())).await;
    }

    /// The state the skin's Anicat buttons and `anicat.lua` read.
    async fn push_state(&self) {
        let p = self.prefs.get();
        self.set_user_data("upscaling", json!(p.upscaling)).await;
        self.set_user_data("autoskip", json!(p.autoskip)).await;
        self.set_user_data("auto-next", json!(p.auto_next)).await;
        self.set_user_data("dub", json!(self.prefer_dub)).await;
    }

    async fn set_user_data(&self, key: &str, value: Value) {
        let name = format!("user-data/anicat/{key}");
        if let Err(e) = self.ipc.command(json!(["set_property", name, value])).await {
            log::warn!("[player] could not set {name}: {e}");
        }
    }

    /// Tagged with the URL they belong to, and `anicat.lua` ignores times
    /// whose URL is not the file playing. mpv opens an appended episode
    /// before this server hears `playlist-pos` move, and in that window the
    /// previous episode's OP times would apply to the new file.
    async fn set_skip_times(&self, url: &str, segments: &[Segment]) {
        self.set_user_data("skip-times", json!({ "url": url, "segments": segments })).await;
    }

    async fn reset_skip_times(&self) {
        let url = self.tracker.url().to_string();
        self.set_skip_times(&url, &[]).await;
    }

    /// Once per episode, when the file is loaded, its duration is known and
    /// the detail fetch has named a MAL id.
    fn maybe_fetch_skips(&mut self) {
        let generation = self.tracker.generation;
        if self.tracker.catalog != FfiCatalog::Anilist
            || self.skips_requested == Some(generation)
            || self.tracker.awaiting_new_file()
        {
            return;
        }
        let Some(duration) = self.tracker.duration else { return };
        let mal_id = match self.mal {
            Some((id, mal)) if id == self.tracker.catalog_id => mal,
            _ => return,
        };
        self.skips_requested = Some(generation);
        let Some(mal_id) = mal_id else {
            log::info!("[aniskip] no MAL id for AniList {}; chapters only", self.tracker.catalog_id);
            return;
        };
        let (http, tx, episode, url) =
            (self.http.clone(), self.internal_tx.clone(), self.tracker.episode(), self.tracker.url().to_string());
        tokio::spawn(async move {
            match aniskip::fetch(&http, mal_id, episode, duration).await {
                Ok(segments) => {
                    let _ = tx.send(Internal::SkipTimes { generation, url, segments });
                }
                Err(e) => log::warn!("[aniskip] MAL {mal_id} episode {episode}: {e}"),
            }
        });
    }

    /// Not awaited: an OSD line is not worth holding the event loop for.
    fn osd(&self, text: &str, seconds: f64) {
        let ipc = self.ipc.clone();
        let cmd = json!(["show-text", text, (seconds * 1000.0) as i64]);
        tokio::spawn(async move {
            let _ = ipc.command(cmd).await;
        });
    }

    fn title_for(&self, episode: i64) -> String {
        title_text(self.title.as_deref(), self.tracker.catalog, episode)
    }

    fn publish(&self) {
        let t = &self.tracker;
        let snap = Snapshot {
            active: true,
            catalog: Some(t.catalog),
            catalog_id: Some(t.catalog_id),
            episode: Some(t.episode()),
            title: self.title.clone(),
            position: t.position,
            duration: t.duration,
            paused: t.paused,
            next_ready: t.next_ready(),
        };
        if let Ok(mut s) = self.snapshot.lock() {
            *s = snap;
        }
    }

    async fn finish(self, write_task: JoinHandle<()>) {
        if let Some((stop, dur)) = self.tracker.final_record() {
            let _ = self.writes.send(Write::Progress(
                self.tracker.catalog,
                self.tracker.catalog_id,
                self.tracker.episode(),
                stop,
                dur,
            ));
        }
        if let Ok(mut s) = self.snapshot.lock() {
            *s = Snapshot::default();
        }
        let engine = self.engine.clone();
        // Closing the queue lets the write task drain and end, so the final
        // position is in the registry before the session is paused.
        drop(self);
        let _ = write_task.await;
        log::info!("[player] playback stopped, pausing the torrent session");
        engine.playback_stopped().await;
    }
}

async fn run_writes(writer: Writer, mut rx: mpsc::UnboundedReceiver<Write>) {
    while let Some(w) = rx.recv().await {
        let result = match w {
            Write::Progress(c, id, ep, stop, dur) => writer.record_progress(c, id, ep, stop, dur).await,
            Write::Completed(c, id, ep) => writer.mark_episode_completed(c, id, ep).await,
        };
        if let Err(e) = result {
            log::warn!("[player] registry write failed: {}", e.message);
        }
    }
}

/// `AppModel.advanceAniListProgress` without a detail page: read the list
/// entry from AniList first. Re-sending a progress the list has already
/// passed drags it backwards on a rewatch. Attempted signed out too, as the
/// Mac does; it fails there and is only logged.
async fn advance_anilist(engine: Arc<AnicatEngine>, writer: Writer, catalog_id: i64, episode: i64) {
    let detail = match engine.media_detail(catalog_id, false).await {
        Ok(d) => d,
        Err(e) => {
            log::warn!("[player] AniList progress not advanced for {catalog_id}: {e}");
            return;
        }
    };
    let listed = i64::from(detail.list_progress.unwrap_or(0));
    if listed >= episode {
        return;
    }
    let count = detail.episode_count.or(detail.chapter_count).map(i64::from);
    let (progress, status) = policy::list_entry_update(episode, count, detail.list_status.as_deref());
    match writer.update_list_entry(catalog_id, status, None, Some(progress)).await {
        Ok(()) => log::info!("[player] AniList progress for {catalog_id} advanced to {progress}"),
        Err(e) => log::warn!("[player] AniList progress not advanced for {catalog_id}: {}", e.message),
    }
}

fn media_title(request: &PlayRequest) -> String {
    title_text(request.title.as_deref(), request.catalog, request.episode)
}

fn title_text(title: Option<&str>, catalog: FfiCatalog, episode: i64) -> String {
    let title = title.filter(|t| !t.trim().is_empty()).unwrap_or("Anicat");
    if catalog == FfiCatalog::TmdbMovie {
        title.to_string()
    } else {
        format!("{title} - Episode {episode}")
    }
}

#[cfg(unix)]
fn remove_socket(endpoint: &std::path::Path) {
    let _ = std::fs::remove_file(endpoint);
}

/// A named pipe goes away with its last handle; there is no file to remove.
#[cfg(windows)]
fn remove_socket(_endpoint: &std::path::Path) {}

/// Puts mpv in a job object that dies with this process. `kill_on_drop` only
/// runs when the server unwinds normally; Task Manager, a crash, or the
/// installer's `Stop-Process` end the server without dropping anything, and
/// mpv would stay open on a stream whose server is gone. The job handle is
/// deliberately leaked: closing it is what kills the children, and the OS
/// closes it when this process exits, however it exits.
#[cfg(windows)]
fn bind_to_server_lifetime(child: &tokio::process::Child) {
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    let Some(process) = child.raw_handle() else { return };
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            log::warn!("[player] could not create a job object; mpv may outlive a crashed server");
            return;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 || AssignProcessToJobObject(job, process as _) == 0 {
            log::warn!("[player] could not bind mpv to the server's lifetime; mpv may outlive a crashed server");
        }
    }
}
