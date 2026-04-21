# splashboard — render on new shell and on directory change
if status is-interactive
    function __splashboard_on_cd --on-variable PWD
        command splashboard --on-cd
    end
    command splashboard
end
