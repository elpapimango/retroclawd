# PowerShell entry point for tools/build_all.sh -- same arguments, e.g.  tools\build_all.ps1 --debug
# Needs Git for Windows (its bash runs the real script) and Rust (cargo) on PATH.
. "$PSScriptRoot\_gitbash.ps1"
Invoke-GitBashScript "build_all.sh" $args
