# splashboard — render on new shell and on directory change
if [[ -o interactive ]]; then
  __splashboard_on_cd() {
    command splashboard --on-cd
  }
  autoload -U add-zsh-hook
  add-zsh-hook chpwd __splashboard_on_cd
  command splashboard
fi
