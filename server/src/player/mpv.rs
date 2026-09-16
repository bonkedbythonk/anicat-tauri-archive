//! Finding mpv and the command line it is started with.

use std::path::{Path, PathBuf};

#[cfg(windows)]
const MPV_EXE: &str = "mpv.exe";
#[cfg(not(windows))]
const MPV_EXE: &str = "mpv";

/// What the page shows when there is no mpv. The zip ships one beside the
/// exe, so reaching this means a manual install lost it.
pub const NOT_FOUND: &str = "mpv was not found. Put mpv beside the Anicat executable, \
     set ANICAT_MPV to its path, or install it so it is on PATH.";

/// Beside the running executable, then `ANICAT_MPV`, then PATH. The bundled
/// copy wins so a different mpv on PATH (an old one, a portable build with
/// its own scripts) is not what a release plays through.
pub fn locate() -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(MPV_EXE)))
        .filter(|p| p.is_file());
    if beside.is_some() {
        return beside;
    }
    if let Some(p) = std::env::var_os("ANICAT_MPV").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
        log::warn!("[player] ANICAT_MPV={} is not a file", p.display());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(MPV_EXE))
        .find(|p| p.is_file())
}

/// The `mpv` folder shipped beside the executable: `mpv.conf`, `input.conf`,
/// `scripts/`, `script-opts/`, `fonts/` and `shaders/`. mpv resolves `~~/`
/// to `--config-dir`, and loads `~~/scripts`, `~~/script-opts`, `~~/fonts`
/// and `~~/input.conf` from it, so pointing it here is what loads the skin.
/// `ANICAT_MPV_CONFIG_DIR` overrides it: a `cargo run` binary sits in
/// `target/debug/` with no `mpv` folder beside it, so without the override
/// every development run is unskinned and nothing about the skin can be
/// tested before packaging.
pub fn bundled_config_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("ANICAT_MPV_CONFIG_DIR") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            return Some(p);
        }
        log::warn!("[player] ANICAT_MPV_CONFIG_DIR={} is not a directory", p.display());
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("mpv")))
        .filter(|p| p.is_dir())
}

/// The viewer's own `mpv.conf`, if they have one.
pub fn user_config() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = dirs::config_dir().map(|d| d.join("mpv")); // %APPDATA%\mpv
    #[cfg(not(windows))]
    let base = dirs::home_dir().map(|d| d.join(".config").join("mpv"));
    base.map(|d| d.join("mpv.conf")).filter(|p| p.is_file())
}

pub struct Launch<'a> {
    pub url: &'a str,
    pub start_seconds: f64,
    pub media_title: &'a str,
    pub ipc: &'a Path,
    pub config_dir: Option<&'a Path>,
    pub user_config: Option<&'a Path>,
    /// Where compiled shaders are kept between launches.
    pub shader_cache: Option<&'a Path>,
    /// `ANICAT_MPV_EXTRA_ARGS`, already split.
    pub extra: &'a [String],
}

pub fn args(l: &Launch) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(dir) = l.config_dir {
        args.push(format!("--config-dir={}", dir.display()));
        // `--config-dir` replaces the viewer's config directory rather than
        // adding to it, so their mpv.conf is pulled in explicitly. Without a
        // bundled folder mpv reads theirs on its own, and including it again
        // would apply every line twice.
        if let Some(user) = l.user_config {
            args.push(format!("--include={}", user.display()));
        }
    }
    // The data dir, not `~~cache/`: with `--config-dir` set that resolves
    // inside the install folder, which an update replaces, and which is not
    // writable for a per-machine install. Without a persistent cache the
    // Anime4K chain is recompiled on every launch, a stall of seconds at the
    // first frame of every play.
    if let Some(dir) = l.shader_cache {
        args.push(format!("--gpu-shader-cache-dir={}", dir.display()));
    }
    args.push(format!("--input-ipc-server={}", l.ipc.display()));
    args.push(format!("--force-media-title={}", l.media_title));
    if l.start_seconds > 0.0 {
        args.push(format!("--start={:.0}", l.start_seconds));
    }
    // After the includes so a viewer's config cannot undo them. Resume is
    // ours (`--start` from the registry); mpv's watch-later would restore its
    // own position on top. `idle` or `keep-open` would keep mpv open at the
    // end of the playlist, and its exit is what releases the playing-file pin
    // and pauses the download.
    args.push("--resume-playback=no".into());
    args.push("--save-position-on-quit=no".into());
    args.push("--idle=no".into());
    args.push("--keep-open=no".into());
    args.extend(l.extra.iter().cloned());
    // `--` so a URL can never be read as an option.
    args.push("--".into());
    args.push(l.url.to_string());
    args
}

pub fn extra_args() -> Vec<String> {
    std::env::var("ANICAT_MPV_EXTRA_ARGS")
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_config_is_included_only_over_a_bundled_config_dir() {
        let ipc = PathBuf::from("/tmp/x.sock");
        let user = PathBuf::from("/home/u/.config/mpv/mpv.conf");
        let dir = PathBuf::from("/opt/anicat/mpv");
        let extra = vec!["--vo=null".to_string()];
        let cache = PathBuf::from("/data/mpv-shader-cache");
        let mut l = Launch {
            url: "http://127.0.0.1:1/s",
            start_seconds: 612.4,
            media_title: "Frieren - Episode 1",
            ipc: &ipc,
            config_dir: Some(&dir),
            user_config: Some(&user),
            shader_cache: Some(&cache),
            extra: &extra,
        };
        let a = args(&l);
        assert_eq!(a[0], "--config-dir=/opt/anicat/mpv");
        assert_eq!(a[1], "--include=/home/u/.config/mpv/mpv.conf");
        assert!(a.contains(&"--start=612".to_string()));
        assert!(a.contains(&"--gpu-shader-cache-dir=/data/mpv-shader-cache".to_string()));
        assert_eq!(a[a.len() - 2], "--");
        assert_eq!(a[a.len() - 1], "http://127.0.0.1:1/s");
        assert!(a.iter().position(|x| x == "--vo=null") > a.iter().position(|x| x == "--idle=no"));

        l.config_dir = None;
        l.start_seconds = 0.0;
        let a = args(&l);
        assert!(!a.iter().any(|x| x.starts_with("--include") || x.starts_with("--config-dir")));
        assert!(!a.iter().any(|x| x.starts_with("--start")));
    }
}
