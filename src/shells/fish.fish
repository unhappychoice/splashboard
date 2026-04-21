# splashboard — render on new shell and on directory change
if status is-interactive
    function __splashboard_render_on_cd --on-variable PWD
        command splashboard
    end
    command splashboard
end
