# Build dist\PolyKara (the program folder) and dist\PolyKara-Setup-<version>.exe on Windows.
# Needs Python 3.11+ on PATH; Inno Setup 6 is installed with Chocolatey when it is missing.
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$root = Split-Path $PSScriptRoot -Parent
$build = Join-Path $PSScriptRoot "build"
if (Test-Path $build) { Remove-Item $build -Recurse -Force }
New-Item -ItemType Directory -Force "$build\bin", "$build\defaults" | Out-Null

python -m pip install --upgrade pip
python -m pip install -r "$PSScriptRoot\requirements-windows.txt"
if ($LASTEXITCODE -ne 0) { throw "pip install failed" }

# ffmpeg and ffprobe with libass (shared build: the two programs share one set of DLLs).
Invoke-WebRequest "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip" -OutFile "$build\ffmpeg.zip"
Expand-Archive "$build\ffmpeg.zip" "$build\ffmpeg"
$ffmpeg = Get-ChildItem "$build\ffmpeg" -Directory | Select-Object -First 1
Get-ChildItem "$($ffmpeg.FullName)\bin" | Where-Object { $_.Name -ne "ffplay.exe" } | Copy-Item -Destination "$build\bin"
Copy-Item "$($ffmpeg.FullName)\LICENSE.txt" "$build\bin\FFMPEG-LICENSE.txt"
Invoke-WebRequest "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" -OutFile "$build\bin\yt-dlp.exe"

# First-run defaults copied to Documents\PolyKara.
Copy-Item "$root\songs.csv", "$root\polykara.toml" "$build\defaults"
Copy-Item "$root\lyrics" "$build\defaults\lyrics" -Recurse

python -m PyInstaller --noconfirm --clean --distpath "$root\dist" --workpath "$build\pyinstaller" "$PSScriptRoot\polykara.spec"
if ($LASTEXITCODE -ne 0) { throw "PyInstaller failed" }

$version = (Select-String -Path "$root\polykara\__init__.py" -Pattern '__version__ = "(.+)"').Matches[0].Groups[1].Value
$iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (-not (Test-Path $iscc)) {
    choco install innosetup -y --no-progress
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup could not be installed" }
}
& $iscc "/DAppVersion=$version" "$PSScriptRoot\installer.iss"
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }
Write-Host "Built dist\PolyKara-Setup-$version.exe"
