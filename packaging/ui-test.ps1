# Drive the built desktop app on Windows: screenshots of each window, and timing one word in the
# timing editor end to end (play, hold Space, save), checked in the saved .timed.lrc.
#   ui-test.ps1 -Program dist\PolyKara -Out screenshots
param([string]$Program = "dist\PolyKara", [string]$Out = "screenshots")
$ErrorActionPreference = "Stop"
$Program = (Resolve-Path $Program).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$home_ = Join-Path $env:RUNNER_TEMP "polykara-ui"
if (Test-Path $home_) { Remove-Item $home_ -Recurse -Force }
New-Item -ItemType Directory -Force "$home_\media", "$home_\work\lyrics" | Out-Null

Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Drawing;
using System.Runtime.InteropServices;
public static class Ui {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
    public static void Capture(IntPtr hwnd, string path) {
        RECT rect; GetWindowRect(hwnd, out rect);
        using (var image = new Bitmap(Math.Max(1, rect.Right - rect.Left), Math.Max(1, rect.Bottom - rect.Top))) {
            using (var graphics = Graphics.FromImage(image)) { IntPtr hdc = graphics.GetHdc(); PrintWindow(hwnd, hdc, 2); graphics.ReleaseHdc(hdc); }
            image.Save(path);
        }
    }
    public static void Key(IntPtr hwnd, int key, bool down) {
        PostMessageW(hwnd, down ? 0x0100u : 0x0101u, (IntPtr)key, (IntPtr)(down ? 1 : unchecked((int)0xC0000001)));
    }
    public static void Command(IntPtr hwnd, int id) { PostMessageW(hwnd, 0x0111, (IntPtr)id, IntPtr.Zero); }
}
"@

# A demo song: 30 s of tone, two timed lines and one untimed line.
$ffmpeg = (Get-ChildItem $Program -Recurse -Filter ffmpeg.exe | Select-Object -First 1).FullName
& $ffmpeg -hide_banner -loglevel error -y -f lavfi -i "sine=frequency=440:duration=30" -ac 2 "$home_\media\demo.wav"
if ($LASTEXITCODE -ne 0) { throw "ffmpeg could not make the demo audio" }
$utf8 = New-Object System.Text.UTF8Encoding $false
[IO.File]::WriteAllText("$home_\songs.csv", "id,url,title,artist,language,template,prompt_ms,align_model`r`ndemo,media/demo.wav,演示 Demo,PolyKara,zh,solo,1000,auto`r`n", $utf8)
$lrc = "[00:02.000]<00:02.000>唤<00:02.400>不<00:02.800>醒<00:03.200>原<00:03.600>来<00:04.000>`n[00:05.000]<00:05.000>Hello <00:05.500>PolyKara <00:06.200>world<00:06.800>`n[00:08.000]重来一次`n"
[IO.File]::WriteAllText("$home_\work\lyrics\demo.timed.lrc", $lrc, $utf8)

function Start-App([string[]]$Arguments) {
    $env:POLYKARA_HOME = $home_
    $env:SDL_AUDIODRIVER = "dummy"
    $app = if ($Arguments) { Start-Process "$Program\PolyKara.exe" -ArgumentList $Arguments -PassThru } else { Start-Process "$Program\PolyKara.exe" -PassThru }
    for ($i = 0; $i -lt 60; $i++) {
        Start-Sleep -Milliseconds 500
        $main = [Ui]::FindWindowW("PolyKaraMain", $null)
        if ($main -ne [IntPtr]::Zero) { break }
        if ($app.HasExited) { throw "PolyKara.exe exited with code $($app.ExitCode)" }
    }
    if ($main -eq [IntPtr]::Zero) { throw "the PolyKara window did not open" }
    return @($app, $main)
}

function Stop-App($app) {
    if ($app.HasExited) { throw "PolyKara.exe exited early with code $($app.ExitCode)" }
    Stop-Process -Id $app.Id -Force
    Start-Sleep -Seconds 2
    Get-Process polykara-cli, ffplay -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}

# 1. The song list.
$app, $main = Start-App @()
Start-Sleep -Seconds 10
[Ui]::Capture($main, "$Out\songs.png")
Stop-App $app

# 2. Settings.
$app, $main = Start-App @("--settings")
Start-Sleep -Seconds 12
$settings = [Ui]::FindWindowW("PolyKaraDialog", $null)
if ($settings -eq [IntPtr]::Zero) { throw "the settings window did not open" }
[Ui]::Capture($settings, "$Out\settings.png")
Stop-App $app

# 3. The timing editor: play, time the first untimed word (重) by holding Space, save.
$app, $main = Start-App @("--open", "demo")
Start-Sleep -Seconds 12
[Ui]::Capture($main, "$Out\editor.png")
[Ui]::Key($main, 0x0D, $true); [Ui]::Key($main, 0x0D, $false)   # Enter: play
Start-Sleep -Milliseconds 2500
[Ui]::Key($main, 0x20, $true)                                     # Space held for 400 ms
Start-Sleep -Milliseconds 400
[Ui]::Key($main, 0x20, $false)
Start-Sleep -Milliseconds 800
[Ui]::Capture($main, "$Out\editor-playing.png")
[Ui]::Key($main, 0x1B, $true); [Ui]::Key($main, 0x1B, $false)   # Escape: pause
Start-Sleep -Milliseconds 500
[Ui]::Command($main, 202)                                         # Save
Start-Sleep -Seconds 4
Stop-App $app
$saved = [IO.File]::ReadAllText("$home_\work\lyrics\demo.timed.lrc", $utf8)
Write-Host $saved
$match = [regex]::Match($saved, "<00:0(\d)\.(\d{3})>重<00:0(\d)\.(\d{3})>")
if (-not $match.Success) { throw "the timed word was not saved" }
$start = [int]$match.Groups[1].Value * 1000 + [int]$match.Groups[2].Value
$end = [int]$match.Groups[3].Value * 1000 + [int]$match.Groups[4].Value
Write-Host "重 timed from $start ms to $end ms (held about 2500-2900 ms after play)"
if ($start -lt 1500 -or $start -gt 4500 -or ($end - $start) -lt 200 -or ($end - $start) -gt 900) { throw "the saved time of 重 is off: $start-$end ms" }
Write-Host "UI TEST OK"
