param(
    [Parameter(Mandatory = $true)]
    [string]$GStreamerRoot,

    [Parameter(Mandatory = $true)]
    [string]$PapoExe,

    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$RsRpcExe,

    [Parameter(Mandatory = $true)]
    [string]$OutputDir
)

$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$payload = Join-Path ([System.IO.Path]::GetTempPath()) ('papo-installer-payload-' + [guid]::NewGuid().ToString('N'))

try {
    & (Join-Path $scriptDir 'bundle.ps1') `
        -GStreamerRoot $GStreamerRoot `
        -PapoExe $PapoExe `
        -RsRpcExe $RsRpcExe `
        -OutputDir $payload `
        -IncludeVCRuntime $false

    New-Item -ItemType Directory -Force $OutputDir | Out-Null

    $programFilesX86 = [Environment]::GetFolderPath('ProgramFilesX86')
    $isccCandidates = @(
        (Join-Path $programFilesX86 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
    )
    $iscc = $isccCandidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
    if (-not $iscc) {
        throw 'Inno Setup 6 compiler (ISCC.exe) not found'
    }

    $resolvedOutput = (Resolve-Path $OutputDir).Path
    $icon = Join-Path $payload 'papo.ico'
    & (Join-Path $scriptDir 'make-icon.ps1') -OutputPath $icon

    $iss = Join-Path $scriptDir 'papo.iss'
    & $iscc `
        "/DMyAppVersion=$Version" `
        "/DPayloadDir=$payload" `
        "/DOutputDir=$resolvedOutput" `
        "/DAppIcon=$icon" `
        $iss

    if ($LASTEXITCODE -ne 0) {
        throw "Inno Setup failed with exit code $LASTEXITCODE"
    }

    $setup = Get-ChildItem $OutputDir -Filter 'Papo-*-Setup.exe' -File |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if (-not $setup) {
        throw 'Inno Setup completed but no Setup.exe was produced'
    }

    Write-Host "Windows installer: $($setup.FullName)"
}
finally {
    if (Test-Path $payload) {
        Remove-Item -Recurse -Force $payload
    }
}
