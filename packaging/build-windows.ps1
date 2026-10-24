# Build dist\PolyKara (the program folder) and dist\PolyKara-Setup-<version>.exe on Windows.
# Needs Rust (cargo) on PATH; Inno Setup 6 is installed with Chocolatey when it is missing.
# Everything else (ffmpeg, yt-dlp, whisper.cpp, the model, the dictionaries) is downloaded.
#   -Stage program    only the program folder (so its programs can be signed first)
#   -Stage installer  only the installer, from an existing dist\PolyKara
#   -Stage all        both (default)
param([ValidateSet("all", "program", "installer")][string]$Stage = "all")
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$root = Split-Path $PSScriptRoot -Parent
$build = Join-Path $PSScriptRoot "build"

if ($Stage -ne "installer") {
    if (Test-Path $build) { Remove-Item $build -Recurse -Force }
    New-Item -ItemType Directory -Force "$build\bin", "$build\defaults", "$build\data", "$build\models" | Out-Null

    # ffmpeg, ffprobe and ffplay (timing editor playback) with libass; the shared build's programs share one set of DLLs.
    Invoke-WebRequest "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip" -OutFile "$build\ffmpeg.zip"
    Expand-Archive "$build\ffmpeg.zip" "$build\ffmpeg"
    $ffmpeg = Get-ChildItem "$build\ffmpeg" -Directory | Select-Object -First 1
    Get-ChildItem "$($ffmpeg.FullName)\bin" | Copy-Item -Destination "$build\bin"
    Copy-Item "$($ffmpeg.FullName)\LICENSE.txt" "$build\bin\FFMPEG-LICENSE.txt"
    Invoke-WebRequest "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" -OutFile "$build\bin\yt-dlp.exe"

    # whisper-cli and its model, plus the readings/romanization dictionaries.
    & "$PSScriptRoot\fetch-data.ps1" -OutDir "$build\data" -BinDir "$build\bin" -ModelDir "$build\models"
    if ($LASTEXITCODE -ne 0) { throw "fetch-data failed" }

    # First-run defaults copied to Documents\PolyKara.
    Copy-Item "$root\songs.csv", "$root\polykara.toml" "$build\defaults"

    # Both programs are Rust; no Python is built or shipped.
    cargo build --release --locked --manifest-path "$root\Cargo.toml"
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $dist = Join-Path $root "dist\PolyKara"
    if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
    New-Item -ItemType Directory -Force $dist | Out-Null
    Copy-Item "$root\target\release\PolyKara.exe", "$root\target\release\polykara-cli.exe" $dist
    Copy-Item "$build\bin", "$build\data", "$build\models", "$build\defaults" $dist -Recurse
    Copy-Item "$root\readings", "$root\web" $dist -Recurse
    Copy-Item "$root\LICENSE", "$root\README.md" $dist
}

if ($Stage -eq "program") { exit 0 }
if (-not (Test-Path "$root\dist\PolyKara\PolyKara.exe")) { throw "dist\PolyKara is missing; run with -Stage program first" }

$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (-not (Test-Path $iscc)) {
    choco install innosetup -y --no-progress
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup could not be installed" }
}
& $iscc "/DAppVersion=$version" "$PSScriptRoot\installer.iss"
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }
Write-Host "Built dist\PolyKara-Setup-$version.exe"
