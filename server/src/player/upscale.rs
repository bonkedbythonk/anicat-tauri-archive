//! Anime4K: which chain, whether a file gets it, and the `glsl-shaders`
//! value that says so.

use std::path::Path;

use anicat_core::ffi::FfiCatalog;

/// Anime4K's "Mode A (Fast)" from its low-end GPU template, the chain both
/// the Tauri build (`apply_shader_args` in commands/playback.rs) and the Mac
/// (`Anime4KPreset.shaderFileNames`) use, in that order. The VL and HQ
/// variants pegged a MacBook's GPU; a laptop on Windows is no better off.
pub const CHAIN: [&str; 6] = [
    "Anime4K_Clamp_Highlights.glsl",
    "Anime4K_Restore_CNN_M.glsl",
    "Anime4K_Upscale_CNN_x2_M.glsl",
    "Anime4K_AutoDownscalePre_x2.glsl",
    "Anime4K_AutoDownscalePre_x4.glsl",
    "Anime4K_Upscale_CNN_x2_S.glsl",
];

/// mpv splits path-list options on `;` on Windows, where `:` is part of a
/// drive letter, and on `:` everywhere else.
#[cfg(windows)]
pub const PATH_LIST_SEP: &str = ";";
#[cfg(not(windows))]
pub const PATH_LIST_SEP: &str = ":";

/// From this height up the source is already at or past any display the
/// chain could upscale it for, and a 2x CNN pass over 3840x2160 frames costs
/// more GPU than most machines have for nothing visible.
pub const NATIVE_HEIGHT: i64 = 2160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    On,
    /// The viewer turned it off. Nothing to say about it.
    Disabled,
    /// A TMDB title. The chain is trained on line art and smears live action.
    LiveAction,
    Native4K,
    /// An anime file whose height mpv has not reported yet. Nothing is
    /// changed until it has: turning the chain on and then off again for a
    /// 2160p file is two render-graph rebuilds, each an audible stall.
    Pending,
}

impl Decision {
    /// The OSD line for a chain that is off for a reason the viewer did not
    /// choose. The live action text is the Mac's log line.
    pub fn message(self) -> Option<&'static str> {
        match self {
            Decision::LiveAction => Some("Anime4K off: live action"),
            Decision::Native4K => Some("Anime4K off: 2160p source"),
            _ => None,
        }
    }
}

pub fn decide(catalog: FfiCatalog, upscaling: bool, height: Option<i64>) -> Decision {
    if !upscaling {
        return Decision::Disabled;
    }
    if catalog != FfiCatalog::Anilist {
        return Decision::LiveAction;
    }
    match height {
        None => Decision::Pending,
        Some(h) if h >= NATIVE_HEIGHT => Decision::Native4K,
        Some(_) => Decision::On,
    }
}

/// The `glsl-shaders` value for the chain in `dir`, skipping files that are
/// not there: mpv refuses a missing shader file, and a build without the
/// shaders should play without upscaling rather than not play.
pub fn shader_list(dir: &Path, sep: &str) -> String {
    CHAIN
        .iter()
        .map(|name| dir.join(name))
        .filter(|p| p.is_file())
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(sep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anime_only_and_not_for_2160p() {
        use Decision::*;
        assert_eq!(decide(FfiCatalog::Anilist, true, Some(1080)), On);
        assert_eq!(decide(FfiCatalog::Anilist, true, Some(2160)), Native4K);
        assert_eq!(decide(FfiCatalog::Anilist, true, None), Pending);
        assert_eq!(decide(FfiCatalog::TmdbMovie, true, Some(1080)), LiveAction);
        assert_eq!(decide(FfiCatalog::TmdbTv, true, None), LiveAction);
        assert_eq!(decide(FfiCatalog::Anilist, false, Some(720)), Disabled);
        assert_eq!(Disabled.message(), None);
        assert_eq!(LiveAction.message(), Some("Anime4K off: live action"));
    }

    #[test]
    fn joins_present_files_in_chain_order_with_either_separator() {
        let dir = std::env::temp_dir().join(format!("anicat-shaders-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(shader_list(&dir, ":"), "");
        for name in CHAIN {
            std::fs::write(dir.join(name), b"//").unwrap();
        }
        // Built from the paths rather than split on the separator: a Windows
        // temp dir holds a ':' of its own.
        let expected = |names: &[&str], sep: &str| {
            names.iter().map(|n| dir.join(n).display().to_string()).collect::<Vec<_>>().join(sep)
        };
        assert_eq!(shader_list(&dir, ":"), expected(&CHAIN, ":"));
        assert_eq!(shader_list(&dir, ";"), expected(&CHAIN, ";"));
        assert!(expected(&CHAIN, ";").starts_with(&dir.join(CHAIN[0]).display().to_string()));
        std::fs::remove_file(dir.join(CHAIN[1])).unwrap();
        let without: Vec<&str> = CHAIN.iter().copied().filter(|n| *n != CHAIN[1]).collect();
        assert_eq!(shader_list(&dir, ";"), expected(&without, ";"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
