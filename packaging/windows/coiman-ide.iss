; Setup program for Coiman IDE on Windows, made with Inno Setup 6:
;   ISCC /DAppVersion=0.7.0 /DPayload=<folder with coiman-ide.exe> /O<output folder> coiman-ide.iss
; The payload is the same folder the release zips (the program, its documents and the
; extensions that come with it).
;
; It installs for the current user by default (no administrator rights needed), or for
; everyone when chosen. Running the setup of a newer version replaces the older one in place:
; the AppId below never changes, so Windows sees one program. Settings in %APPDATA%\coiman-ide
; are never touched, by installing, updating or uninstalling.

#ifndef AppVersion
  #error Pass the version: /DAppVersion=0.7.0
#endif
#ifndef Payload
  #error Pass the folder to install: /DPayload=dist\coiman-ide-0.7.0-windows-x64
#endif

[Setup]
AppId={{173E221E-EB7A-4B40-8017-BF2081138B47}
AppName=Coiman IDE
AppVersion={#AppVersion}
AppVerName=Coiman IDE {#AppVersion}
AppPublisher=Manuel Pereira
AppPublisherURL=https://github.com/ManuelCoimbra/coiman-ide
AppSupportURL=https://github.com/ManuelCoimbra/coiman-ide/issues
AppUpdatesURL=https://github.com/ManuelCoimbra/coiman-ide/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\Coiman IDE
DefaultGroupName=Coiman IDE
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename=coiman-ide-{#AppVersion}-windows-x64-setup
SetupIconFile=..\..\assets\icon\coiman.ico
UninstallDisplayIcon={app}\coiman-ide.exe
UninstallDisplayName=Coiman IDE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; A running Coiman is asked to close before its files are replaced.
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[InstallDelete]
; Extensions that a newer version no longer ships don't stay behind.
Type: filesandordirs; Name: "{app}\extensions"

[Files]
Source: "{#Payload}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Coiman IDE"; Filename: "{app}\coiman-ide.exe"
Name: "{autodesktop}\Coiman IDE"; Filename: "{app}\coiman-ide.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\coiman-ide.exe"; Description: "{cm:LaunchProgram,Coiman IDE}"; Flags: nowait postinstall skipifsilent
