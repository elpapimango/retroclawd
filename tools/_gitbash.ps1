# Shared helper: run one of the bash scripts in tools/ through Git for Windows' bash.
# (The scripts are the single implementation; the .ps1 files next to them only start them.)
function Find-GitBash {
    # Not "bash" from PATH: on Windows that is often WSL's bash.exe, which cannot see these paths.
    $candidates = @(
        "$env:ProgramFiles\Git\bin\bash.exe",
        "${env:ProgramFiles(x86)}\Git\bin\bash.exe",
        "$env:LOCALAPPDATA\Programs\Git\bin\bash.exe"
    )
    $git = Get-Command git -ErrorAction SilentlyContinue
    if ($git) {
        # ...\Git\cmd\git.exe  ->  ...\Git\bin\bash.exe
        $candidates += (Join-Path (Split-Path (Split-Path $git.Source -Parent) -Parent) "bin\bash.exe")
    }
    foreach ($c in $candidates) { if (Test-Path $c) { return $c } }
    throw "Git for Windows' bash.exe was not found. Install Git for Windows (https://gitforwindows.org) or run the .sh script from WSL."
}

function Invoke-GitBashScript {
    param([string]$Script, [string[]]$ScriptArgs)
    $bash = Find-GitBash
    $path = (Join-Path $PSScriptRoot $Script) -replace '\\', '/'
    $env:CHERE_INVOKING = "1"          # keep the current folder in a login shell
    & $bash -l $path @ScriptArgs
    exit $LASTEXITCODE
}
