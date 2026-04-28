//! Loader for `$HOME/.splashboard/secrets.toml` — a flat top-level TOML map of
//! `KEY = "value"` pairs that get exported as process env at startup. Lives in its own
//! file (not `settings.toml`) so dotfiles-in-git users can git-ignore exactly the file
//! that holds tokens without losing their settings.
//!
//! Shell env wins: a key already present in the environment is left alone, so a
//! `GH_TOKEN=... splashboard ...` style override still works.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

/// Keys we refuse to import from `secrets.toml`. Two reasons:
///
/// - `SPLASHBOARD_*` controls splashboard's own behavior (HOME, log filter, trust escape
///   hatch). Letting a config file flip those creates a confusing layered setup.
/// - `PATH` / `HOME` / `SHELL` etc. are ambient process state owned by the shell. A typo
///   here could redirect `git2`/`gix` to a different binary or break HOME discovery.
const DENYLIST: &[&str] = &[
    "PATH",
    "HOME",
    "SHELL",
    "USER",
    "LOGNAME",
    "PWD",
    "OLDPWD",
    "IFS",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct SecretsConfig {
    entries: BTreeMap<String, String>,
}

impl SecretsConfig {
    pub fn parse(toml_str: &str) -> Result<Self, String> {
        toml::from_str(toml_str).map_err(|e| e.to_string())
    }

    /// Loads the file if present, returns an empty config if missing, errors on parse
    /// failure. Same shape as `SettingsConfig::load_or_default`.
    pub fn load_or_default(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::parse(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Iterate the entries that survived denylist + `SPLASHBOARD_*` filtering. Skips
    /// silently rather than erroring so a stray entry doesn't tank the whole splash.
    pub fn importable(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .filter(|(k, _)| is_importable(k))
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Apply each entry to the process env, but only if the env doesn't already have a
    /// value for that key. Shell env always wins. Returns the keys actually set.
    pub fn apply_to_env<F, G>(&self, get: F, mut set: G) -> Vec<String>
    where
        F: Fn(&str) -> Option<String>,
        G: FnMut(&str, &str),
    {
        self.importable()
            .filter(|(k, _)| get(k).is_none())
            .map(|(k, v)| {
                set(k, v);
                k.to_string()
            })
            .collect()
    }
}

fn is_importable(key: &str) -> bool {
    !key.starts_with("SPLASHBOARD_") && !DENYLIST.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_toml() {
        let s = SecretsConfig::parse("GH_TOKEN = \"ghp_x\"\nTODOIST_TOKEN = \"t_y\"\n").unwrap();
        let mut got: Vec<_> = s.importable().collect();
        got.sort();
        assert_eq!(got, vec![("GH_TOKEN", "ghp_x"), ("TODOIST_TOKEN", "t_y")]);
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let s = SecretsConfig::load_or_default(&dir.path().join("secrets.toml")).unwrap();
        assert_eq!(s.importable().count(), 0);
    }

    #[test]
    fn parse_failure_surfaces() {
        let err = SecretsConfig::parse("not = valid = toml").unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn denylist_blocks_dangerous_keys() {
        let s = SecretsConfig::parse("PATH = \"/tmp\"\nGH_TOKEN = \"x\"\n").unwrap();
        let keys: Vec<_> = s.importable().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["GH_TOKEN"]);
    }

    #[test]
    fn splashboard_prefix_is_ignored() {
        let s = SecretsConfig::parse(
            "SPLASHBOARD_HOME = \"/x\"\nSPLASHBOARD_TRUST_ALL = \"1\"\nGH_TOKEN = \"x\"\n",
        )
        .unwrap();
        let keys: Vec<_> = s.importable().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["GH_TOKEN"]);
    }

    #[test]
    fn shell_env_wins_over_secrets() {
        let s = SecretsConfig::parse("GH_TOKEN = \"from_secrets\"\nNEW_KEY = \"new\"\n").unwrap();
        let env: BTreeMap<&str, &str> = [("GH_TOKEN", "from_shell")].into();
        let mut written: BTreeMap<String, String> = BTreeMap::new();
        let applied = s.apply_to_env(
            |k| env.get(k).map(|v| v.to_string()),
            |k, v| {
                written.insert(k.to_string(), v.to_string());
            },
        );
        assert_eq!(applied, vec!["NEW_KEY"]);
        assert_eq!(written.get("NEW_KEY").map(|s| s.as_str()), Some("new"));
        assert!(!written.contains_key("GH_TOKEN"));
    }
}
