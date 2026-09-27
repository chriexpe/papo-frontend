#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif
#ifndef PayloadDir
  #error PayloadDir must point at the prepared Windows installer payload
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif
#ifndef AppIcon
  #error AppIcon must point at the generated Windows ICO
#endif

#define MyAppName "Papo"
#define MyAppExeName "papo.exe"
#define MyAppPublisher "Papo Chat"
#define MyAppURL "https://github.com/Papo-Chat/papo-frontend"
#define MyAppId "{{A9194D63-2E47-590F-BC86-95BEB1F6D7C2}"
#define MyAppUserModelID "io.github.chriexpe.Papo"

[Setup]
AppId={#MyAppId}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\Papo
DefaultGroupName=Papo
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=Papo-{#MyAppVersion}-Setup
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
SetupLogging=yes
SetupIconFile={#AppIcon}
UninstallDisplayIcon={app}\{#MyAppExeName}
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Papo"; Filename: "{app}\{#MyAppExeName}"; AppUserModelID: "{#MyAppUserModelID}"
Name: "{autodesktop}\Papo"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon; AppUserModelID: "{#MyAppUserModelID}"

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,Papo}"; Flags: nowait postinstall skipifsilent

[Code]
const
  VcRedistUrl = 'https://aka.ms/vs/17/release/vc_redist.x64.exe';
  WebView2Url = 'https://go.microsoft.com/fwlink/p/?LinkId=2124703';
  WebView2ClientGuid = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

var
  NeedVcRuntime: Boolean;
  NeedWebView2: Boolean;
  PrereqsPrepared: Boolean;
  VcRedistPath: String;
  WebView2Path: String;

function VcRuntimeInstalled: Boolean;
var
  Installed: Cardinal;
begin
  Result :=
    RegQueryDWordValue(
      HKEY_LOCAL_MACHINE_64,
      'SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64',
      'Installed',
      Installed
    ) and (Installed = 1);
end;

function WebView2Installed: Boolean;
var
  Version: String;
  Key: String;
begin
  Key := 'SOFTWARE\Microsoft\EdgeUpdate\Clients\' + WebView2ClientGuid;
  Result :=
    (RegQueryStringValue(HKEY_LOCAL_MACHINE_32, Key, 'pv', Version) and
      (Version <> '') and (Version <> '0.0.0.0')) or
    (RegQueryStringValue(HKEY_CURRENT_USER_32, Key, 'pv', Version) and
      (Version <> '') and (Version <> '0.0.0.0'));
end;

function PrerequisiteSummary: String;
begin
  Result := '';
  if NeedVcRuntime then
    Result := Result + #13#10 + '  • Microsoft Visual C++ 2015-2022 Runtime (x64)';
  if NeedWebView2 then
    Result := Result + #13#10 + '  • Microsoft Edge WebView2 Evergreen Runtime';
end;

function DownloadProgress(
  const Url, FileName: String;
  const Progress, ProgressMax: Int64
): Boolean;
begin
  if ProgressMax > 0 then
    WizardForm.StatusLabel.Caption :=
      Format('Baixando pré-requisito... %d%%', [(Progress * 100) div ProgressMax])
  else
    WizardForm.StatusLabel.Caption := 'Baixando pré-requisito...';
  Result := True;
end;

function PreparePrerequisites: Boolean;
var
  Answer: Integer;
begin
  Result := False;

  NeedVcRuntime := not VcRuntimeInstalled;
  NeedWebView2 := not WebView2Installed;

  if not NeedVcRuntime and not NeedWebView2 then
  begin
    PrereqsPrepared := True;
    Result := True;
    Exit;
  end;

  if WizardSilent then
  begin
    Answer := IDYES;
  end
  else
  begin
    Answer := MsgBox(
      'O Papo precisa destes componentes da Microsoft que ainda não foram encontrados:' +
      PrerequisiteSummary + #13#10#13#10 +
      'O Setup pode baixá-los agora diretamente da Microsoft. Continuar?',
      mbConfirmation,
      MB_YESNO
    );
  end;

  if Answer <> IDYES then
    Exit;

  try
    if NeedVcRuntime then
    begin
      DownloadTemporaryFile(
        VcRedistUrl,
        'vc_redist.x64.exe',
        '',
        @DownloadProgress
      );
      VcRedistPath := ExpandConstant('{tmp}\vc_redist.x64.exe');
    end;

    if NeedWebView2 then
    begin
      DownloadTemporaryFile(
        WebView2Url,
        'MicrosoftEdgeWebview2Setup.exe',
        '',
        @DownloadProgress
      );
      WebView2Path := ExpandConstant('{tmp}\MicrosoftEdgeWebview2Setup.exe');
    end;
  except
    MsgBox(
      'Não foi possível baixar os pré-requisitos:' + #13#10 +
      GetExceptionMessage,
      mbError,
      MB_OK
    );
    Exit;
  end;

  PrereqsPrepared := True;
  Result := True;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID = wpReady) and not PrereqsPrepared then
    Result := PreparePrerequisites;
end;

function InstallOnePrerequisite(
  const FileName, Params, DisplayName: String
): Boolean;
var
  ResultCode: Integer;
begin
  WizardForm.StatusLabel.Caption := 'Instalando ' + DisplayName + '...';
  Result :=
    Exec(
      FileName,
      Params,
      ExpandConstant('{tmp}'),
      SW_HIDE,
      ewWaitUntilTerminated,
      ResultCode
    ) and
    ((ResultCode = 0) or (ResultCode = 1638) or (ResultCode = 3010));

  if not Result then
    MsgBox(
      DisplayName + ' não pôde ser instalado.' + #13#10 +
      'Código: ' + IntToStr(ResultCode),
      mbError,
      MB_OK
    );
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep <> ssInstall then
    Exit;

  if NeedVcRuntime then
  begin
    if not InstallOnePrerequisite(
      VcRedistPath,
      '/install /quiet /norestart',
      'Microsoft Visual C++ Runtime'
    ) then
      RaiseException('Falha ao instalar o Visual C++ Runtime.');
  end;

  if NeedWebView2 then
  begin
    if not InstallOnePrerequisite(
      WebView2Path,
      '/silent /install',
      'Microsoft Edge WebView2 Runtime'
    ) then
      RaiseException('Falha ao instalar o WebView2 Runtime.');
  end;
end;
