; The setup program for Windows: zeno-<version>-windows-setup.exe.
;
; This is a script for Inno Setup (https://jrsoftware.org/isinfo.php),
; which makes one .exe from it. build.cmd runs it:
;
;     tools\installer\build.cmd 0.2.0 files
;
; `files` is a folder laid out like this, and the release workflow
; fills it from the three Windows builds:
;
;     files\x86_64\zeno.exe     at least one of these three
;     files\x86\zeno.exe
;     files\arm64\zeno.exe
;     files\zeno.wasm           Zeno built for a browser
;     files\README.md  files\docs  files\examples  files\editors
;
; What the setup program does, on the machine it is run on:
;
;   - copies the `zeno.exe` for that machine's processor, `zeno.wasm`,
;     the docs, the examples and the editor setup into
;     %LOCALAPPDATA%\Programs\Zeno. That folder is the user's own, so
;     nothing asks for an administrator.
;   - adds that folder to the user's PATH, so that `zeno` works in any
;     terminal opened afterwards.
;   - offers to install the two programs that parts of Zeno hand work
;     to and that are not on this machine: a C compiler (`zeno build`)
;     and Git (`zeno fetch`). Windows' own package manager, winget,
;     downloads and installs them.
;   - adds an uninstaller, which takes all of the first two back.
;
; docs/install.md walks through it.

#ifndef Version
  #error Say which version this is: ISCC /DVersion=0.2.0 /DFiles=files zeno.iss
#endif
#ifndef Files
  #error Say where the files are: ISCC /DVersion=0.2.0 /DFiles=files zeno.iss
#endif

; Which builds of `zeno.exe` the folder has. A trial may have only one.
#define Kinds ""
#if FileExists(AddBackslash(Files) + "x86_64\zeno.exe")
  #define Kinds Kinds + " x86_64"
#endif
#if FileExists(AddBackslash(Files) + "x86\zeno.exe")
  #define Kinds Kinds + " x86"
#endif
#if FileExists(AddBackslash(Files) + "arm64\zeno.exe")
  #define Kinds Kinds + " arm64"
#endif
#if Kinds == ""
  #error There is no zeno.exe in the folder of files
#endif

[Setup]
; This says which program an installer is for. A later version has the
; same one, and so is installed over the earlier one and not beside it.
AppId={{BFA50675-706F-4C99-A355-0C786FDB36B9}
AppName=Zeno
AppVersion={#Version}
AppVerName=Zeno {#Version}
AppPublisher=The Zeno language
AppPublisherURL=https://zeno-lang.org
AppSupportURL=https://zeno-lang.org
VersionInfoVersion={#Version}
; For this user only, in a folder that is theirs: no administrator.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Zeno
DefaultGroupName=Zeno
DisableProgramGroupPage=yes
; The PATH is changed, and programs that are running should be told.
ChangesEnvironment=yes
OutputBaseFilename=zeno-{#Version}-windows-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName=Zeno {#Version}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "path"; Description: "Add Zeno to the PATH, so that a terminal finds `zeno`"
; These two are offered only when the program is missing and winget is
; there to install it.
Name: "compiler"; Description: "Install a C compiler (gcc and clang, from WinLibs: a download of about 300 MB). `zeno build` needs one"; GroupDescription: "Programs that Zeno hands work to, and that this computer does not have:"; Check: CanInstall('gcc.exe') and CanInstall('clang.exe') and CanInstall('cc.exe')
Name: "git"; Description: "Install Git. `zeno fetch` needs it, to get packages"; GroupDescription: "Programs that Zeno hands work to, and that this computer does not have:"; Check: CanInstall('git.exe')

[Files]
; One `zeno.exe`: the one built for this machine's processor.
#if FileExists(AddBackslash(Files) + "x86_64\zeno.exe")
Source: "{#Files}\x86_64\zeno.exe"; DestDir: "{app}"; Flags: ignoreversion; Check: Installs('x86_64')
#endif
#if FileExists(AddBackslash(Files) + "x86\zeno.exe")
Source: "{#Files}\x86\zeno.exe"; DestDir: "{app}"; Flags: ignoreversion; Check: Installs('x86')
#endif
#if FileExists(AddBackslash(Files) + "arm64\zeno.exe")
Source: "{#Files}\arm64\zeno.exe"; DestDir: "{app}"; Flags: ignoreversion; Check: Installs('arm64')
#endif
; Zeno built for a browser. `zeno build --web` looks for it beside zeno.exe.
Source: "{#Files}\zeno.wasm"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Files}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Files}\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Files}\examples\*"; DestDir: "{app}\examples"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Files}\editors\*"; DestDir: "{app}\editors"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
; In the Start menu. The first opens a terminal in which `zeno` works
; even if the PATH was left alone.
Name: "{group}\Zeno Prompt"; Filename: "{cmd}"; Parameters: "/k ""set ""PATH={app};%PATH%"" && zeno --version"""; WorkingDir: "{userdocs}"
Name: "{group}\Zeno examples"; Filename: "{app}\examples"
Name: "{group}\Zeno docs"; Filename: "{app}\docs"
Name: "{group}\Uninstall Zeno"; Filename: "{uninstallexe}"

[Run]
; winget shows what it is doing in a window of its own.
Filename: "{cmd}"; Parameters: "/c winget install -e --id BrechtSanders.WinLibs.POSIX.UCRT.LLVM --accept-package-agreements --accept-source-agreements"; StatusMsg: "Installing a C compiler. This is a large download..."; Tasks: compiler
Filename: "{cmd}"; Parameters: "/c winget install -e --id Git.Git --accept-package-agreements --accept-source-agreements"; StatusMsg: "Installing Git..."; Tasks: git
Filename: "{app}\examples"; Description: "Open the folder of examples"; Flags: postinstall shellexec skipifsilent unchecked

[Code]
const
  { Where Windows keeps this user's own environment variables. }
  Environment = 'Environment';

{ Is a program of this name on the PATH? }
function OnPath(Name: string): Boolean;
begin
  Result := FileSearch(Name, GetEnv('PATH')) <> '';
end;

{ Should installing this program be offered? Only if it is missing,
  and winget is there to install it. }
function CanInstall(Name: string): Boolean;
begin
  Result := (not OnPath(Name)) and OnPath('winget.exe');
end;

{ Was a `zeno.exe` for this kind of processor put into the setup? }
function Has(Kind: string): Boolean;
begin
  Result := Pos(' ' + Kind + ' ', '{#Kinds} ') > 0;
end;

{ The `zeno.exe` this machine gets. Its own kind if there is one. A
  64-bit machine also runs the one for 32-bit x86, and Windows on ARM
  runs both of the x86 ones, more slowly. Empty if nothing here runs. }
function Chosen: string;
begin
  Result := '';
  case ProcessorArchitecture of
    paArm64:
      if Has('arm64') then Result := 'arm64'
      else if Has('x86_64') then Result := 'x86_64'
      else if Has('x86') then Result := 'x86';
    paX64:
      if Has('x86_64') then Result := 'x86_64'
      else if Has('x86') then Result := 'x86';
    paX86:
      if Has('x86') then Result := 'x86';
  end;
end;

function Installs(Kind: string): Boolean;
begin
  Result := Chosen = Kind;
end;

function InitializeSetup: Boolean;
begin
  Result := Chosen <> '';
  if not Result then
    MsgBox('This setup program has no Zeno for this computer''s processor. Zeno can be built from its source: see zeno-lang.org.', mbError, MB_OK);
end;

{ The user's PATH with `Dir` added at its end, or as it is if `Dir` is
  already in it. }
procedure AddToPath(Dir: string);
var
  Paths: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, Environment, 'Path', Paths) then
    Paths := '';
  if Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Paths) + ';') > 0 then
    exit;
  if (Paths <> '') and (Paths[Length(Paths)] <> ';') then
    Paths := Paths + ';';
  RegWriteExpandStringValue(HKEY_CURRENT_USER, Environment, 'Path', Paths + Dir);
end;

{ The user's PATH with `Dir` taken out again. }
procedure RemoveFromPath(Dir: string);
var
  Paths: string;
  At: Integer;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, Environment, 'Path', Paths) then
    exit;
  { With a `;` at each end, every folder in it is `;folder;`. }
  Paths := ';' + Paths + ';';
  At := Pos(';' + Uppercase(Dir) + ';', Uppercase(Paths));
  if At = 0 then
    exit;
  Delete(Paths, At, Length(Dir) + 1);
  RegWriteExpandStringValue(HKEY_CURRENT_USER, Environment, 'Path', Copy(Paths, 2, Length(Paths) - 2));
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('path') then
    AddToPath(ExpandConstant('{app}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
