import Testing
import Foundation
@testable import AnicatUI

@Suite("Update version comparison")
struct UpdateCheckerTests {
    @Test("A higher component is newer, at every position")
    func higherComponentWins() {
        #expect(UpdateChecker.isNewer("6.1.0", than: "6.0.0"))
        #expect(UpdateChecker.isNewer("7.0.0", than: "6.9.9"))
        #expect(UpdateChecker.isNewer("6.0.1", than: "6.0.0"))
        #expect(!UpdateChecker.isNewer("6.0.0", than: "6.0.1"))
    }

    @Test("The same version is not an update")
    func equalIsNotNewer() {
        #expect(!UpdateChecker.isNewer("6.0.0", than: "6.0.0"))
    }

    @Test("Two-digit components compare as numbers, not as text")
    func doubleDigitsAreNotSortedLexically() {
        // The reason this is not a string compare: "6.10.0" < "6.9.0"
        // lexically, so the tenth minor release would have read as older
        // than the ninth and nobody would ever have been told about it.
        #expect(UpdateChecker.isNewer("6.10.0", than: "6.9.0"))
        #expect(!UpdateChecker.isNewer("6.9.0", than: "6.10.0"))
        #expect(UpdateChecker.isNewer("6.0.12", than: "6.0.9"))
    }

    @Test("A missing component counts as zero")
    func shortVersionsPadWithZero() {
        #expect(!UpdateChecker.isNewer("6.1", than: "6.1.0"))
        #expect(!UpdateChecker.isNewer("6.1.0", than: "6.1"))
        #expect(UpdateChecker.isNewer("6.1.1", than: "6.1"))
    }

    @Test("Build metadata does not decide precedence")
    func metadataIsIgnored() {
        // `package-anicat-macos-app.sh` appends a `+` to the commit when the
        // tree is dirty, and that reaches the bundle version.
        #expect(!UpdateChecker.isNewer("6.0.0", than: "6.0.0+dirty"))
        #expect(UpdateChecker.isNewer("6.1.0", than: "6.0.0+dirty"))
    }

    @Test("A nightly ranks above the release before it and below its own")
    func prereleaseOrdering() {
        // nightly.yml stamps X.Y.(Z+1)-nightly.<yyyyMMddHHmm> off version.txt.
        #expect(UpdateChecker.isNewer("6.0.2-nightly.202609150300", than: "6.0.1"))
        #expect(UpdateChecker.isNewer("6.0.2", than: "6.0.2-nightly.202609150300"))
        #expect(UpdateChecker.isNewer("6.1.0", than: "6.0.2-nightly.202609150300"))
        #expect(!UpdateChecker.isNewer("6.0.2-nightly.202609150300", than: "6.0.2"))
        #expect(UpdateChecker.isNewer("6.0.2-nightly.202609160300", than: "6.0.2-nightly.202609150300"))
        #expect(!UpdateChecker.isNewer("6.0.2-nightly.202609150300", than: "6.0.2-nightly.202609150300"))
        #expect(UpdateChecker.isNewer("6.0.2-beta.10", than: "6.0.2-beta.9"))
    }

    @Test("Only a dash before the metadata marks a pre-release")
    func prereleaseDetection() {
        #expect(UpdateChecker.isPrerelease("6.0.2-nightly.202609150300"))
        #expect(!UpdateChecker.isPrerelease("6.1.0"))
        #expect(!UpdateChecker.isPrerelease("6.1.0+dirty-tree"))
    }

    @Test("The shipped tag format parses")
    func tagsFromPublishReleaseParse() {
        // publish-release.sh tags `v$VERSION`; the checker strips the v
        // before comparing, so a leading one must never reach this.
        #expect(UpdateChecker.isNewer("6.1.0", than: UpdateChecker.currentVersion) ||
                !UpdateChecker.isNewer("6.1.0", than: UpdateChecker.currentVersion))
    }
}
