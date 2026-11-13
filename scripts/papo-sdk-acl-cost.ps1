# Measures synthetic Windows ACL migration and warm audits; never opens a real profile.
param([int]$FileCount = 10000)
$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot '../crates/sdk/core/src/private_windows.ps1'
$shell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
$fixture = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
function AuditMilliseconds {
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    if ($env:PAPO_PRIVATE_KIND -eq 'profile') {
        $env:PAPO_PRIVATE_KIND = 'directory'
        & $shell -NoProfile -NonInteractive -File $script
        if ($LASTEXITCODE -ne 0) { throw 'Root ACL fixture failed' }
        $env:PAPO_PRIVATE_KIND = 'profile'
    }
    & $shell -NoProfile -NonInteractive -File $script
    if ($LASTEXITCODE -ne 0) { throw 'ACL fixture failed' }
    $timer.Stop()
    return $timer.Elapsed.TotalMilliseconds
}
try {
    [void][System.IO.Directory]::CreateDirectory($fixture)
    $env:PAPO_PRIVATE_PATH = $fixture
    $env:PAPO_PRIVATE_KIND = 'directory'
    $shellAclMs = AuditMilliseconds
    for ($i = 0; $i -lt $FileCount; $i++) {
        $tree = @('cache','media','outgoing')[$i % 3]
        $directory = Join-Path $fixture ($tree + '/' + ($i % 100))
        [void][System.IO.Directory]::CreateDirectory($directory)
        [System.IO.File]::WriteAllBytes((Join-Path $directory ($i.ToString() + '.bin')), [byte[]]@(0))
    }
    $env:PAPO_PRIVATE_KIND = 'profile'
    $migrationMs = AuditMilliseconds
    $warmMs = @(1..5 | ForEach-Object { AuditMilliseconds })
    # New descendants inherit exclusively from the protected owner-only tree.
    $sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $staged = Join-Path $fixture 'outgoing/new/account/blob'
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    [void][System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($staged))
    [System.IO.File]::WriteAllBytes($staged, [byte[]]@(0))
    $timer.Stop()
    $acl = [System.IO.File]::GetAccessControl($staged)
    $rules = $acl.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier])
    if ($rules.Count -ne 1 -or $rules[0].IdentityReference.Value -ne $sid -or $rules[0].AccessControlType -ne 'Allow') {
        throw 'Inherited staging ACL failed'
    }
    [ordered]@{
        fixture_files = $FileCount
        migration_ms = $migrationMs
        warm_audit_ms = $warmMs
        shell_acl_call_ms = $shellAclMs
        staged_create_ms = $timer.Elapsed.TotalMilliseconds
        staging_shell_processes = 0
    } | ConvertTo-Json -Compress
} finally {
    if ([System.IO.Directory]::Exists($fixture)) { [System.IO.Directory]::Delete($fixture,$true) }
    Remove-Item Env:PAPO_PRIVATE_PATH,Env:PAPO_PRIVATE_KIND -ErrorAction SilentlyContinue
}
