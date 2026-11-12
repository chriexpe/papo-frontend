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
            $expected = DirectoryAcl
            $sections = [System.Security.AccessControl.AccessControlSections]'Access,Owner'
            # Reapplying an unchanged inheritable DACL can propagate through new files.
            # Compare the root ACL first so warm startup does not indirectly walk the tree.
            if ($info.GetAccessControl().GetSecurityDescriptorSddlForm($sections) -ne $expected.GetSecurityDescriptorSddlForm($sections)) {
                $info.SetAccessControl($expected)
            }
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
    function Migrated($marker) {
        if (-not [System.IO.File]::Exists($marker)) { return $false }
        $info = [System.IO.FileInfo]::new($marker)
        if ($info.Attributes -band [System.IO.FileAttributes]::ReparsePoint) { throw 'reparse point' }
        $acl = $info.GetAccessControl()
        $rules = $acl.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier])
        return ($acl.AreAccessRulesProtected -and
            $acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value -eq $sid.Value -and
            $rules.Count -eq 1 -and $rules[0].IdentityReference.Value -eq $sid.Value -and
            $rules[0].AccessControlType -eq 'Allow' -and
            $rules[0].FileSystemRights -eq [System.Security.AccessControl.FileSystemRights]::FullControl -and
            $info.Length -eq 17 -and
            [System.IO.File]::ReadAllText($marker) -eq 'papo-owner-acl-v1')
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
            # Only a completed owner-private migration can skip the legacy tree walk.
            # The profile lease serializes migration with every SDK file creator.
            $marker = [System.IO.Path]::Combine($path,'.private-acl-v1')
            if (-not (Migrated $marker)) {
                foreach ($name in @('cache','media','outgoing')) {
                    $child = [System.IO.Path]::Combine($path,$name)
                    if ([System.IO.Directory]::Exists($child)) { SecureTree $child }
                }
                SecureFile $marker
                $stream = [System.IO.File]::Open($marker,'Truncate','Write','None')
                try {
                    $bytes = [System.Text.Encoding]::UTF8.GetBytes('papo-owner-acl-v1')
                    $stream.Write($bytes,0,$bytes.Length)
                    $stream.Flush($true)
                } finally { $stream.Dispose() }
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
