# splashboard — render on new shell and on directory change
$env.config = ($env.config? | default {})
$env.config.hooks = ($env.config.hooks? | default {})
$env.config.hooks.env_change = ($env.config.hooks.env_change? | default {})
$env.config.hooks.env_change.PWD = (
    $env.config.hooks.env_change.PWD?
    | default []
    | append {|before, after| if $before != null and $before != $after { ^splashboard --on-cd } }
)
^splashboard
