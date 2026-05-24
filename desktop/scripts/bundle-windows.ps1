# Bundle Observability Studio for Windows.
#
# Phase-1 distribution: a self-contained ZIP (binary + assets stub). MSI via
# WiX comes in a follow-up once we wire up a signing cert. The ZIP is enough
# for a downloadable artifact in CI today.
#
# Usage (from desktop/):
#   pwsh scripts/bundle-windows.ps1
#
# Outputs:
#   target\bundle\windows\SmooAI-Observability-Studio-x86_64.zip

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$DesktopDir = Split-Path -Parent $ScriptDir
Set-Location $DesktopDir

$AppName = "SmooAI Observability Studio"
$BinName = "observability-studio.exe"
$VersionLine = (Select-String -Path "crates/observability-studio-app/Cargo.toml" -Pattern '^version' | Select-Object -First 1).Line
$AppVersion = ($VersionLine -split '"')[1]
$BundleOut = "target\bundle\windows"
$StageDir = Join-Path $BundleOut "SmooAI-Observability-Studio"
$ZipOut = Join-Path $BundleOut "SmooAI-Observability-Studio-x86_64.zip"

if ($args -notcontains "--skip-build") {
    Write-Host "▸ cargo build --release"
    cargo build --release -p observability-studio-app
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

$MetaJson = cargo metadata --format-version=1 --no-deps | ConvertFrom-Json
$BinSrc = Join-Path $MetaJson.target_directory "release\$BinName"
if (-not (Test-Path $BinSrc)) { throw "release binary not found at $BinSrc" }

Write-Host "▸ staging $StageDir"
if (Test-Path $StageDir) { Remove-Item -Recurse -Force $StageDir }
New-Item -ItemType Directory -Path $StageDir | Out-Null
Copy-Item $BinSrc (Join-Path $StageDir $BinName)
Copy-Item "assets\icons\icon.png" (Join-Path $StageDir "icon.png")

@"
SmooAI Observability Studio v$AppVersion
Native client for SmooAI logs, errors, metrics.
Run observability-studio.exe.
"@ | Set-Content -Path (Join-Path $StageDir "README.txt") -Encoding UTF8

Write-Host "▸ packaging $ZipOut"
if (Test-Path $ZipOut) { Remove-Item -Force $ZipOut }
Compress-Archive -Path "$StageDir\*" -DestinationPath $ZipOut

Write-Host "✓ bundle ready"
Get-Item $ZipOut | Format-List Name, Length
