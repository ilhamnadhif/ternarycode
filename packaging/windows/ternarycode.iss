; Inno Setup script for the Windows installer (TernaryCode-<version>-windows-x64-setup.exe).
;
;   iscc /DAppVersion=1.4.0 /DSourceExe=..\..\target\release\ternarycode.exe /DOutputDir=..\..\dist packaging\windows\ternarycode.iss
;
; Installs for the current user by default (no administrator rights needed), or for all users
; when chosen in the first dialog. An update installs over the previous version (same AppId)
; and closes a running TernaryCode first. The in-app updater runs it with /SILENT /update=1, which
; opens TernaryCode again when it is done.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\ternarycode.exe"
#endif
#ifndef SourceHook
  #define SourceHook "..\..\target\release\ternarycode-hook.exe"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
AppId={{A0EB1C5A-F267-4813-A91D-ED51FE173017}
AppName=TernaryCode
AppVersion={#AppVersion}
AppVerName=TernaryCode {#AppVersion}
AppPublisher=TernaryCode contributors
AppPublisherURL=https://github.com/ilhamnadhif/ternarycode
AppSupportURL=https://github.com/ilhamnadhif/ternarycode/issues
AppUpdatesURL=https://github.com/ilhamnadhif/ternarycode/releases
DefaultDirName={autopf}\TernaryCode
DefaultGroupName=TernaryCode
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE
SetupIconFile=ternarycode.ico
UninstallDisplayIcon={app}\ternarycode.exe
UninstallDisplayName=TernaryCode
OutputDir={#OutputDir}
OutputBaseFilename=TernaryCode-{#AppVersion}-windows-x64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
ChangesEnvironment=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "addtopath"; Description: "Add ternarycode to PATH (open a project from a terminal: ternarycode .)"; GroupDescription: "Command line:"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "ternarycode.exe"; Flags: ignoreversion
; Agents run it from their hooks; the app finds it beside itself.
Source: "{#SourceHook}"; DestDir: "{app}"; DestName: "ternarycode-hook.exe"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\TernaryCode"; Filename: "{app}\ternarycode.exe"
Name: "{autodesktop}\TernaryCode"; Filename: "{app}\ternarycode.exe"; Tasks: desktopicon

[Registry]
; ternarycode.exe on PATH (per user, or for all users in an administrative install).
Root: HKA; Subkey: "{code:EnvKey}"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; \
  Check: NeedsAddPath(ExpandConstant('{app}')); Tasks: addtopath

[Run]
Filename: "{app}\ternarycode.exe"; Description: "{cm:LaunchProgram,TernaryCode}"; Flags: nowait postinstall skipifsilent
; After an update from the app: open TernaryCode again, as the user (not elevated).
Filename: "{app}\ternarycode.exe"; Flags: nowait runasoriginaluser; Check: IsAppUpdate

[Code]
function IsAppUpdate: Boolean;
begin
  Result := ExpandConstant('{param:update|0}') = '1';
end;

function EnvKey(Param: String): String;
begin
  if IsAdminInstallMode then
    Result := 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
  else
    Result := 'Environment';
end;

function NeedsAddPath(Dir: String): Boolean;
var
  Paths: String;
  RootKey: Integer;
begin
  if IsAdminInstallMode then RootKey := HKEY_LOCAL_MACHINE else RootKey := HKEY_CURRENT_USER;
  if not RegQueryStringValue(RootKey, EnvKey(''), 'Path', Paths) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Paths) + ';') = 0;
end;
