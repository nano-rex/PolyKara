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
    # A script call does not set $LASTEXITCODE; fetch-data throws on any failure, and
    # with $ErrorActionPreference = "Stop" that stops this build too.
    & "$PSScriptRoot\fetch-data.ps1" -OutDir "$build\data" -BinDir "$build\bin" -ModelDir "$build\models"

    # Vocal separator for `generate`: stems.cpp (HTDemucs v4 on ggml, MIT), built from
    # source with Visual Studio, plus its GGUF model. Best-effort: a separator build
    # problem warns but never blocks the installer (Generate lyrics then needs a
    # separator on PATH). Set POLYKARA_SKIP_SEPARATOR=1 to skip it.
    if (-not $env:POLYKARA_SKIP_SEPARATOR) {
        try {
            $stems = Join-Path $build "stems.cpp"
            git clone --recurse-submodules https://github.com/tinycrops/stems.cpp $stems
            if ($LASTEXITCODE -ne 0) { throw "git clone stems.cpp failed" }
            Push-Location $stems
            try { & cmd /c "build.cmd cpu"; $built = $LASTEXITCODE } finally { Pop-Location }
            if ($built -ne 0) { throw "stems.cpp build.cmd failed ($built)" }
            $exe = Join-Path $stems "build\bin\Release\stems-split.exe"
            if (-not (Test-Path $exe)) { throw "stems-split.exe not found at $exe" }
            Copy-Item $exe "$build\bin"
            # Any runtime DLLs the build placed beside it (ggml backends, stems.dll).
            Get-ChildItem "$stems\build\bin\Release" -Filter *.dll -ErrorAction SilentlyContinue | Copy-Item -Destination "$build\bin"
            Copy-Item "$stems\LICENSE" "$build\bin\STEMS-LICENSE.txt" -ErrorAction SilentlyContinue
            # The HTDemucs GGUF model (a re-encode of demucs' MIT checkpoints).
            Invoke-WebRequest "https://github.com/tinycrops/stems.cpp/releases/download/models-v1/htdemucs-f32.gguf" -OutFile "$build\models\htdemucs-f32.gguf"
            Write-Host "stems.cpp separator bundled ($((Get-Item "$build\bin\stems-split.exe").Length) bytes)"
        } catch {
            Write-Warning "separator (stems.cpp) not bundled: $_. 'Generate lyrics' will need a separator on PATH."
        }
    }

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
