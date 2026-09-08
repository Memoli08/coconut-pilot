# Produce the portable Windows download used by GitHub Releases.
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
Set-Location $project

cargo build --release --locked
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"$').Matches[0].Groups[1].Value
if ([string]::IsNullOrWhiteSpace($version)) { throw 'Could not read the package version.' }

$stage = Join-Path ([System.IO.Path]::GetTempPath()) ("coconut-pilot-" + [Guid]::NewGuid())
$root = Join-Path $stage "coconut-pilot-$version-windows-x86_64"
$archive = Join-Path $project "dist/coconut-pilot-$version-windows-x86_64.zip"
try {
    New-Item -ItemType Directory -Force -Path $root, (Split-Path -Parent $archive) | Out-Null
    Copy-Item target/release/coconut.exe, packaging/install.ps1, README.md, LICENSE, CHANGELOG.md, CONTRIBUTING.md, CODE_OF_CONDUCT.md, SECURITY.md, SUPPORT.md -Destination $root
    Copy-Item -Recurse assets -Destination $root
    if (Test-Path $archive) { Remove-Item -Force $archive }
    Compress-Archive -Path $root -DestinationPath $archive
    Get-FileHash -Algorithm SHA256 $archive
}
finally {
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
}
