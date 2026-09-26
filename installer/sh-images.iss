; Sh Images — Windows installer (Inno Setup 6)
;
; Update model: there is no in-app updater. A user upgrades by running a NEW
; installer over the EXISTING installation. Everything below exists to make
; that in-place upgrade predictable for a non-technical user.
;
; User data lives OUTSIDE the install directory, in %APPDATA%\sh_images
; (see crates/sh-app/src/main.rs :: config_dir). Install and uninstall must
; therefore never read, move or delete anything under that path — the user's
; theme and settings survive every upgrade untouched. That is also why the
; install directory defaults to {localappdata}\Programs\Sh Images: it is
; disposable program files, not state.
;
; Requires Inno Setup 6.3 or newer (ArchitecturesAllowed=x64compatible).
; Compile from the repository root so relative Source: paths resolve:
;   ISCC.exe installer\sh-images.iss /DAppVersion=0.1.0

#ifndef AppVersion
  ; Local default. MUST stay in sync with [workspace.package] version in the
  ; root Cargo.toml so a compile without /DAppVersion still stamps correctly.
  ; CI always overrides it with /DAppVersion=<parsed from Cargo.toml>, so this
  ; literal is a developer convenience, never the source of truth.
  #define AppVersion "0.1.0"
#endif

#define AppExeName "sh-app.exe"
#define AppRepoURL "https://github.com/Shoking01/Sh_Images-Reborn"

[Setup]
; FIXED LITERAL GUID — THIS VALUE MUST NEVER CHANGE.
; AppId is the upgrade identity, not a cosmetic identifier. Inno matches an
; existing installation by AppId + install directory: keep this GUID stable and
; a new installer recognises the old install, runs its uninstaller first, and
; replaces the program files in place while leaving user data alone. Change
; this value and every existing user ends up with a second, conflicting copy
; and two uninstall entries in Add/Remove Programs. If a GUID is ever truly
; required to change, that is a new product and needs a migration plan, not a
; find-and-replace.
AppId=2ABBD0F1-2CF2-4B42-A948-3E35666D70AD

AppName=Sh Images
; Explicit on purpose. When DefaultGroupName is left unset, Inno falls back to a
; group literally named "(Default)", so the Start Menu entry would read
; "(Default)\Sh Images" — meaningless to a non-technical user. Naming it here
; makes {group} resolve to "Sh Images".
DefaultGroupName=Sh Images
AppVersion={#AppVersion}
AppVerName=Sh Images {#AppVersion}
AppPublisher=Sh Images
AppPublisherURL={#AppRepoURL}
AppSupportURL={#AppRepoURL}/issues
AppUpdatesURL={#AppRepoURL}/releases

; Per-user install under the user's own profile. No administrator rights, no
; UAC elevation prompt, no machine-wide Program Files write — the single
; biggest barrier to a non-technical user running an installer at all.
DefaultDirName={localappdata}\Programs\Sh Images
PrivilegesRequired=lowest

; The MIT text is shown during setup AND installed next to the executable, so
; the licence travels with the software instead of only being a repo file.
LicenseFile=..\LICENSE

SetupIconFile=..\assets\branding\sh-images.ico
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName=Sh Images

Uninstallable=yes
; Write the Add/Remove Programs entry. Without it a non-technical user has no
; discoverable, supported way to remove the app.
CreateUninstallRegKey=yes

; Restart Manager detects a running viewer and closes it so the upgrade can
; replace the locked executable. RestartApplications=no suppresses ONLY the
; Restart Manager relaunch: we do not silently respawn an app the user closed.
; Relaunching after an upgrade is still offered, but explicitly, by the
; post-install checkbox on the final page — see [Run].
CloseApplications=yes
RestartApplications=no

WizardStyle=modern
SetupLogging=yes

; LZMA2/max with solid compression: one small download and the slowest
; possible single-file extraction. The payload is a single Rust executable, so
; there is no decompression-latency trade-off worth making here.
Compression=lzma2/max
SolidCompression=yes

; x64-only product (gpui 0.2.2 / Rust target is x86_64-pc-windows-msvc).
; x64compatible covers x64 plus the ARM64 Windows that can run it natively.
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

; Setup.exe file properties, so the installer shows a real version in Windows
; Explorer's file properties instead of 0.0.0.0. VersionInfoVersion is the
; binary VS_FIXEDFILEINFO field and therefore takes up to four dot-separated
; numbers only; a non-numeric pre-release suffix would be rejected here, which
; is a constraint to remember if the project ever adopts SemVer pre-releases.
VersionInfoVersion={#AppVersion}
VersionInfoProductVersion={#AppVersion}
VersionInfoDescription=Sh Images Setup
VersionInfoCompany=Sh Images
VersionInfoProductName=Sh Images
VersionInfoCopyRight=Copyright (C) 2026 Adrian Quiros

; Relative to this file (installer\), landing in the git-ignored target/ tree
; so compiled installers never pollute the working tree status.
OutputDir=..\target\installer
OutputBaseFilename=ShImages-Setup-{#AppVersion}-win-x64

[Files]
; Executable name comes from the crate, not a guess: crates/sh-app/Cargo.toml
; declares package "sh-app" with src/main.rs and no [[bin]] override, so
; `cargo build --release -p sh-app` produces target\release\sh-app.exe.
Source: "..\target\release\{#AppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
; The branding icon is installed into {app} so the shortcuts can point at a
; file that provably exists on the target machine. [Icons] IconFilename is a
; RUNTIME path, not a compile-time Source: the setup runs from a temp
; extraction directory, so a repo-relative path there resolves to nothing.
; Because this entry is in [Files], the uninstaller tracks the .ico and
; removes it with the rest of the install.
Source: "..\assets\branding\sh-images.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; IconFilename is a RUNTIME path (see the [Files] note above): it is written
; verbatim into the .lnk, never resolved or validated at compile time, and
; Inno does not check it at install time either. A relative path here is
; therefore silently accepted and produces a shortcut pointing at a file that
; does not exist. Referencing the copy in {app} makes the shortcut correct by
; construction rather than by luck. Index 0 on a multi-resolution .ico lets
; Windows pick the size it needs.
;
; Start Menu: the entry point for an installed desktop app on Windows, and the
; only one guaranteed to exist for every user profile.
Name: "{group}\Sh Images"; Filename: "{app}\{#AppExeName}"; IconFilename: "{app}\sh-images.ico"
Name: "{group}\Uninstall Sh Images"; Filename: "{uninstallexe}"
; Desktop shortcut is a task-page choice. The Check guard is what actually
; enforces "no shortcut during a silent install": it holds regardless of how
; the task flags were resolved for this run.
Name: "{autodesktop}\Sh Images"; Filename: "{app}\{#AppExeName}"; IconFilename: "{app}\sh-images.ico"; Tasks: desktopicon; Check: not WizardSilent

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Run]
; Post-install launch offer. The checkbox label is the Description parameter
; ([Run] has no Name parameter); skipifsilent covers unattended / CI runs so a
; silent install never leaves a process behind.
;
; The checkbox is intentionally left CHECKED by default (no unchecked flag):
; someone who just installed or upgraded an image viewer overwhelmingly wants
; to see the result, and a pre-ticked box still requires them to press Finish.
; This is the relaunch path the [Setup] comment refers to; it is a visible,
; dismissible choice, not Restart Manager silently restarting a closed app.
; Add unchecked only if the product decision changes.
;
; There is deliberately only ONE entry here. [Run] executes a program; it
; cannot create a shortcut. An earlier revision added a second entry labelled
; "Create a desktop shortcut" that pointed at {#AppExeName}, which did not
; create anything and instead launched the viewer a second time under a
; misleading label. The desktop shortcut is created solely by [Icons] under
; the desktopicon task, so ticking that box and then ticking "launch" would
; have started two instances. Do not re-add a shortcut-creating [Run] entry.
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram,Sh Images}"; Flags: nowait postinstall skipifsilent

[Registry]
; Intentionally empty — NO FILE ASSOCIATIONS ARE REGISTERED HERE.
;
; This was a deliberate decision, not an oversight, and the prerequisite is
; already satisfied. Verified on origin/main @ b18bf08:
;   * crates/sh-app/src/main.rs:30 reads argv[1] as a path
;     (std::env::args().nth(1));
;   * main.rs:112-118 classifies it with Path::is_dir;
;   * crates/sh-app/src/state/view.rs:37-43 routes a file argument to
;     StartupTarget::Viewer, and main.rs then resolves the parent folder so the
;     filmstrip and folder navigation work.
; main.rs:3-7 also documents that the #![windows_subsystem = "windows"]
; attribute does not affect argv forwarding. So double-click-to-open would
; work today.
;
; Associations are still deferred because registering them writes per-user HKCR
; handler entries that take over the user's default handler for those
; extensions. That is a system-wide side effect with its own failure modes —
; what happens to a pre-existing handler, whether "Open with" state survives
; uninstall, and how the .png default is restored afterwards. None of that is
; covered by the install/uninstall smoke test in release.yml, and it deserves
; its own reviewed change plus manual QA rather than riding along with
; packaging. Add a [Registry] block here once that is decided; the argv
; handling it depends on is already in place.

[UninstallDelete]
; Intentionally empty.
; The installer owns no subdirectories: it writes only {#AppExeName}, LICENSE,
; sh-images.ico and its own uninstaller into {app}, and Inno removes {app}
; itself when it becomes empty. Every one of those files is recorded in
; unins000.dat by [Files], so the uninstaller deletes them without any help
; from this section.
;
; Hard rule for future edits: never add a path here that resolves inside
; %APPDATA%\sh_images (or any other user data location). Deleting a user's
; settings or custom themes on uninstall is data loss, not cleanup. There is
; also no wildcard deletion — only literal, installer-owned paths belong here.

[Code]
const
  UninstallKey =
    'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#SetupSetting("AppId")}_is1';

{ Refuse a silent downgrade.
  Every [Files] entry is ignoreversion, because the release executable is
  stripped and carries no usable version resource, so Inno has nothing to
  compare and an OLD installer would happily overwrite a NEWER install. The
  visible symptom is AppVerName walking backwards in Add/Remove Programs while
  the user believes they are current.

  There is no declarative directive for this: MinVersion is the minimum WINDOWS
  version on which Setup may run, not the minimum already-installed app
  version, and ISCC rejects it for any value below 6.1. The only supported guard
  is an explicit comparison against the installed app's DisplayVersion, which
  is why this function exists.

  PrivilegesRequired=lowest is why the key is read from HKEY_CURRENT_USER:
  CreateUninstallRegKey writes a per-user entry there and nowhere else. }
function InitializeSetup(): Boolean;
var
  InstalledVersion: String;
  InstalledPacked: Int64;
  IncomingPacked: Int64;
begin
  Result := True;

  { Not installed yet, or no version recorded: nothing to compare against. }
  if not RegQueryStringValue(HKEY_CURRENT_USER, UninstallKey,
    'DisplayVersion', InstalledVersion) then
    Exit;

  { An unparsable version is not a reason to block the user. }
  if (not StrToVersion(InstalledVersion, InstalledPacked)) or
     (not StrToVersion('{#AppVersion}', IncomingPacked)) then
    Exit;

  { Same or older installed version: this is a normal install or upgrade. }
  if InstalledPacked <= IncomingPacked then
    Exit;

  { A newer version is already installed. Abort without touching anything, and
    never raise a dialog under a silent install, or an unattended run would
    block forever on a message box nobody can click. }
  if not WizardSilent then
    MsgBox('A newer version of {#SetupSetting("AppName")} is already installed.' + #13#10 +
      'Installed: ' + InstalledVersion + #13#10 +
      'This installer: {#SetupSetting("AppVersion")}' + #13#10#13#10 +
      'Setup will exit without changing anything.',
      mbError, MB_OK);
  Result := False;
end;
