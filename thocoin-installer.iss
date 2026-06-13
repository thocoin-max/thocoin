; ThoCoin installer (Inno Setup). Build BOTH exes first with build-release.bat:
;   cargo build --release --bin thocoin-gui
;   copy target\release\thocoin-gui.exe build\thocoin-gui-cpu.exe
;   cargo build --release --features gpu --bin thocoin-gui
;   copy target\release\thocoin-gui.exe build\thocoin-gui-gpu.exe

#define AppName "ThoCoin Wallet"
#define AppVersion "0.1.0"
#define AppExe "thocoin-gui.exe"

[Setup]
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=ThoCoin
DefaultDirName={autopf}\ThoCoin
DefaultGroupName=ThoCoin
DisableProgramGroupPage=yes
OutputDir=dist
OutputBaseFilename=ThoCoin-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=assets\logo.ico
CloseApplications=yes
CloseApplicationsFilter=*.exe
RestartApplications=no
AppMutex=ThoCoin_SingleInstance_Mutex
UninstallDisplayIcon={app}\assets\logo.ico

[Files]
; GPU edition: exe built with --features gpu + OpenCL ICD loader
Source: "build\thocoin-gui-gpu.exe"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion; Check: UseGpu
Source: "redist\OpenCL.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist; Check: UseGpu
; CPU edition: exe built without gpu feature + Mesa software GL
Source: "build\thocoin-gui-cpu.exe"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion; Check: UseCpu
Source: "opengl32.dll"; DestDir: "{app}\mesa"; Flags: ignoreversion skipifsourcedoesntexist; Check: UseCpu
Source: "libgallium_wgl.dll"; DestDir: "{app}\mesa"; Flags: ignoreversion skipifsourcedoesntexist; Check: UseCpu
Source: "libglapi.dll"; DestDir: "{app}\mesa"; Flags: ignoreversion skipifsourcedoesntexist; Check: UseCpu
; Common
Source: "assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; IconFilename: "{app}\assets\logo.ico"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; IconFilename: "{app}\assets\logo.ico"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Optional:"

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch ThoCoin Wallet"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: files; Name: "{app}\render.cfg"

[Code]
var
  RenderPage: TInputOptionWizardPage;

procedure InitializeWizard;
begin
  RenderPage := CreateInputOptionPage(wpSelectTasks,
    'Edition', 'Choose your ThoCoin edition',
    'CPU edition works on every computer (recommended for VMs/old PCs). ' +
    'GPU edition enables GPU mining and needs a modern graphics driver with OpenCL.',
    True, False);
  RenderPage.Add('CPU edition - CPU mining only, software rendering, works everywhere');
  RenderPage.Add('GPU edition - GPU + CPU mining, hardware rendering');
  RenderPage.SelectedValueIndex := 0;
end;

function UseCpu(): Boolean;
begin
  Result := RenderPage.SelectedValueIndex = 0;
end;

function UseGpu(): Boolean;
begin
  Result := not UseCpu();
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Mode: String;
begin
  if CurStep = ssPostInstall then
  begin
    if UseCpu() then Mode := 'cpu' else Mode := 'gpu';
    SaveStringToFile(ExpandConstant('{app}\render.cfg'), Mode, False);
    DeleteFile(ExpandConstant('{app}\opengl32.dll'));
    DeleteFile(ExpandConstant('{app}\libgallium_wgl.dll'));
    DeleteFile(ExpandConstant('{app}\libglapi.dll'));
  end;
end;
