#ifndef AppVersion
  #error Supply AppVersion on the compiler command line.
#endif
#ifndef AppBinary
  #error Supply AppBinary on the compiler command line.
#endif

[Setup]
AppId={{35C81461-40B8-4FA4-A50E-1C7CC10EBEBE}
AppName=Elearnin'
AppVersion={#AppVersion}
AppPublisher=Eng-ReQTeCH
AppPublisherURL=https://github.com/Eng-ReQTeCH/Elernin-
AppSupportURL=https://github.com/Eng-ReQTeCH/Elernin-/issues
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Elearnin
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\Elearnin.ico
SetupIconFile=Elearnin.ico
OutputBaseFilename=Elearnin-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#AppBinary}"; DestDir: "{app}"; DestName: "Elearnin.exe"; Flags: ignoreversion
Source: "Elearnin.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Elearnin'"; Filename: "{app}\Elearnin.exe"; IconFilename: "{app}\Elearnin.ico"; WorkingDir: "{app}"
Name: "{autodesktop}\Elearnin'"; Filename: "{app}\Elearnin.exe"; IconFilename: "{app}\Elearnin.ico"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\Elearnin.exe"; Description: "Launch Elearnin'"; Flags: nowait postinstall skipifsilent
