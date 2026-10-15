//! SemVer update decisions.

use std::cmp::Ordering;

use semver::Version;
use serde::Serialize;
use ts_rs::TS;

/// Outcome of comparing the installed version with an advertised release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UpdateDecision {
    /// The candidate is newer and acceptable.
    Install,
    /// Same version (build metadata is ignored, per SemVer precedence).
    UpToDate,
    /// The candidate is older: never downgrade automatically.
    Older,
    /// The candidate is a pre-release and the installed build is stable.
    PrereleaseNotAllowed,
}

/// Decides whether `candidate` should be offered to a user running `current`.
///
/// * Precedence follows SemVer 2.0 (`1.0.0-beta < 1.0.0`, build metadata ignored).
/// * Stable installations are never moved to a pre-release unless
///   `allow_prerelease` is set; pre-release installations may move to newer
///   pre-releases or to stable.
pub fn decide(current: &Version, candidate: &Version, allow_prerelease: bool) -> UpdateDecision {
    match candidate.cmp_precedence(current) {
        Ordering::Less => UpdateDecision::Older,
        Ordering::Equal => UpdateDecision::UpToDate,
        Ordering::Greater => {
            if !candidate.pre.is_empty() && current.pre.is_empty() && !allow_prerelease {
                UpdateDecision::PrereleaseNotAllowed
            } else {
                UpdateDecision::Install
            }
        }
    }
}

/// Parses `v1.2.3` or `1.2.3` (as found in Git tags) into a version.
pub fn parse_tag(tag: &str) -> Result<Version, semver::Error> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn newer_stable_is_installed() {
        assert_eq!(
            decide(&v("0.1.0"), &v("0.1.1"), false),
            UpdateDecision::Install
        );
        assert_eq!(
            decide(&v("0.9.9"), &v("0.10.0"), false),
            UpdateDecision::Install
        );
        assert_eq!(
            decide(&v("1.0.0-beta.2"), &v("1.0.0"), false),
            UpdateDecision::Install
        );
    }

    #[test]
    fn same_or_older_is_not_installed() {
        assert_eq!(
            decide(&v("0.2.0"), &v("0.2.0"), false),
            UpdateDecision::UpToDate
        );
        assert_eq!(
            decide(&v("0.2.0"), &v("0.2.0+build.7"), false),
            UpdateDecision::UpToDate
        );
        assert_eq!(
            decide(&v("0.2.0"), &v("0.1.9"), false),
            UpdateDecision::Older
        );
        assert_eq!(
            decide(&v("1.0.0"), &v("1.0.0-rc.1"), true),
            UpdateDecision::Older
        );
    }

    #[test]
    fn stable_never_moves_to_prerelease_by_default() {
        assert_eq!(
            decide(&v("0.1.0"), &v("0.2.0-beta.1"), false),
            UpdateDecision::PrereleaseNotAllowed
        );
        assert_eq!(
            decide(&v("0.1.0"), &v("0.2.0-beta.1"), true),
            UpdateDecision::Install
        );
    }

    #[test]
    fn prerelease_users_follow_newer_prereleases() {
        assert_eq!(
            decide(&v("0.2.0-beta.1"), &v("0.2.0-beta.2"), false),
            UpdateDecision::Install
        );
        assert_eq!(
            decide(&v("0.2.0-beta.10"), &v("0.2.0-beta.9"), false),
            UpdateDecision::Older
        );
    }

    #[test]
    fn parses_tags() {
        assert_eq!(parse_tag("v0.1.0").unwrap(), v("0.1.0"));
        assert_eq!(parse_tag("1.2.3-rc.1").unwrap(), v("1.2.3-rc.1"));
        assert!(parse_tag("v1.2").is_err());
        assert!(parse_tag("latest").is_err());
    }
}
