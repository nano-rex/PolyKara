# Download what PolyKara needs but does not keep in git: the dictionaries behind
# readings and romanization, whisper.cpp (word timing) and a ggml model.
# Pinned commits and SHA-256 make the build reproducible.
#   -OutDir   dictionaries (data\ in the program folder)
#   -BinDir   whisper-cli.exe and its DLLs (bin\)
#   -ModelDir ggml-<model>.bin (models\)
#   -Model    small (default), base, medium, large-v3, tiny, or a file name
param(
    [string]$OutDir = (Join-Path $PSScriptRoot "build\data"),
    [string]$BinDir = (Join-Path $PSScriptRoot "build\bin"),
    [string]$ModelDir = (Join-Path $PSScriptRoot "build\models"),
    [string]$Model = "small",
    [switch]$SkipModel
)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
New-Item -ItemType Directory -Force $OutDir, $BinDir, $ModelDir | Out-Null

function Get-CheckedFile([string]$Url, [string]$Target, [string]$Sha256) {
    if (Test-Path $Target) {
        $have = (Get-FileHash $Target -Algorithm SHA256).Hash.ToLower()
        if ($have -eq $Sha256) { Write-Host "ok $Target"; return }
    }
    Write-Host "get $Url"
    Invoke-WebRequest $Url -OutFile $Target
    $have = (Get-FileHash $Target -Algorithm SHA256).Hash.ToLower()
    if ($have -ne $Sha256) {
        Remove-Item $Target -Force
        throw "$Target has the wrong SHA-256: $have (wanted $Sha256)"
    }
}

# pinyin-data and phrase-pinyin-data (MIT), ToJyutping's trie (BSD-2-Clause),
# pykakasi's kakasi dictionaries (GPL-3.0), OpenCC TSCharacters (Apache-2.0).
$files = @(
    @{ Name = "pinyin.txt"; Url = "https://raw.githubusercontent.com/mozillazg/pinyin-data/9d05c9e29cb6a7e5c79f2f5279f5c1014246a19a/pinyin.txt"; Sha = "90c11bd680ccfe7f2617a6e25abdb1fb87b5540df3e6f011ca50aaf3c7263a89" },
    @{ Name = "phrases.txt"; Url = "https://raw.githubusercontent.com/mozillazg/phrase-pinyin-data/cee0ed6e6e4898580cafd2bd5e3723e20b214aa0/pinyin.txt"; Sha = "dcc769607c220b312fea3e71cb63421298b4b891b1f7356a95ab58f2c96fff81" },
    @{ Name = "jyutping-trie.txt"; Url = "https://raw.githubusercontent.com/CanCLID/ToJyutping/a692787e9af65b27310580604b1d69b96b2165be/src/ToJyutping/trie.txt"; Sha = "4995ec440243d468a4c816697e7fd9a87b8efbb90e54db119deb78c829a87729" },
    @{ Name = "hepburnhira.utf8"; Url = "https://raw.githubusercontent.com/miurahr/pykakasi/717c52850d240f33b5b87f3a6fa021e07c9c82b5/src/data/hepburnhira.utf8"; Sha = "6f3b3694eef8512123c5132096b076fd8c0d41de80ef306026c2a607061ff568" },
    @{ Name = "itaijidict.utf8"; Url = "https://raw.githubusercontent.com/miurahr/pykakasi/717c52850d240f33b5b87f3a6fa021e07c9c82b5/src/data/itaijidict.utf8"; Sha = "890183324709ed25d834a2b2f51b903876218301df92c9be38f0ae4da4205c44" },
    @{ Name = "TSCharacters.txt"; Url = "https://raw.githubusercontent.com/BYVoid/OpenCC/bcf847cf3fbd79c74779567f9a491ea8ab878183/data/dictionary/TSCharacters.txt"; Sha = "9ff46a7d30e5765375eb13d33f2b03a34d298913caf2b120380679f33ae1642d" }
)
foreach ($file in $files) {
    Get-CheckedFile $file.Url (Join-Path $OutDir $file.Name) $file.Sha
}

# whisper.cpp: whisper-cli.exe and the ggml/whisper DLLs it needs.
$whisper = @{
    Version = "v1.8.2"
    Url     = "https://github.com/ggml-org/whisper.cpp/releases/download/v1.8.2/whisper-bin-x64.zip"
    Sha     = "b1514ebc099765e39fa37eb780b92a140a94c86bb0b3b3d98226b38825979732"
}
$zip = Join-Path ([System.IO.Path]::GetTempPath()) "whisper-bin-x64.zip"
if (-not (Test-Path $zip) -or (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower() -ne $whisper.Sha) {
    Invoke-WebRequest $whisper.Url -OutFile $zip
    $have = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    if ($have -ne $whisper.Sha) { throw "whisper-bin-x64.zip has the wrong SHA-256: $have" }
}
$unpack = Join-Path ([System.IO.Path]::GetTempPath()) "whisper-bin-x64"
if (Test-Path $unpack) { Remove-Item $unpack -Recurse -Force }
Expand-Archive $zip $unpack
Copy-Item "$unpack\Release\whisper-cli.exe" $BinDir -Force
Get-ChildItem "$unpack\Release" -Filter *.dll | Copy-Item -Destination $BinDir -Force
Write-Host "whisper.cpp $($whisper.Version) -> $BinDir"

if (-not $SkipModel) {
    $name = if ($Model.EndsWith(".bin")) { $Model } else { "ggml-$Model.bin" }
    $url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/$name"
    $target = Join-Path $ModelDir $name
    # Only the default model is pinned; other sizes are for people who want them.
    $pinned = @{ "ggml-small.bin" = "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b" }
    if ($pinned.ContainsKey($name)) {
        Get-CheckedFile $url $target $pinned[$name]
    } elseif (-not (Test-Path $target)) {
        Write-Host "get $url (large download)"
        Invoke-WebRequest $url -OutFile $target
    }
    Write-Host "model $target"
}
