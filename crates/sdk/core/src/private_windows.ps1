# Owner-only ACLs at creation and for existing SDK files; never emits paths or contents.
$ErrorActionPreference = 'Stop'
try {
    $sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
    function DirectoryAcl {
        $acl = [System.Security.AccessControl.DirectorySecurity]::new()
        $acl.SetOwner($sid)
        $acl.SetAccessRuleProtection($true, $false)
        $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
        return $acl
    }
    function FileAcl {
        $acl = [System.Security.AccessControl.FileSecurity]::new()
        $acl.SetOwner($sid)
        $acl.SetAccessRuleProtection($true, $false)
        $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'Allow'))
        return $acl
    }
    function SecureDirectory($path) {
        if ([System.IO.Directory]::Exists($path)) {
            $info = [System.IO.DirectoryInfo]::new($path)
            if ($info.Attributes -band [System.IO.FileAttributes]::ReparsePoint) { throw 'reparse point' }
            $info.SetAccessControl((DirectoryAcl))
        } else {
            [void][System.IO.Directory]::CreateDirectory($path, (DirectoryAcl))
        }
    }
    function SecureFile($path) {
        if ([System.IO.File]::Exists($path)) {
            $info = [System.IO.FileInfo]::new($path)
            if ($info.Attributes -band [System.IO.FileAttributes]::ReparsePoint) { throw 'reparse point' }
            $info.SetAccessControl((FileAcl))
        } else {
            $file = [System.IO.FileStream]::new($path, 'OpenOrCreate', [System.Security.AccessControl.FileSystemRights]::FullControl, 'ReadWrite', 4096, 'None', (FileAcl))
            $file.Dispose()
        }
    }
    function SecureTree($path) {
        SecureDirectory $path
        foreach ($entry in [System.IO.Directory]::EnumerateFileSystemEntries($path)) {
            if ([System.IO.Directory]::Exists($entry)) { SecureTree $entry } else { SecureFile $entry }
        }
    }
    $path = $env:PAPO_PRIVATE_PATH
    switch ($env:PAPO_PRIVATE_KIND) {
        'directory' { SecureDirectory $path }
        'file' { SecureFile $path }
        'profile' {
            foreach ($name in @('cache','media','outgoing')) {
                $child = [System.IO.Path]::Combine($path,$name)
                if ([System.IO.Directory]::Exists($child)) { SecureTree $child }
            }
            foreach ($name in @('.lock','state.sqlite','state.sqlite-wal','state.sqlite-shm','state.sqlite-journal')) {
                $child = [System.IO.Path]::Combine($path,$name)
                if ([System.IO.File]::Exists($child)) { SecureFile $child }
            }
        }
        default { throw 'invalid operation' }
    }
    exit 0
} catch { exit 1 }
