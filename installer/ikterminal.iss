; Inno Setup 6 script. scripts\build-installer.ps1 passes AppVersion from Cargo.toml.
#ifndef AppVersion
  #error AppVersion is required. Build with scripts\build-installer.ps1.
#endif

#define AppName "IkTerminal"
#define AppExe "ikterminal.exe"

[Setup]
AppId={{6A1C1E52-7F3B-4C8E-9C1D-3E2B8B6F4A10}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppName}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\installer
OutputBaseFilename={#AppName}-{#AppVersion}-setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; DestName: "ikt.exe"; Flags: ignoreversion

[Registry]
; Explorer and the Run dialog resolve `ikt` without a PATH lookup.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\ikt.exe"; ValueType: string; ValueName: ""; ValueData: "{app}\ikt.exe"; Flags: uninsdeletekey

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  EnvironmentKey = 'Environment';
  WM_SETTINGCHANGE = $001A;
  SMTO_ABORTIFHUNG = 2;

function SendMessageTimeout(hWnd: Integer; Msg: Cardinal; wParam: Integer; lParam: string; fuFlags: Cardinal; uTimeout: Cardinal; var lpdwResult: Integer): Integer;
  external 'SendMessageTimeoutW@user32.dll stdcall';

function NeedsAddPath(Param: string): Boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', OrigPath) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Param) + ';', ';' + Uppercase(OrigPath) + ';') = 0;
end;

procedure RefreshEnvironment;
var
  Dummy: Integer;
begin
  { Tell Explorer and other programs to reload PATH so `ikt` works without a new login. }
  SendMessageTimeout(HWND_BROADCAST, WM_SETTINGCHANGE, 0, 'Environment', SMTO_ABORTIFHUNG, 5000, Dummy);
end;

procedure AddToPath(Path: string);
var
  Paths: string;
begin
  if not NeedsAddPath(Path) then
    exit;
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Paths) then
    Paths := '';
  if Paths = '' then
    Paths := Path
  else
    Paths := Paths + ';' + Path;
  RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Paths);
end;

function RemoveFromPathList(const Paths, Path: string): string;
var
  Rest, Item, Built: string;
  P: Integer;
begin
  Rest := Paths;
  Built := '';
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    if P = 0 then
    begin
      Item := Rest;
      Rest := '';
    end
    else
    begin
      Item := Copy(Rest, 1, P - 1);
      Rest := Copy(Rest, P + 1, MaxInt);
    end;
    if (Item <> '') and (CompareText(Item, Path) <> 0) then
    begin
      if Built = '' then
        Built := Item
      else
        Built := Built + ';' + Item;
    end;
  end;
  Result := Built;
end;

procedure RemoveFromPath(Path: string);
var
  Paths: string;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Paths) then
    exit;
  Paths := RemoveFromPathList(Paths, Path);
  RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Paths);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    AddToPath(ExpandConstant('{app}'));
    RefreshEnvironment;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    RemoveFromPath(ExpandConstant('{app}'));
    RefreshEnvironment;
  end;
end;
