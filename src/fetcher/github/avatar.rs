//! `github_avatar` — download a github.com avatar PNG and expose it as `Body::Image`.
//!
//! Safety::Safe: the host is hardcoded (`github.com/<user>.png`), so config can't redirect
//! traffic to a different origin. No auth is needed — the endpoint is public — so we do not
//! attach `GH_TOKEN`, avoiding accidental token exposure along the redirect chain.
//!
//! Caching: the PNG is written to `$SPLASHBOARD_HOME/cache/avatars/<user>-<size>.png`. The
//! fetcher framework already gates re-downloads via `refresh_interval`; we default to one week
//! since avatars rarely change.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::paths;
use crate::payload::{Body, ImageData, Payload, TextData};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::http;
use super::common::cache_key;

const AVATAR_BASE: &str = "https://github.com";
const DEFAULT_SIZE: u32 = 200;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "user",
        type_hint: "string (github login)",
        required: false,
        default: Some("$GITHUB_USER env var"),
        description: "GitHub login to fetch the avatar for. Falls back to the `GITHUB_USER` env var.",
    },
    OptionSchema {
        name: "size",
        type_hint: "integer (px)",
        required: false,
        default: Some("200"),
        description: "Requested avatar edge size in pixels. GitHub serves a square image of this size.",
    },
];

/// Downloads the user's avatar PNG to the cache and emits an `Image` body pointing at it.
pub struct GithubAvatar;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub size: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubAvatar {
    fn name(&self) -> &str {
        "github_avatar"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Image]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = ctx
            .options
            .as_ref()
            .and_then(|v| toml::to_string(v).ok())
            .unwrap_or_default();
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, _shape: Shape) -> Option<Body> {
        None
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return Ok(placeholder(&msg)),
        };
        let user = match resolve_user(opts.user.as_deref()) {
            Ok(u) => u,
            Err(msg) => return Ok(placeholder(&msg)),
        };
        let size = opts.size.unwrap_or(DEFAULT_SIZE);
        let path = match download_avatar(&user, size).await {
            Ok(p) => p,
            Err(e) => return Ok(placeholder(&e.to_string())),
        };
        Ok(payload(Body::Image(ImageData {
            path: path.to_string_lossy().into_owned(),
        })))
    }
}

async fn download_avatar(user: &str, size: u32) -> Result<PathBuf, FetchError> {
    let out_dir = avatar_dir()
        .ok_or_else(|| FetchError::Failed("$HOME not available for avatar cache".into()))?;
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| FetchError::Failed(format!("create avatar cache dir: {e}")))?;
    let url = format!("{AVATAR_BASE}/{user}.png?size={size}");
    let res = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("avatar request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("avatar {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("avatar body: {e}")))?;
    let path = out_dir.join(format!("{user}-{size}.png"));
    std::fs::write(&path, &bytes).map_err(|e| FetchError::Failed(format!("write avatar: {e}")))?;
    Ok(path)
}

fn avatar_dir() -> Option<PathBuf> {
    paths::cache_dir().map(|d| d.join("avatars"))
}

fn resolve_user(explicit: Option<&str>) -> Result<String, String> {
    if let Some(u) = explicit.filter(|s| !s.is_empty()) {
        return Ok(u.into());
    }
    if let Ok(u) = std::env::var("GITHUB_USER")
        && !u.is_empty()
    {
        return Ok(u);
    }
    Err("set `user = \"<login>\"` or the GITHUB_USER env var".into())
}

fn parse_options<T: serde::de::DeserializeOwned + Default>(
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

fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

fn placeholder(msg: &str) -> Payload {
    payload(Body::Text(TextData {
        value: format!("⚠ {msg}"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        restore: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(pairs: &[(&'static str, Option<&str>)]) -> Self {
            let lock = crate::paths::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let restore = pairs
                .iter()
                .map(|(k, v)| {
                    let prev = std::env::var(k).ok();
                    match v {
                        Some(value) => unsafe { std::env::set_var(k, value) },
                        None => unsafe { std::env::remove_var(k) },
                    }
                    (*k, prev)
                })
                .collect();
            Self {
                _lock: lock,
                restore,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.restore {
                match v {
                    Some(value) => unsafe { std::env::set_var(k, value) },
                    None => unsafe { std::env::remove_var(k) },
                }
            }
        }
    }

    #[test]
    fn resolve_user_prefers_explicit_option() {
        let _g = EnvGuard::set(&[("GITHUB_USER", Some("from-env"))]);
        assert_eq!(resolve_user(Some("from-opt")).unwrap(), "from-opt");
    }

    #[test]
    fn resolve_user_falls_back_to_env() {
        let _g = EnvGuard::set(&[("GITHUB_USER", Some("env-user"))]);
        assert_eq!(resolve_user(None).unwrap(), "env-user");
    }

    #[test]
    fn resolve_user_empty_option_falls_through_to_env() {
        let _g = EnvGuard::set(&[("GITHUB_USER", Some("env-user"))]);
        assert_eq!(resolve_user(Some("")).unwrap(), "env-user");
    }

    #[test]
    fn resolve_user_fails_when_neither_supplied() {
        let _g = EnvGuard::set(&[("GITHUB_USER", None)]);
        let err = resolve_user(None).unwrap_err();
        assert!(err.contains("GITHUB_USER"));
    }

    #[test]
    fn options_default_when_absent() {
        let opts: Options = parse_options(None).unwrap();
        assert!(opts.user.is_none());
        assert!(opts.size.is_none());
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = 1").unwrap();
        assert!(parse_options::<Options>(Some(&raw)).is_err());
    }
}
