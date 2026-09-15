import Foundation

/// Whether a newer build has been published, asked of the GitHub releases
/// API for the repository this app is cut from.
///
/// There was no update mechanism at all: the Settings card said so in as many
/// words, so an install sat on whatever version it was downloaded at until
/// somebody happened to visit the releases page. A single unauthenticated
/// GET answers it, which is the whole of what the shipping arrangement
/// needs -- `scripts/publish-release.sh` tags `v<version>` and attaches
/// `Anicat-<version>-macos-arm64.zip`, so the tag alone says what is out.
///
/// Deliberately not Sparkle, and not a self-replacing updater of our own: the
/// owner chose the one-line installer (`scripts/install_macos.sh`) as the
/// update path in 2026-09. Not because the ad-hoc signature rules Sparkle out
/// -- its validator accepts an update on an EdDSA signature alone when the
/// host has no Developer ID (read from `SUUpdateValidator`, never tried
/// here) -- but because a signing key, an appcast and an updater's failure
/// modes are more than this distribution wants to carry.
public enum UpdateChecker {
    public struct Release: Sendable, Equatable {
        /// Tag with the leading `v` stripped, so it compares against
        /// `CFBundleShortVersionString` directly.
        public let version: String
        public let pageURL: URL
        public let notes: String?
    }

    /// Drafts and pre-releases are excluded by `/releases/latest` itself,
    /// which is why `publish-release.sh` may create drafts, and the nightly
    /// workflow may replace its pre-release every night, without every stable
    /// install being told about them.
    static let endpoint = URL(string: "https://api.github.com/repos/bonkedbythonk/anicat/releases/latest")!
    /// The nightly's view: every published release, pre-releases included.
    /// Read only by a build that is itself a pre-release.
    static let listEndpoint = URL(string: "https://api.github.com/repos/bonkedbythonk/anicat/releases?per_page=20")!

    /// Not checked more than once a day. A launch is not a reason to spend a
    /// request, and GitHub rate-limits unauthenticated callers by IP -- which
    /// on a shared network is everyone at once.
    static let checkInterval: TimeInterval = 24 * 60 * 60
    static let lastCheckKey = "anicat_last_update_check"
    /// The version whose prompt has already been answered. Stored rather
    /// than a bare bool so the next release asks again.
    public static let dismissedVersionKey = "anicat_update_prompt_dismissed"

    public static var currentVersion: String {
        // `ANICAT_FAKE_VERSION` pretends this build is older than it is, so
        // the prompt can be exercised against the real API without waiting
        // for a release newer than the running app -- which is otherwise the
        // only way to see it, and so the one path that would ship untested.
        // Same shape as the other `ANICAT_*` debug hooks; absent in normal
        // use, and it only ever makes the app *more* willing to say an
        // update exists, never less.
        if let forced = ProcessInfo.processInfo.environment["ANICAT_FAKE_VERSION"],
           !forced.isEmpty {
            return forced
        }
        return bundleVersion ?? "0"
    }

    /// The bundle's version, or nil when there is no readable Info.plist --
    /// a binary run straight out of `.build`, or a bundle a packaging step
    /// left without the key. `currentVersion` still answers "0" for the
    /// User-Agent and the Settings row, but "0" is a version every release
    /// ever published is newer than, so the update check treated an unknown
    /// build as maximally out of date and opened a prompt offering to
    /// "update" it to something older than itself.
    static var bundleVersion: String? {
        (Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String)
            .flatMap { $0.isEmpty ? nil : $0 }
    }

    /// Compares versions without `import` of anything.
    ///
    /// Not a string compare: "6.10.0" sorts before "6.9.0" lexically, so a
    /// tenth minor release would have read as older than the ninth and
    /// nobody would have been told about it. Missing components count as
    /// zero, so "6.1" and "6.1.0" are the same version.
    ///
    /// A `-nightly.N` suffix is a pre-release and ranks below the same
    /// version without one, as in semver. Stripping it like build metadata
    /// made every nightly of a version equal, so a nightly install was never
    /// told about the next one, nor about that version once it shipped.
    public static func isNewer(_ candidate: String, than current: String) -> Bool {
        func split(_ s: String) -> (core: [Int], pre: [Substring]) {
            // Anything after a `+` is build metadata, not precedence.
            let noMeta = s.split(separator: "+", maxSplits: 1).first ?? Substring(s)
            let halves = noMeta.split(separator: "-", maxSplits: 1)
            let core = (halves.first ?? "").split(separator: ".").map { Int($0.filter(\.isNumber)) ?? 0 }
            let pre = halves.count > 1 ? halves[1].split(separator: ".") : []
            return (core, pre)
        }
        let a = split(candidate), b = split(current)
        for i in 0..<max(a.core.count, b.core.count) {
            let l = i < a.core.count ? a.core[i] : 0
            let r = i < b.core.count ? b.core[i] : 0
            if l != r { return l > r }
        }
        if a.pre.isEmpty || b.pre.isEmpty { return a.pre.isEmpty && !b.pre.isEmpty }
        for i in 0..<max(a.pre.count, b.pre.count) {
            guard i < a.pre.count else { return false }
            guard i < b.pre.count else { return true }
            let l = a.pre[i], r = b.pre[i]
            if l == r { continue }
            if let li = Int(l), let ri = Int(r) { return li > ri }
            return l > r
        }
        return false
    }

    /// Whether this build is a pre-release, i.e. its version carries a `-`
    /// suffix. Such a build was installed on purpose from the nightly and is
    /// offered newer nightlies as well as stable releases.
    static func isPrerelease(_ version: String) -> Bool {
        (version.split(separator: "+", maxSplits: 1).first ?? "").contains("-")
    }

    /// `nil` on any failure at all -- no network, rate limited, a repository
    /// with no releases yet. An update check that cannot reach GitHub is not
    /// something to interrupt anyone about.
    public static func latestRelease(includingPrereleases: Bool = false) async -> Release? {
        var request = URLRequest(url: includingPrereleases ? listEndpoint : endpoint)
        // GitHub asks for an explicit Accept and rejects an empty User-Agent.
        request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        request.setValue("Anicat/\(currentVersion)", forHTTPHeaderField: "User-Agent")
        request.timeoutInterval = 15

        guard let (data, response) = try? await URLSession.shared.data(for: request),
              let http = response as? HTTPURLResponse, http.statusCode == 200,
              let json = try? JSONSerialization.jsonObject(with: data)
        else { return nil }

        let objects: [[String: Any]]
        if includingPrereleases {
            objects = (json as? [[String: Any]] ?? []).filter { ($0["draft"] as? Bool) != true }
        } else {
            objects = (json as? [String: Any]).map { [$0] } ?? []
        }
        let releases: [Release] = objects.compactMap { json in
            guard let tag = json["tag_name"] as? String,
                  let page = (json["html_url"] as? String).flatMap(URL.init(string:))
            else { return nil }
            // The nightly lives under the fixed tag `nightly` so the installer
            // has a stable URL; its version is only in the title, which
            // nightly.yml writes as "Anicat <version>". Read as a tag, it
            // parsed as version 0 and no nightly was ever newer than anything.
            let version: String
            if tag.hasPrefix("v") {
                version = String(tag.dropFirst())
            } else if let name = json["name"] as? String,
                      let last = name.split(separator: " ").last,
                      last.first?.isNumber == true {
                version = String(last)
            } else {
                return nil
            }
            let notes = (json["body"] as? String).flatMap { $0.isEmpty ? nil : $0 }
            return Release(version: version, pageURL: page, notes: notes)
        }
        // Highest version, not the first in the list: GitHub orders by
        // creation date, and the nightly is recreated every night, so it
        // would sit on top even after a newer stable release shipped.
        return releases.max { isNewer($1.version, than: $0.version) }
    }

    /// The newer release, or nil when this build is current. `force` skips
    /// the once-a-day gate, for the button in Settings.
    public static func check(force: Bool) async -> Release? {
        let defaults = UserDefaults.standard
        if !force {
            let last = defaults.double(forKey: lastCheckKey)
            if last > 0, Date().timeIntervalSince1970 - last < checkInterval { return nil }
        }
        // Nothing to compare against, so nothing to offer. `ANICAT_FAKE_VERSION`
        // still gets through: it is set precisely to exercise this path.
        let running = ProcessInfo.processInfo.environment["ANICAT_FAKE_VERSION"]
            .flatMap { $0.isEmpty ? nil : $0 } ?? bundleVersion
        guard let running else { return nil }
        guard let release = await latestRelease(includingPrereleases: isPrerelease(running)) else { return nil }
        defaults.set(Date().timeIntervalSince1970, forKey: lastCheckKey)
        guard isNewer(release.version, than: running) else { return nil }
        return release
    }
}
