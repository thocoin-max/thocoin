; ThoCoin Installer — Inno Setup script
; Build:
;   1. cargo build --release
;   2. Mở Inno Setup Compiler → File → Open → chọn file này → Build → Compile
;   3. File output: D:\thocoin\installer\ThoCoin-Setup-0.1.0.exe

#define MyAppName        "ThoCoin"
#define MyAppVersion     "0.1.0"
#define MyAppPublisher   "ThoCoin"
#define MyAppExeName     "thocoin-gui.exe"
#define MyAppDaemonName  "thocoind.exe"

[Setup]
AppId={{8B7C2D4A-5E3F-4A92-9C1B-22D9F8E1A777}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=installer
OutputBaseFilename=ThoCoin-Setup-{#MyAppVersion}
SetupIconFile=assets\logo.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/ultra
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: checkedonce

[Files]
Source: "target\release\{#MyAppExeName}";    DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\{#MyAppDaemonName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "assets\logo.png";                   DestDir: "{app}\assets"; Flags: ignoreversion
Source: "assets\logo.ico";                   DestDir: "{app}\assets"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName} Wallet"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\assets\logo.ico"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName} Wallet"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\assets\logo.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch ThoCoin Wallet"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Không xóa wallet/data — user phải tự xóa nếu muốn
; Để xóa luôn data khi uninstall, uncomment dòng dưới:
; Type: filesandordirs; Name: "{userappdata}\ThoCoin"
