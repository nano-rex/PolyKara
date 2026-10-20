# Build dist\PolyKara (the program folder) and dist\PolyKara-Setup-<version>.exe on Windows.
# Needs Python 3.11+ on PATH; Inno Setup 6 is installed with Chocolatey when it is missing.
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
    New-Item -ItemType Directory -Force "$build\bin", "$build\defaults" | Out-Null

    python -m pip install --upgrade pip
    python -m pip install -r "$PSScriptRoot\requirements-windows.txt"
    if ($LASTEXITCODE -ne 0) { throw "pip install failed" }

    # ffmpeg, ffprobe and ffplay (timing editor playback) with libass; the shared build's programs share one set of DLLs.
    Invoke-WebRequest "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip" -OutFile "$build\ffmpeg.zip"
    Expand-Archive "$build\ffmpeg.zip" "$build\ffmpeg"
    $ffmpeg = Get-ChildItem "$build\ffmpeg" -Directory | Select-Object -First 1
    Get-ChildItem "$($ffmpeg.FullName)\bin" | Copy-Item -Destination "$build\bin"
    Copy-Item "$($ffmpeg.FullName)\LICENSE.txt" "$build\bin\FFMPEG-LICENSE.txt"
    Invoke-WebRequest "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" -OutFile "$build\bin\yt-dlp.exe"

    # First-run defaults copied to Documents\PolyKara.
    Copy-Item "$root\songs.csv", "$root\polykara.toml" "$build\defaults"

    python -m PyInstaller --noconfirm --clean --distpath "$root\dist" --workpath "$build\pyinstaller" "$PSScriptRoot\polykara.spec"
    if ($LASTEXITCODE -ne 0) { throw "PyInstaller failed" }
}

if ($Stage -eq "program") { exit 0 }
if (-not (Test-Path "$root\dist\PolyKara\PolyKara.exe")) { throw "dist\PolyKara is missing; run with -Stage program first" }

$version = (Select-String -Path "$root\polykara\__init__.py" -Pattern '__version__ = "(.+)"').Matches[0].Groups[1].Value
$iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (-not (Test-Path $iscc)) {
    choco install innosetup -y --no-progress
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup could not be installed" }
}
& $iscc "/DAppVersion=$version" "$PSScriptRoot\installer.iss"
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }
Write-Host "Built dist\PolyKara-Setup-$version.exe"
