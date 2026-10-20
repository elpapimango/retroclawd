# PowerShell entry point for tools/check_all.sh -- same arguments, e.g.  tools\check_all.ps1 --acid
# Needs Git for Windows (its bash runs the real script) and Rust (cargo) on PATH.
. "$PSScriptRoot\_gitbash.ps1"
Invoke-GitBashScript "check_all.sh" $args
