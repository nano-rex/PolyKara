; Inno Setup script for the PolyKara installer. build-windows.ps1 passes /DAppVersion=...
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{4C7E2B1A-9D3F-4E8B-A6C1-5F2D7B9E0A34}
AppName=PolyKara
AppVersion={#AppVersion}
AppPublisher=PolyKara
; Installed for the current user, so the bundled yt-dlp can update itself without admin rights.
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\PolyKara
DefaultGroupName=PolyKara
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename=PolyKara-Setup-{#AppVersion}
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
UninstallDisplayIcon={app}\PolyKara.exe

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "..\dist\PolyKara\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{group}\PolyKara"; Filename: "{app}\PolyKara.exe"
Name: "{group}\PolyKara command line"; Filename: "{cmd}"; Parameters: "/k ""{app}\polykara-cli.exe"" --help"
Name: "{group}\Uninstall PolyKara"; Filename: "{uninstallexe}"
Name: "{autodesktop}\PolyKara"; Filename: "{app}\PolyKara.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\PolyKara.exe"; Description: "Start PolyKara"; Flags: nowait postinstall skipifsilent
