# Builds the release binary and the Windows installer (requires Inno Setup 6).
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") |
        Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { throw "ISCC.exe not found. Install Inno Setup 6 (winget install JRSoftware.InnoSetup)." }

& $iscc /Q "/DAppVersion=$version" installer\ikterminal.iss
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
Get-Item "target\installer\IkTerminal-$version-setup.exe"
