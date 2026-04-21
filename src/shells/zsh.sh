# splashboard — render on new shell and on directory change
if [[ -o interactive ]]; then
  __splashboard_render() {
    command splashboard
  }
  autoload -U add-zsh-hook
  add-zsh-hook chpwd __splashboard_render
  __splashboard_render
fi
