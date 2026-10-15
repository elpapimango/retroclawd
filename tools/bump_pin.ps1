# PowerShell entry point for tools/bump_pin.sh -- same arguments, e.g.  tools\bump_pin.ps1 --debug
# Needs Git for Windows (its bash runs the real script) and Rust (cargo) on PATH.
. "$PSScriptRoot\_gitbash.ps1"
Invoke-GitBashScript "bump_pin.sh" $args
