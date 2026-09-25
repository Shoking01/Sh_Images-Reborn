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
; replace the locked executable. RestartApplications=no is deliberate: we do
; not relaunch on the user's behalf after an upgrade they did not ask for.
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

[Icons]
; Start Menu: the entry point for an installed desktop app on Windows, and the
; only one guaranteed to exist for every user profile.
Name: "{group}\Sh Images"; Filename: "{app}\{#AppExeName}"; IconFilename: "..\assets\branding\sh-images.ico"
Name: "{group}\Uninstall Sh Images"; Filename: "{uninstallexe}"
; Desktop shortcut is a task-page choice. The Check guard is what actually
; enforces "no shortcut during a silent install": it holds regardless of how
; the task flags were resolved for this run.
Name: "{autodesktop}\Sh Images"; Filename: "{app}\{#AppExeName}"; IconFilename: "..\assets\branding\sh-images.ico"; Tasks: desktopicon; Check: not WizardSilent

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Run]
; Post-install launch offer. The checkbox label is the Description parameter
; ([Run] has no Name parameter); skipifsilent covers unattended / CI runs so a
; silent install never leaves a process behind.
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram,Sh Images}"; Flags: nowait postinstall skipifsilent
; Re-assert the desktop shortcut after the files are in place. Writing a
; shortcut path that already exists is an idempotent overwrite, so this is
; harmless on a fresh install and keeps the shortcut present on an in-place
; upgrade where Inno leaves an unchanged [Icons] entry alone.
Filename: "{app}\{#AppExeName}"; Description: "{cm:CreateDesktopIcon}"; Tasks: desktopicon; Flags: nowait postinstall skipifsilent

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
; The installer owns no subdirectories: it writes only {#AppExeName}, LICENSE
; and its own uninstaller into {app}, and Inno removes {app} itself when it
; becomes empty.
;
; Hard rule for future edits: never add a path here that resolves inside
; %APPDATA%\sh_images (or any other user data location). Deleting a user's
; settings or custom themes on uninstall is data loss, not cleanup. There is
; also no wildcard deletion — only literal, installer-owned paths belong here.
