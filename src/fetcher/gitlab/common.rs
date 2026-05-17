//! Cross-cutting helpers for the `gitlab_*` fetchers: option parsing, project resolution, host
//! validation, cache-key construction, slug extraction from git remotes.
//!
//! Mirrors `github/common.rs` in shape; the differences are GitLab-specific:
//!
//! - [`ProjectPath`] accepts multi-segment paths (`group/subgroup/project`) because GitLab
//!   nests groups arbitrarily, whereas GitHub's owner/name pair is always two segments.
//! - Project lookups go to `/api/v4/projects/{path}` with the slashes URL-encoded as `%2F`.
//! - [`validate_host`] is the trust-boundary check: the `host` option ultimately decides where
//!   the auth token leaves to, so the parser rejects schemes / slashes / paths / dotless
//!   trailers before the value ever reaches [`super::client::rest_get`].

use chrono::DateTime;
use sha2::{Digest, Sha256};

use crate::fetcher::{FetchContext, FetchError};
use crate::payload::{Body, Payload};

pub fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

pub fn parse_options<T: serde::de::DeserializeOwned + Default>(
    raw: Option<&toml::Value>,
) -> Result<T, String> {
    match raw {
        None => Ok(T::default()),
        Some(value) => value
            .clone()
            .try_into::<T>()
            .map_err(|e| format!("invalid options: {e}")),
    }
}

/// Unix timestamp from an RFC3339 string. GitLab sends UTC strings like
/// `"2026-04-22T10:15:30.123Z"`; chrono parses the fractional seconds gracefully.
pub fn parse_timestamp(raw: &str) -> i64 {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
}

/// Cache key that mixes fetcher name + shape + format + auxiliary fields (host, project path,
/// reviewer scope). Stable for identical invocations so the daemon reuses the on-disk file.
pub fn cache_key(name: &str, ctx: &FetchContext, extra: &str) -> String {
    let shape = ctx.shape.map(|s| s.as_str()).unwrap_or("default");
    let raw = format!(
        "{}|{}|{}|{}",
        name,
        shape,
        ctx.format.as_deref().unwrap_or(""),
        extra
    );
    let digest = Sha256::digest(raw.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{name}-{hex}")
}

/// Validate `host` as a bare hostname. Trust-boundary: `host` is the only piece of config that
/// changes where the auth token leaves to, so we reject anything outside `[a-z0-9.-]` plus the
/// usual hostname structural rules (no leading / trailing dot or dash, no empty labels, no
/// schemes, no slashes).
pub fn validate_host(host: &str) -> Result<&str, String> {
    let too_short_or_long = host.is_empty() || host.len() > 253;
    let bad_chars = host
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'));
    let bad_anchor = host.starts_with('.')
        || host.ends_with('.')
        || host.starts_with('-')
        || host.ends_with('-');
    let empty_label = host.contains("..");
    if too_short_or_long || bad_chars || bad_anchor || empty_label {
        return Err(format!("invalid gitlab host: {host:?}"));
    }
    Ok(host)
}

/// GitLab project identifier. Can carry an arbitrary number of path segments because GitLab
/// nests groups (`my-group/sub-group/my-project`). Each segment is non-empty and
/// path-safe — slashes get percent-encoded for the API at lookup time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectPath(String);

impl ProjectPath {
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim().trim_matches('/');
        // GitLab projects always live under at least one group / namespace, so a bare
        // single-segment string ("group") never resolves to a real project. Reject it here
        // so the trailing-slash form ("group/") and a typo'd group name both fail fast
        // instead of generating a 404 path at the API.
        let segments: Vec<&str> = trimmed.split('/').collect();
        let valid = !trimmed.is_empty()
            && segments.len() >= 2
            && segments.iter().all(|seg| {
                !seg.is_empty()
                    && seg
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
            });
        valid.then(|| Self(trimmed.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// URL-encoded form used as the `{id}` placeholder in `/api/v4/projects/{id}/...`.
    pub fn encoded(&self) -> String {
        self.0.replace('/', "%2F")
    }

    /// Last segment — used by `repo_stars` headlines etc. where the full path would be too long.
    pub fn name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(self.as_str())
    }
}

/// Resolve the project for a repo-scope fetcher: explicit `options.project` wins, otherwise the
/// git remote of the cwd is parsed. Only matches the configured `host` so a cwd whose remote
/// lives on a different GitLab instance doesn't get silently mistargeted.
pub fn resolve_project(explicit: Option<&str>, host: &str) -> Result<ProjectPath, FetchError> {
    if let Some(raw) = explicit {
        return ProjectPath::parse(raw)
            .ok_or_else(|| FetchError::Failed(format!("invalid project option: {raw:?}")));
    }
    match remote_project_from_cwd(host) {
        Some(path) => Ok(path),
        None => Err(FetchError::Failed(
            "no project: set `project = \"group/name\"` or run inside a git repo with a gitlab \
             remote matching `host`"
                .into(),
        )),
    }
}

fn remote_project_from_cwd(host: &str) -> Option<ProjectPath> {
    let cwd = std::env::current_dir().ok()?;
    let repo = gix::discover(&cwd).ok()?;
    let names: Vec<_> = repo.remote_names().into_iter().collect();
    let preferred = names
        .iter()
        .find(|n| n.as_ref() == "origin")
        .or_else(|| names.first())?;
    let remote = repo.find_remote(preferred.as_ref()).ok()?;
    let url = remote.url(gix::remote::Direction::Fetch)?;
    project_from_url(&url.to_bstring().to_string(), host)
}

/// Extract a project path from a git remote URL, only if the URL's host matches the configured
/// GitLab `host`. Supports the four shapes splashboard users commonly write:
/// - `git@gitlab.com:group/project(.git)?`
/// - `https://gitlab.com/group/project(.git)?`
/// - `https://<user>(:<token>)?@gitlab.com/group/project(.git)?`
/// - `ssh://git@gitlab.com/group/project(.git)?`
pub fn project_from_url(url: &str, host: &str) -> Option<ProjectPath> {
    let host_ssh = format!("git@{host}:");
    let host_https = format!("https://{host}/");
    let host_ssh_scheme = format!("ssh://git@{host}/");
    let rest = url
        .strip_prefix(&host_ssh)
        .or_else(|| strip_https_userinfo(url, host))
        .or_else(|| url.strip_prefix(&host_https))
        .or_else(|| url.strip_prefix(&host_ssh_scheme))?;
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    ProjectPath::parse(rest)
}

fn strip_https_userinfo<'a>(url: &'a str, host: &str) -> Option<&'a str> {
    let after_scheme = url.strip_prefix("https://")?;
    let (userinfo, after_userinfo) = after_scheme.split_once('@')?;
    if userinfo.contains('/') {
        return None;
    }
    let host_slash = format!("{host}/");
    after_userinfo.strip_prefix(&host_slash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_host_accepts_canonical_gitlab_dot_com() {
        assert_eq!(validate_host("gitlab.com").unwrap(), "gitlab.com");
    }

    #[test]
    fn validate_host_accepts_self_hosted_subdomain() {
        assert_eq!(
            validate_host("git.example-corp.io").unwrap(),
            "git.example-corp.io"
        );
    }

    #[test]
    fn validate_host_rejects_schemes_and_paths() {
        assert!(validate_host("https://gitlab.com").is_err());
        assert!(validate_host("gitlab.com/api").is_err());
        assert!(validate_host("gitlab.com:8080").is_err());
    }

    #[test]
    fn validate_host_rejects_structural_garbage() {
        assert!(validate_host("").is_err());
        assert!(validate_host(".gitlab.com").is_err());
        assert!(validate_host("gitlab.com.").is_err());
        assert!(validate_host("-gitlab.com").is_err());
        assert!(validate_host("gitlab..com").is_err());
        assert!(validate_host("gitlab com").is_err());
    }

    #[test]
    fn project_path_parses_two_and_three_segments() {
        let two = ProjectPath::parse("group/proj").unwrap();
        assert_eq!(two.as_str(), "group/proj");
        assert_eq!(two.encoded(), "group%2Fproj");
        assert_eq!(two.name(), "proj");

        let three = ProjectPath::parse("group/sub/proj").unwrap();
        assert_eq!(three.encoded(), "group%2Fsub%2Fproj");
        assert_eq!(three.name(), "proj");
    }

    #[test]
    fn project_path_trims_leading_and_trailing_slashes() {
        let p = ProjectPath::parse("/group/proj/").unwrap();
        assert_eq!(p.as_str(), "group/proj");
    }

    #[test]
    fn project_path_rejects_invalid_segments() {
        assert!(ProjectPath::parse("").is_none());
        assert!(ProjectPath::parse("/").is_none());
        assert!(ProjectPath::parse("group/").is_none());
        assert!(ProjectPath::parse("group//proj").is_none());
        assert!(ProjectPath::parse("group/proj?weird").is_none());
        assert!(ProjectPath::parse("group/proj space").is_none());
    }

    #[test]
    fn project_from_url_matches_each_documented_shape() {
        let host = "gitlab.com";
        for url in [
            "git@gitlab.com:unhappychoice/splashboard.git",
            "https://gitlab.com/unhappychoice/splashboard",
            "https://gitlab.com/unhappychoice/splashboard.git",
            "https://fdncred@gitlab.com/unhappychoice/splashboard.git",
            "https://user:tok@gitlab.com/unhappychoice/splashboard",
            "ssh://git@gitlab.com/unhappychoice/splashboard.git",
        ] {
            let p = project_from_url(url, host).unwrap_or_else(|| panic!("could not parse {url}"));
            assert_eq!(p.as_str(), "unhappychoice/splashboard", "url={url}");
        }
    }

    #[test]
    fn project_from_url_carries_subgroups_through() {
        let p = project_from_url("git@gitlab.com:my-group/sub/proj.git", "gitlab.com").unwrap();
        assert_eq!(p.as_str(), "my-group/sub/proj");
    }

    #[test]
    fn project_from_url_rejects_non_matching_hosts() {
        // The host gate is what keeps a `gitlab_*` fetcher from picking up a GitHub remote on
        // a polyrepo machine; the `slug` parser already passes structurally, the host check is
        // the only safeguard.
        assert!(
            project_from_url("git@github.com:unhappychoice/splashboard.git", "gitlab.com")
                .is_none()
        );
        assert!(project_from_url("https://gitlab.example.org/u/proj.git", "gitlab.com").is_none());
    }

    #[test]
    fn parse_timestamp_reads_rfc3339_with_fractions() {
        assert_eq!(parse_timestamp("2026-04-22T10:15:30Z"), 1_776_852_930);
        assert_eq!(parse_timestamp("2026-04-22T10:15:30.123Z"), 1_776_852_930);
    }

    #[test]
    fn parse_timestamp_falls_back_to_zero_on_garbage() {
        assert_eq!(parse_timestamp("not-a-date"), 0);
    }

    #[test]
    fn cache_key_changes_with_extra_and_shape() {
        let ctx = FetchContext {
            shape: Some(crate::render::Shape::Entries),
            ..Default::default()
        };
        let a = cache_key("gitlab_repo_mrs", &ctx, "gitlab.com|group/proj");
        let b = cache_key("gitlab_repo_mrs", &ctx, "gitlab.com|group/proj2");
        assert_ne!(a, b);
        assert!(a.starts_with("gitlab_repo_mrs-"));
    }

    #[test]
    fn parse_options_returns_default_when_no_table_is_present() {
        #[derive(Debug, Default, PartialEq, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Opts {
            #[serde(default)]
            host: Option<String>,
        }
        let opts: Opts = parse_options(None).unwrap();
        assert_eq!(opts, Opts::default());
    }

    #[test]
    fn parse_options_surfaces_serde_failures() {
        #[derive(Debug, Default, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Opts {
            #[serde(default)]
            #[allow(dead_code)]
            host: Option<String>,
        }
        let raw: toml::Value = toml::from_str("bogus = 1").unwrap();
        let err = parse_options::<Opts>(Some(&raw)).unwrap_err();
        assert!(err.contains("invalid options"));
    }
}
