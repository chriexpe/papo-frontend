param(
    [Parameter(Mandatory = $true)]
    [string]$GStreamerRoot,

    [Parameter(Mandatory = $true)]
    [string]$PapoExe,

    [Parameter(Mandatory = $true)]
    [string]$OutputDir
)

$ErrorActionPreference = 'Stop'

$root = (Resolve-Path $GStreamerRoot).Path
$exe = (Resolve-Path $PapoExe).Path

if (Test-Path $OutputDir) {
    Remove-Item -Recurse -Force $OutputDir
}
New-Item -ItemType Directory -Force $OutputDir | Out-Null
New-Item -ItemType Directory -Force (Join-Path $OutputDir 'gstreamer-1.0') | Out-Null
New-Item -ItemType Directory -Force (Join-Path $OutputDir 'gio-modules') | Out-Null

Copy-Item $exe (Join-Path $OutputDir 'papo.exe')
Copy-Item (Join-Path $PSScriptRoot '..\..\LICENSE') (Join-Path $OutputDir 'LICENSE')

# DLL lookup on Windows checks the executable directory first. Shipping all
# runtime DLLs from GStreamer's bin makes the Papo folder independent from a
# machine-wide GStreamer installation and also covers transitive plugin DLLs.
Get-ChildItem (Join-Path $root 'bin') -Filter '*.dll' -File |
    Copy-Item -Destination $OutputDir

$plugins = Join-Path $root 'lib\gstreamer-1.0'
if (-not (Test-Path $plugins)) {
    throw "GStreamer plugin directory not found: $plugins"
}
Get-ChildItem $plugins -Filter '*.dll' -File |
    Copy-Item -Destination (Join-Path $OutputDir 'gstreamer-1.0')

$gio = Join-Path $root 'lib\gio\modules'
if (Test-Path $gio) {
    Get-ChildItem $gio -Filter '*.dll' -File |
        Copy-Item -Destination (Join-Path $OutputDir 'gio-modules')
}

$scannerCandidates = @(
    (Join-Path $root 'libexec\gstreamer-1.0\gst-plugin-scanner.exe'),
    (Join-Path $root 'bin\gst-plugin-scanner.exe')
)
$scanner = $scannerCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $scanner) {
    throw 'gst-plugin-scanner.exe not found in the GStreamer runtime'
}
Copy-Item $scanner (Join-Path $OutputDir 'gst-plugin-scanner.exe')

# Keep upstream notices when the runtime package exposes them.
foreach ($name in @('COPYING', 'COPYING.LIB', 'LICENSE', 'LICENSE.txt')) {
    $candidate = Join-Path $root $name
    if (Test-Path $candidate) {
        Copy-Item $candidate (Join-Path $OutputDir "GSTREAMER-$name")
    }
}

$required = @(
    'papo.exe',
    'gstreamer-1.0',
    'gst-plugin-scanner.exe'
)
foreach ($item in $required) {
    if (-not (Test-Path (Join-Path $OutputDir $item))) {
        throw "Bundle incomplete: missing $item"
    }
}

Write-Host "Windows bundle assembled at $OutputDir"
Write-Host "Runtime DLLs: $((Get-ChildItem $OutputDir -Filter '*.dll' -File).Count)"
Write-Host "GStreamer plugins: $((Get-ChildItem (Join-Path $OutputDir 'gstreamer-1.0') -Filter '*.dll' -File).Count)"
