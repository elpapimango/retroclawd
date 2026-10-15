# PowerShell entry point for tools/smoke_test.sh -- same arguments, e.g.  tools\smoke_test.ps1 --debug
# Needs Git for Windows (its bash runs the real script) and Rust (cargo) on PATH.
. "$PSScriptRoot\_gitbash.ps1"
Invoke-GitBashScript "smoke_test.sh" $args
