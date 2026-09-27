param(
    [Parameter(Mandatory = $true)]
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$sizes = @(16, 22, 24, 32, 48, 64, 128, 256)
$images = @()

foreach ($size in $sizes) {
    $path = Join-Path $repo "assets\icons\$size.png"
    if (-not (Test-Path $path)) { throw "Missing icon asset: $path" }
    $images += ,@($size, [System.IO.File]::ReadAllBytes($path))
}

$stream = [System.IO.MemoryStream]::new()
$writer = [System.IO.BinaryWriter]::new($stream)
$writer.Write([UInt16]0)
$writer.Write([UInt16]1)
$writer.Write([UInt16]$images.Count)

$offset = 6 + 16 * $images.Count
foreach ($entry in $images) {
    $size = [int]$entry[0]
    $bytes = [byte[]]$entry[1]
    $writer.Write([byte]$(if ($size -eq 256) { 0 } else { $size }))
    $writer.Write([byte]$(if ($size -eq 256) { 0 } else { $size }))
    $writer.Write([byte]0)
    $writer.Write([byte]0)
    $writer.Write([UInt16]1)
    $writer.Write([UInt16]32)
    $writer.Write([UInt32]$bytes.Length)
    $writer.Write([UInt32]$offset)
    $offset += $bytes.Length
}

foreach ($entry in $images) {
    $writer.Write([byte[]]$entry[1])
}
$writer.Flush()

$parent = Split-Path -Parent $OutputPath
if ($parent) { New-Item -ItemType Directory -Force $parent | Out-Null }
[System.IO.File]::WriteAllBytes($OutputPath, $stream.ToArray())
$writer.Dispose()
$stream.Dispose()
Write-Host "Windows icon: $OutputPath"
