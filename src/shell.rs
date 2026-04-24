use std::path::PathBuf;

use clap::ValueEnum;

const BASH: &str = include_str!("shells/bash.sh");
const ZSH: &str = include_str!("shells/zsh.sh");
const FISH: &str = include_str!("shells/fish.fish");
const POWERSHELL: &str = include_str!("shells/powershell.ps1");

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

impl Shell {
    pub fn as_str(self) -> &'static str {
        match self {
            Shell::Bash => "bash",
            Shell::Zsh => "zsh",
            Shell::Fish => "fish",
            Shell::Powershell => "powershell",
        }
    }
}

pub fn init_snippet(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => BASH,
        Shell::Zsh => ZSH,
        Shell::Fish => FISH,
        Shell::Powershell => POWERSHELL,
    }
}

/// Best-effort guess from `$SHELL` (or `$PSModulePath` for Windows PowerShell). Returns
/// `None` when neither env var points to a recognised shell so callers can fall back to
/// an interactive picker or fail with a clear message.
pub fn detect_shell(env: impl Fn(&str) -> Option<String>) -> Option<Shell> {
    if let Some(shell) = env("SHELL").as_deref().and_then(classify_shell_path) {
        return Some(shell);
    }
    // PowerShell typically doesn't set `$SHELL`; this env var is a strong signal we're
    // running under it (present on both Windows PowerShell and PowerShell 7 sessions).
    if env("PSModulePath").is_some() {
        return Some(Shell::Powershell);
    }
    None
}

fn classify_shell_path(path: &str) -> Option<Shell> {
    // Strip the directory prefix by splitting on either separator ourselves — Rust's
    // `Path` is platform-aware (`\` isn't a separator on Unix), so using it here would
    // leave `C:\...\pwsh.exe` as one opaque segment on Linux tests and in WSL. Split on
    // both `/` and `\` then trim the trailing `.exe` so `zsh.exe` / `pwsh.exe` classify
    // on every platform.
    let basename = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let bare = basename
        .split('.')
        .next()
        .unwrap_or(basename)
        .to_ascii_lowercase();
    match bare.as_str() {
        "bash" => Some(Shell::Bash),
        "zsh" => Some(Shell::Zsh),
        "fish" => Some(Shell::Fish),
        "pwsh" | "powershell" => Some(Shell::Powershell),
        _ => None,
    }
}

/// Default rc file path for the given shell. Callers should still check existence and
/// decide whether to create the file (creating a missing `~/.zshrc` is fine, creating
/// `$PROFILE` on a Windows box where it's missing needs `--yes`).
pub fn default_rc_path(shell: Shell) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(match shell {
        Shell::Bash => home.join(".bashrc"),
        Shell::Zsh => home.join(".zshrc"),
        Shell::Fish => home.join(".config").join("fish").join("config.fish"),
        // PowerShell's real answer is `$PROFILE` which we can't resolve without invoking
        // pwsh. This path is the cross-version CurrentUserAllHosts default and is what
        // most users edit; callers should treat missing parent dirs as "needs --yes".
        Shell::Powershell => home
            .join("Documents")
            .join("PowerShell")
            .join("Microsoft.PowerShell_profile.ps1"),
    })
}

/// Source line appended inside the marker block; delegates to `splashboard init <shell>`
/// so any snippet update lands automatically on next shell start.
pub fn source_line(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => "eval \"$(splashboard init bash)\"",
        Shell::Zsh => "eval \"$(splashboard init zsh)\"",
        Shell::Fish => "splashboard init fish | source",
        Shell::Powershell => "Invoke-Expression (& splashboard init powershell | Out-String)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn bash_snippet_hooks_prompt_command() {
        let s = init_snippet(Shell::Bash);
        assert!(s.contains("PROMPT_COMMAND"));
        assert!(s.contains("__splashboard_on_chpwd"));
    }

    #[test]
    fn zsh_snippet_uses_chpwd_hook() {
        let s = init_snippet(Shell::Zsh);
        assert!(s.contains("add-zsh-hook chpwd"));
    }

    #[test]
    fn fish_snippet_watches_pwd_variable() {
        let s = init_snippet(Shell::Fish);
        assert!(s.contains("--on-variable PWD"));
    }

    #[test]
    fn powershell_snippet_sets_location_changed_action() {
        let s = init_snippet(Shell::Powershell);
        assert!(s.contains("LocationChangedAction"));
    }

    #[test]
    fn all_snippets_guard_interactivity() {
        assert!(init_snippet(Shell::Bash).contains("$-"));
        assert!(init_snippet(Shell::Zsh).contains("interactive"));
        assert!(init_snippet(Shell::Fish).contains("is-interactive"));
    }

    #[test]
    fn cd_hooks_call_on_cd_flag_not_bare_splash() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::Powershell] {
            let s = init_snippet(shell);
            assert!(
                s.contains("splashboard --on-cd"),
                "{:?} snippet missing --on-cd",
                shell
            );
        }
    }

    #[test]
    fn detect_maps_known_shells_from_shell_env() {
        assert_eq!(
            detect_shell(env_with(&[("SHELL", "/bin/zsh")])),
            Some(Shell::Zsh)
        );
        assert_eq!(
            detect_shell(env_with(&[("SHELL", "/usr/local/bin/bash")])),
            Some(Shell::Bash)
        );
        assert_eq!(
            detect_shell(env_with(&[("SHELL", "/opt/homebrew/bin/fish")])),
            Some(Shell::Fish)
        );
    }

    #[test]
    fn detect_falls_back_to_powershell_when_psmodulepath_is_set() {
        let env = env_with(&[(
            "PSModulePath",
            "C:\\Users\\x\\Documents\\PowerShell\\Modules",
        )]);
        assert_eq!(detect_shell(env), Some(Shell::Powershell));
    }

    #[test]
    fn detect_returns_none_for_unknown_shell() {
        assert_eq!(detect_shell(env_with(&[("SHELL", "/bin/ksh")])), None);
        assert_eq!(detect_shell(env_with(&[])), None);
    }

    /// Windows-style `$SHELL` values (rare but seen on WSL with a PowerShell shim) carry
    /// `\` separators and a `.exe` suffix. The classifier has to strip both to resolve
    /// the shell identity.
    #[test]
    fn detect_handles_windows_style_paths() {
        assert_eq!(
            detect_shell(env_with(&[(
                "SHELL",
                "C:\\Program Files\\PowerShell\\7\\pwsh.exe"
            )])),
            Some(Shell::Powershell)
        );
        assert_eq!(
            detect_shell(env_with(&[("SHELL", "C:\\cygwin64\\bin\\zsh.exe")])),
            Some(Shell::Zsh)
        );
    }

    #[test]
    fn default_rc_path_for_known_shells_ends_in_expected_file() {
        let rc = default_rc_path(Shell::Zsh).unwrap();
        assert!(rc.ends_with(".zshrc"));
        let rc = default_rc_path(Shell::Bash).unwrap();
        assert!(rc.ends_with(".bashrc"));
        let rc = default_rc_path(Shell::Fish).unwrap();
        assert!(rc.ends_with("config.fish"));
    }

    #[test]
    fn source_line_covers_all_shells() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::Powershell] {
            let line = source_line(shell);
            assert!(line.contains("splashboard"), "{:?}", shell);
        }
    }
}
