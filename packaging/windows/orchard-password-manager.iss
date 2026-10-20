; Windows installer (Inno Setup 6). Built in CI from the staged app folder
; (packaging/stage.sh win-x64 ... dist\orchard-password-manager):
;   iscc /DAppVersion=1.0.0 packaging\windows\orchard-password-manager.iss
; Output: dist\Orchard-Password-Manager-Setup.exe
; Installs for the current user or for all users (the user chooses; /CURRENTUSER
; or /ALLUSERS on the command line), adds a Start menu entry, an optional
; desktop shortcut and an uninstaller. Silent installs:
; /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-
; Keep the AppId: later versions upgrade an installation in place with it.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{34D8C140-8A2C-4C17-9551-D4B6D33FA849}
AppName=Orchard Password Manager
AppVersion={#AppVersion}
AppVerName=Orchard Password Manager {#AppVersion}
AppPublisher=Stephan Lengl
AppPublisherURL=https://github.com/sandboxly/orchard-password-manager
AppSupportURL=https://github.com/sandboxly/orchard-password-manager/issues
AppUpdatesURL=https://github.com/sandboxly/orchard-password-manager/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\Orchard Password Manager
DefaultGroupName=Orchard Password Manager
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
LicenseFile=..\..\LICENSE
SetupIconFile=..\icons\app.ico
UninstallDisplayIcon={app}\orchard-password-manager.ico
UninstallDisplayName=Orchard Password Manager
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
OutputDir=..\..\dist
OutputBaseFilename=Orchard-Password-Manager-Setup
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\dist\orchard-password-manager\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
; The same icon as the one in the executable, as a file: shortcuts and Apps &
; features point at it.
Source: "..\icons\app.ico"; DestDir: "{app}"; DestName: "orchard-password-manager.ico"; Flags: ignoreversion

[InstallDelete]
; A new version replaces the license folder completely.
Type: filesandordirs; Name: "{app}\licenses"

[Icons]
Name: "{autoprograms}\Orchard Password Manager"; Filename: "{app}\orchard-password-manager.exe"; IconFilename: "{app}\orchard-password-manager.ico"
Name: "{autodesktop}\Orchard Password Manager"; Filename: "{app}\orchard-password-manager.exe"; IconFilename: "{app}\orchard-password-manager.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\orchard-password-manager.exe"; Description: "{cm:LaunchProgram,Orchard Password Manager}"; Flags: nowait postinstall skipifsilent
