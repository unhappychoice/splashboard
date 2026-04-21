# splashboard — render on new shell and on directory change
$ExecutionContext.InvokeCommand.LocationChangedAction = {
    & splashboard --on-cd
}
& splashboard
