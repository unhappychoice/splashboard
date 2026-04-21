# splashboard — render on new shell and on directory change
if [[ $- == *i* ]]; then
  __splashboard_render() {
    command splashboard
  }
  __splashboard_on_chpwd() {
    if [[ "$PWD" != "$__SPLASHBOARD_LAST_PWD" ]]; then
      __SPLASHBOARD_LAST_PWD="$PWD"
      __splashboard_render
    fi
  }
  __SPLASHBOARD_LAST_PWD="$PWD"
  PROMPT_COMMAND="__splashboard_on_chpwd${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
  __splashboard_render
fi
