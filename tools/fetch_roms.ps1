# PowerShell entry point for tools/fetch_roms.sh -- same arguments, e.g.  toolsetch_roms.ps1 --debug
# Needs Git for Windows (its bash runs the real script), ROMM_URL and ROMM_TOKEN.
. "$PSScriptRoot\_gitbash.ps1"
Invoke-GitBashScript "fetch_roms.sh" $args
