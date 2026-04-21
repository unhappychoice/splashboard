use clap::ValueEnum;

const BASH: &str = include_str!("shells/bash.sh");
const ZSH: &str = include_str!("shells/zsh.sh");
const FISH: &str = include_str!("shells/fish.fish");
const POWERSHELL: &str = include_str!("shells/powershell.ps1");

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

pub fn init_snippet(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => BASH,
        Shell::Zsh => ZSH,
        Shell::Fish => FISH,
        Shell::Powershell => POWERSHELL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
