# Run from the extracted Windows GitHub Release bundle.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$source = Join-Path $root 'coconut.exe'
if (-not (Test-Path $source)) { throw 'coconut.exe was not found next to install.ps1.' }
$destination = Join-Path $env:LOCALAPPDATA 'CoconutPilot'
New-Item -ItemType Directory -Force -Path $destination | Out-Null
Copy-Item -Force $source (Join-Path $destination 'coconut.exe')
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (";$userPath;" -notlike "*;$destination;*") {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$destination", 'User')
}
Write-Host "Installed: $destination\coconut.exe"
Write-Host 'Open a new PowerShell window, then run: coconut setup'
