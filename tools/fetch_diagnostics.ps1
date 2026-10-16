# PowerShell entry point for tools/fetch_diagnostics.sh -- same arguments.
# Needs Git for Windows (its bash runs the real script), curl and python3.
. "$PSScriptRoot\_gitbash.ps1"
Invoke-GitBashScript "fetch_diagnostics.sh" $args
