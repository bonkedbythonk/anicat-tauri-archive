//! What a playback tick means, with no IO: the Mac's
//! `AppModel.handlePlaybackPositionChange` rules and the playlist
//! bookkeeping that keeps one episode's ticks off another. The session in
//! `session.rs` feeds it mpv's events and performs whatever it answers.

use std::time::{Duration, Instant};

use anicat_core::ffi::FfiCatalog;

/// `AppModel.watchedThresholdPct`. Same line `record_progress` uses for its
/// own `completed` flag (`db::stats::WATCHED_FRACTION`).
pub const WATCHED_THRESHOLD_PCT: f64 = 85.0;
/// `AppModel.nextEpisodePreloadPct`: far enough from the end that a cold
/// resolve (2 to 10 s, more on a slow swarm) lands before mpv reaches the
/// end of the file, late enough that the second download slot is not spent
/// on episodes nobody reaches.
pub const NEXT_EPISODE_PRELOAD_PCT: f64 = 75.0;
/// `AppModel.minimumCredibleDurationSeconds`. A stream mpv cannot read still
/// reports a sliver of a duration and an instant end; without this floor
/// every rule fired on it at once and a season was marked watched in seconds.
pub const MINIMUM_CREDIBLE_DURATION: f64 = 60.0;
/// `AppModel.minimumPlaybackBeforeCompletionSeconds`, for the same failure.
pub const MINIMUM_PLAYBACK_BEFORE_COMPLETION: Duration = Duration::from_secs(15);

/// `AppModel.autoAdvanceRemainingSeconds`. mpv's own playlist advance
/// needs `eof`, and a torrent stream does not reliably reach it: measured
/// 2026-09-12 against Frieren episode 1, a seek to 1555 of 1560 s left mpv
/// in `seeking=true` with a 0.1 s underrun for 95 s before mpv moved on
/// to the episode already appended. Within this distance of the end the
/// session sends `playlist-next` itself.
pub const AUTO_ADVANCE_REMAINING: f64 = 2.0;

/// One entry in mpv's playlist, in mpv's order.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub episode: i64,
    pub url: String,
}

/// What a tick asks the session to do.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Actions {
    /// `(stop_time, duration)` in whole seconds.
    pub record: Option<(i64, i64)>,
    pub mark_watched: bool,
    /// Resolve this episode with `preload: true` and append it.
    pub preload: Option<i64>,
    /// Send `playlist-next`: the episode is over and the next is appended.
    pub advance: bool,
}

/// The playing title's state across its episodes.
#[derive(Debug)]
pub struct Tracker {
    pub catalog: FfiCatalog,
    pub catalog_id: i64,
    playlist: Vec<Entry>,
    current: usize,
    /// Bumped on every episode change. Async results (a preload) carry the
    /// generation they were started under and are dropped when it moved on.
    pub generation: u64,
    /// The Mac's `awaitingNewFile`. Between an episode change and mpv's
    /// `file-loaded`, `time-pos` and `duration` still describe the outgoing
    /// file: its last tick (1438 of 1440) was handled as the new episode's,
    /// which marked it watched within a second.
    awaiting_new_file: bool,
    file_started_at: Option<Instant>,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    /// A `duration` reported while awaiting the new file. mpv does not
    /// promise its property change lands after the `file-loaded` event, and
    /// dropping one that came first left the episode with no duration and
    /// nothing recorded until the next change, which for a file is never.
    pending_duration: Option<f64>,
    pub paused: Option<bool>,
    last_recorded_second: i64,
    watched_fired: bool,
    preload_fired: bool,
    advance_fired: bool,
    /// `(number, is_aired)` in list order, once the detail fetch lands.
    episodes: Option<Vec<(i64, bool)>>,
    /// The `auto_next` setting. Off means no preload either: a preload
    /// spends the second download slot on an episode nobody will reach.
    pub auto_next: bool,
}

impl Tracker {
    pub fn new(catalog: FfiCatalog, catalog_id: i64, episode: i64, url: String) -> Self {
        Self {
            catalog,
            catalog_id,
            playlist: vec![Entry { episode, url }],
            current: 0,
            generation: 0,
            awaiting_new_file: true,
            file_started_at: None,
            position: None,
            duration: None,
            pending_duration: None,
            paused: None,
            last_recorded_second: -1,
            watched_fired: false,
            preload_fired: false,
            advance_fired: false,
            episodes: None,
            auto_next: true,
        }
    }

    pub fn episode(&self) -> i64 {
        self.playlist[self.current].episode
    }

    /// The URL of the entry mpv is playing.
    pub fn url(&self) -> &str {
        &self.playlist[self.current].url
    }

    /// Whether the entry after the current one is already in mpv's playlist.
    pub fn next_ready(&self) -> bool {
        self.current + 1 < self.playlist.len()
    }

    pub fn awaiting_new_file(&self) -> bool {
        self.awaiting_new_file
    }

    /// `loadfile replace`: mpv clears its playlist, so ours starts over.
    /// Returns whether the title changed, in which case the episode list
    /// has to be fetched again.
    pub fn replace(&mut self, catalog: FfiCatalog, catalog_id: i64, episode: i64, url: String) -> bool {
        let same_title = self.catalog == catalog && self.catalog_id == catalog_id;
        self.catalog = catalog;
        self.catalog_id = catalog_id;
        if !same_title {
            self.episodes = None;
        }
        self.playlist = vec![Entry { episode, url }];
        self.current = 0;
        self.begin_episode();
        !same_title
    }

    fn begin_episode(&mut self) {
        self.generation += 1;
        self.awaiting_new_file = true;
        self.file_started_at = None;
        self.position = None;
        self.duration = None;
        self.pending_duration = None;
        self.last_recorded_second = -1;
        self.watched_fired = false;
        self.preload_fired = false;
        self.advance_fired = false;
    }

    pub fn has_episode_list(&self) -> bool {
        self.episodes.is_some()
    }

    pub fn set_episodes(&mut self, episodes: Vec<(i64, bool)>) {
        self.episodes = Some(episodes);
    }

    /// The episode after `current` in list order, when it has aired. An
    /// unaired one is refused: auto-next walked into episode 11 of a
    /// ten-episodes-so-far show, the resolve could only fail.
    pub fn next_episode(&self) -> Option<i64> {
        if self.catalog == FfiCatalog::TmdbMovie {
            return None;
        }
        let list = self.episodes.as_ref()?;
        let at = list.iter().position(|(n, _)| *n == self.episode())?;
        list.get(at + 1).filter(|(_, aired)| *aired).map(|(n, _)| *n)
    }

    /// The episode before `current` in list order, for `anicat-previous-episode`.
    /// Before the list arrives, the number below it.
    pub fn previous_episode(&self) -> Option<i64> {
        if self.catalog == FfiCatalog::TmdbMovie {
            return None;
        }
        match self.episodes.as_ref() {
            Some(list) => {
                let at = list.iter().position(|(n, _)| *n == self.episode())?;
                at.checked_sub(1).map(|i| list[i].0)
            }
            None => Some(self.episode() - 1).filter(|n| *n >= 1),
        }
    }

    /// The index of the appended next entry in mpv's playlist, for removing
    /// it when auto-next is turned off after the preload landed.
    pub fn appended_index(&self) -> Option<(usize, i64)> {
        self.next_ready().then(|| (self.current + 1, self.playlist[self.current + 1].episode))
    }

    /// A preload finished. Appends it when it still belongs to the episode
    /// it was started for; returns whether the session should send the
    /// `loadfile append`.
    pub fn accept_preload(&mut self, generation: u64, episode: i64, url: String) -> bool {
        if generation != self.generation || !self.auto_next || self.next_ready() || self.next_episode() != Some(episode) {
            return false;
        }
        self.playlist.push(Entry { episode, url });
        true
    }

    /// Undoes `accept_preload` when mpv refused the append.
    pub fn retract_append(&mut self, episode: i64) {
        if self.next_ready() && self.playlist.last().map(|e| e.episode) == Some(episode) {
            self.playlist.pop();
        }
    }

    /// mpv's `playlist-pos`. Returns the entry that became current when the
    /// position moved to a different entry.
    pub fn playlist_pos(&mut self, pos: i64) -> Option<Entry> {
        let pos = usize::try_from(pos).ok()?;
        if pos == self.current || pos >= self.playlist.len() {
            return None;
        }
        self.current = pos;
        self.begin_episode();
        Some(self.playlist[pos].clone())
    }

    /// Idempotent: the session also calls it when it finds a file already
    /// loaded at connect, and a second call would restart the 15 s clock and
    /// throw away the duration that arrived in between.
    pub fn file_loaded(&mut self, now: Instant) {
        if !self.awaiting_new_file {
            return;
        }
        self.awaiting_new_file = false;
        self.file_started_at = Some(now);
        self.duration = self.pending_duration.take();
    }

    pub fn set_duration(&mut self, duration: Option<f64>) {
        let duration = duration.filter(|d| *d > 0.0);
        if self.awaiting_new_file {
            self.pending_duration = duration;
        } else {
            self.duration = duration;
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = Some(paused);
    }

    /// The last position worth writing, for the final write on exit.
    pub fn final_record(&self) -> Option<(i64, i64)> {
        if self.awaiting_new_file {
            return None;
        }
        let (pos, dur) = (self.position?, self.duration?);
        let dur = dur as i64;
        Some(((pos as i64).min(dur).max(0), dur))
    }

    /// mpv's `time-pos`.
    pub fn time_pos(&mut self, time: f64, now: Instant) -> Actions {
        let mut actions = Actions::default();
        if self.awaiting_new_file || !time.is_finite() || time < 0.0 {
            return actions;
        }
        self.position = Some(time);
        let Some(duration) = self.duration else { return actions };
        let dur = duration as i64;
        let stop = (time as i64).min(dur);

        let played_for = self
            .file_started_at
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or_default();
        let completion_rules_apply =
            duration >= MINIMUM_CREDIBLE_DURATION && played_for >= MINIMUM_PLAYBACK_BEFORE_COMPLETION;
        let percent = stop as f64 / dur as f64 * 100.0;

        if completion_rules_apply && !self.watched_fired && percent >= WATCHED_THRESHOLD_PCT {
            self.watched_fired = true;
            actions.mark_watched = true;
        }
        if completion_rules_apply && self.auto_next && !self.preload_fired && percent >= NEXT_EPISODE_PRELOAD_PCT && !self.next_ready()
        {
            // Not consumed while the episode list has not arrived: the flag
            // would otherwise be spent on a tick that had nothing to preload.
            if let Some(next) = self.next_episode() {
                self.preload_fired = true;
                actions.preload = Some(next);
            }
        }
        if completion_rules_apply && self.auto_next && !self.advance_fired && self.next_ready() && duration - time <= AUTO_ADVANCE_REMAINING {
            self.advance_fired = true;
            actions.advance = true;
        }
        // Once per whole second, the Mac's `lastRecordedSecond` gate. mpv
        // reports `time-pos` about once a frame.
        if stop != self.last_recorded_second {
            self.last_recorded_second = stop;
            actions.record = Some((stop, dur));
        }
        actions
    }
}

/// `AppModel.listEntryUpdate` for a watched episode: the progress and
/// status AniList should hold. Clamped to the episode count with
/// `COMPLETED`, or `CURRENT` for an entry that was unset or `PLANNING`.
pub fn list_entry_update(episode: i64, episode_count: Option<i64>, list_status: Option<&str>) -> (i64, Option<String>) {
    let mut progress = episode;
    let mut status = None;
    match episode_count {
        Some(total) if total > 0 && progress >= total => {
            progress = total;
            status = Some("COMPLETED".to_string());
        }
        _ if progress > 0 && matches!(list_status, None | Some("PLANNING")) => {
            status = Some("CURRENT".to_string());
        }
        _ => {}
    }
    (progress, status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> Tracker {
        let mut t = Tracker::new(FfiCatalog::Anilist, 154587, 1, "u1".into());
        t.set_episodes((1..=28).map(|n| (n, true)).collect());
        t
    }

    /// Loaded 20 s ago with a 1440 s duration, so completion rules apply.
    fn loaded(t: &mut Tracker) -> Instant {
        let start = Instant::now();
        t.file_loaded(start);
        t.set_duration(Some(1440.0));
        start + Duration::from_secs(20)
    }

    #[test]
    fn records_once_per_whole_second() {
        let mut t = tracker();
        let now = loaded(&mut t);
        assert_eq!(t.time_pos(10.1, now).record, Some((10, 1440)));
        assert_eq!(t.time_pos(10.6, now).record, None);
        assert_eq!(t.time_pos(11.0, now).record, Some((11, 1440)));
    }

    #[test]
    fn nothing_before_file_loaded() {
        let mut t = tracker();
        t.set_duration(Some(1440.0));
        assert_eq!(t.time_pos(1300.0, Instant::now()), Actions::default());
        assert_eq!(t.duration, None);
    }

    #[test]
    fn watched_at_85_once() {
        let mut t = tracker();
        let now = loaded(&mut t);
        assert!(!t.time_pos(1223.0, now).mark_watched); // 84.9%
        assert!(t.time_pos(1224.0, now).mark_watched); // 85%
        assert!(!t.time_pos(1300.0, now).mark_watched);
    }

    #[test]
    fn preload_at_75_once_for_the_next_episode() {
        let mut t = tracker();
        let now = loaded(&mut t);
        assert_eq!(t.time_pos(1079.0, now).preload, None);
        assert_eq!(t.time_pos(1080.0, now).preload, Some(2));
        assert_eq!(t.time_pos(1081.0, now).preload, None);
    }

    #[test]
    fn completion_rules_wait_for_fifteen_seconds_and_a_credible_duration() {
        let mut t = tracker();
        let start = Instant::now();
        t.file_loaded(start);
        t.set_duration(Some(1440.0));
        let early = t.time_pos(1300.0, start + Duration::from_secs(5));
        assert!(!early.mark_watched && early.preload.is_none());
        let later = t.time_pos(1301.0, start + Duration::from_secs(16));
        assert!(later.mark_watched && later.preload == Some(2));

        let mut short = tracker();
        short.file_loaded(start);
        short.set_duration(Some(30.0));
        assert!(!short.time_pos(29.0, start + Duration::from_secs(60)).mark_watched);
    }

    #[test]
    fn preload_waits_for_the_episode_list_without_spending_its_flag() {
        let mut t = Tracker::new(FfiCatalog::Anilist, 1, 1, "u1".into());
        let now = loaded(&mut t);
        assert_eq!(t.time_pos(1100.0, now).preload, None);
        t.set_episodes(vec![(1, true), (2, true)]);
        assert_eq!(t.time_pos(1101.0, now).preload, Some(2));
    }

    #[test]
    fn no_preload_past_the_last_aired_episode_or_for_a_film() {
        let mut t = Tracker::new(FfiCatalog::Anilist, 1, 10, "u".into());
        t.set_episodes(vec![(9, true), (10, true), (11, false)]);
        let now = loaded(&mut t);
        assert_eq!(t.time_pos(1100.0, now).preload, None);

        let mut film = Tracker::new(FfiCatalog::TmdbMovie, 1, 1, "u".into());
        film.set_episodes(vec![(1, true), (2, true)]);
        let now = loaded(&mut film);
        assert_eq!(film.time_pos(1100.0, now).preload, None);
    }

    #[test]
    fn playlist_advance_switches_episode_and_gates_the_outgoing_ticks() {
        let mut t = tracker();
        let now = loaded(&mut t);
        t.time_pos(1080.0, now);
        let generation = t.generation;
        assert!(t.accept_preload(generation, 2, "u2".into()));
        assert!(t.next_ready());

        let switched = t.playlist_pos(1).expect("moved to entry 1");
        assert_eq!(switched.episode, 2);
        assert_eq!(t.episode(), 2);
        assert!(!t.next_ready());
        assert_ne!(t.generation, generation);

        // The outgoing file's last tick, delivered after the switch.
        assert_eq!(t.time_pos(1438.0, now), Actions::default());
        t.set_duration(Some(1440.0));
        assert_eq!(t.duration, None);
        assert_eq!(t.final_record(), None);

        t.file_loaded(now);
        t.set_duration(Some(1420.0));
        assert_eq!(t.time_pos(3.0, now).record, Some((3, 1420)));
        // Flags are per episode: episode 2 gets its own watched mark.
        let later = now + Duration::from_secs(30);
        assert!(t.time_pos(1300.0, later).mark_watched);
    }

    #[test]
    fn advance_near_the_end_only_with_the_next_entry_appended() {
        let mut t = tracker();
        let now = loaded(&mut t);
        assert!(!t.time_pos(1439.0, now).advance);
        let generation = t.generation;
        assert!(t.accept_preload(generation, 2, "u2".into()));
        assert!(!t.time_pos(1437.9, now).advance);
        assert!(t.time_pos(1438.0, now).advance);
        assert!(!t.time_pos(1439.0, now).advance);
    }

    #[test]
    fn auto_next_off_neither_preloads_nor_advances() {
        let mut t = tracker();
        t.auto_next = false;
        let now = loaded(&mut t);
        assert_eq!(t.time_pos(1100.0, now).preload, None);
        assert!(!t.accept_preload(t.generation, 2, "u2".into()));
        t.auto_next = true;
        assert_eq!(t.time_pos(1101.0, now).preload, Some(2));
        assert!(t.accept_preload(t.generation, 2, "u2".into()));
        assert_eq!(t.appended_index(), Some((1, 2)));
        t.auto_next = false;
        assert!(!t.time_pos(1439.0, now).advance);
    }

    #[test]
    fn previous_episode_follows_the_list_or_counts_down() {
        let mut t = Tracker::new(FfiCatalog::Anilist, 1, 3, "u".into());
        assert_eq!(t.previous_episode(), Some(2));
        t.set_episodes(vec![(0, true), (3, true)]);
        assert_eq!(t.previous_episode(), Some(0));
        let first = Tracker::new(FfiCatalog::Anilist, 1, 1, "u".into());
        assert_eq!(first.previous_episode(), None);
        assert_eq!(Tracker::new(FfiCatalog::TmdbMovie, 1, 1, "u".into()).previous_episode(), None);
    }

    #[test]
    fn playlist_pos_that_does_not_move_is_ignored() {
        let mut t = tracker();
        assert_eq!(t.playlist_pos(0), None);
        assert_eq!(t.playlist_pos(-1), None);
        assert_eq!(t.playlist_pos(5), None);
    }

    #[test]
    fn stale_preload_is_dropped() {
        let mut t = tracker();
        let old = t.generation;
        t.replace(FfiCatalog::Anilist, 154587, 5, "u5".into());
        assert!(!t.accept_preload(old, 2, "u2".into()));
        // Wrong episode for the current generation.
        assert!(!t.accept_preload(t.generation, 2, "u2".into()));
        assert!(t.accept_preload(t.generation, 6, "u6".into()));
        // Only once.
        assert!(!t.accept_preload(t.generation, 6, "u6".into()));
        t.retract_append(6);
        assert!(!t.next_ready());
    }

    #[test]
    fn replace_resets_the_episode_list_only_for_another_title() {
        let mut t = tracker();
        assert!(!t.replace(FfiCatalog::Anilist, 154587, 3, "u3".into()));
        assert_eq!(t.next_episode(), Some(4));
        assert!(t.replace(FfiCatalog::TmdbTv, 1399, 1, "x".into()));
        assert_eq!(t.next_episode(), None);
        assert!(t.awaiting_new_file());
    }

    #[test]
    fn list_entry_update_branches() {
        assert_eq!(list_entry_update(28, Some(28), Some("CURRENT")), (28, Some("COMPLETED".into())));
        assert_eq!(list_entry_update(30, Some(28), None), (28, Some("COMPLETED".into())));
        assert_eq!(list_entry_update(3, Some(28), None), (3, Some("CURRENT".into())));
        assert_eq!(list_entry_update(3, None, Some("PLANNING")), (3, Some("CURRENT".into())));
        assert_eq!(list_entry_update(3, Some(28), Some("PAUSED")), (3, None));
        assert_eq!(list_entry_update(3, Some(0), Some("CURRENT")), (3, None));
    }
}
