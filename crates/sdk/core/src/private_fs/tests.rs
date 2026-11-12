//! Permission regressions, including inherited Windows DACLs; no user data or network.

use super::*;

#[cfg(unix)]
#[test]
fn profile_and_staged_paths_are_private_even_below_a_public_parent() {
    use std::os::unix::fs::PermissionsExt;
    let parent = tempfile::tempdir().unwrap();
    fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let root = parent.path().join("profile");
    profile(&root).unwrap();
    let path = root.join("staging");
    directory(&path).unwrap();
    let bytes = path.join("file");
    file(&bytes).unwrap();
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&bytes).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&bytes, fs::Permissions::from_mode(0o644)).unwrap();
    file(&bytes).unwrap();
    assert_eq!(
        fs::metadata(bytes).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(windows)]
#[test]
fn profile_and_staged_files_have_only_owner_allow_rules() {
    let parent = tempfile::tempdir().unwrap();
    let shell = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let public = r"$ErrorActionPreference='Stop';try {
        $path=$env:PAPO_PUBLIC_PARENT
        $acl=Get-Acl -LiteralPath $path
        $world=[System.Security.Principal.SecurityIdentifier]::new('S-1-1-0')
        $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($world,'FullControl','ContainerInherit,ObjectInherit','None','Allow'))
        [System.IO.Directory]::SetAccessControl($path,$acl)
        exit 0
    } catch {exit 1}";
    assert!(
        std::process::Command::new(&shell)
            .args(["-NoProfile", "-NonInteractive", "-Command", public])
            .env("PAPO_PUBLIC_PARENT", parent.path())
            .status()
            .unwrap()
            .success()
    );
    let root = parent.path().join("profile");
    fs::create_dir(&root).unwrap();
    let state = root.join("state.sqlite");
    fs::write(&state, []).unwrap();
    profile(&root).unwrap();
    audit(&root).unwrap();
    let path = root.join("staging");
    directory(&path).unwrap();
    let bytes = path.join("file");
    file(&bytes).unwrap();
    let script = r"$ErrorActionPreference='Stop';try {
        $sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        foreach($path in @($env:PAPO_ACL_ROOT,$env:PAPO_ACL_DIR,$env:PAPO_ACL_FILE,$env:PAPO_ACL_STATE)) {
            $acl=Get-Acl -LiteralPath $path
            if(-not $acl.AreAccessRulesProtected) {exit 2}
            $rules=$acl.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier])
            if($rules.Count -ne 1 -or $rules[0].IdentityReference.Value -ne $sid -or $rules[0].AccessControlType -ne 'Allow') {exit 3}
        };exit 0
    } catch {exit 1}";
    let result = std::process::Command::new(shell)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("PAPO_ACL_ROOT", root)
        .env("PAPO_ACL_DIR", path)
        .env("PAPO_ACL_FILE", bytes)
        .env("PAPO_ACL_STATE", state)
        .status()
        .unwrap();
    assert!(result.success());
}
