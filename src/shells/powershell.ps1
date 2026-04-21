# splashboard — render on new shell and on directory change
function Invoke-Splashboard {
    & splashboard
}
$ExecutionContext.InvokeCommand.LocationChangedAction = {
    Invoke-Splashboard
}
Invoke-Splashboard
