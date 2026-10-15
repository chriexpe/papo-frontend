//! Iniciar o Papo junto com a sessão.

#[cfg(target_os = "linux")]
mod imp {
    use std::path::PathBuf;

    const FILE_NAME: &str = "io.github.chriexpe.Papo.desktop";

    fn config_home() -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME")
            && !dir.is_empty()
        {
            return Some(PathBuf::from(dir));
        }
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
    }

    fn entry_path() -> Option<PathBuf> {
        Some(config_home()?.join("autostart").join(FILE_NAME))
    }

    fn exec_command() -> String {
        if super::super::in_flatpak() {
            format!("flatpak run {}", crate::APP_ID)
        } else {
            let exe = std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "papo".to_owned());
            format!("\"{exe}\"")
        }
    }

    pub fn is_enabled() -> bool {
        entry_path().map(|path| path.is_file()).unwrap_or(false)
    }

    pub fn set(enabled: bool) -> Result<(), String> {
        let Some(path) = entry_path() else {
            return Err("sem pasta de configuração do usuário".to_owned());
        };
        if enabled {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            std::fs::write(&path, desktop_entry()).map_err(|error| error.to_string())?;
        } else if path.exists() {
            std::fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn desktop_entry() -> String {
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Papo\n\
             Comment=Cliente nativo do Papo\n\
             Exec={}\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n",
            exec_command()
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn entrada_tem_as_chaves_que_o_ambiente_le() {
            let entry = desktop_entry();
            assert!(entry.starts_with("[Desktop Entry]"));
            assert!(entry.contains("Type=Application"));
            assert!(entry.contains("\nExec="));
            assert!(entry.contains("X-GNOME-Autostart-enabled=true"));
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SZ, RegCloseKey,
        RegCreateKeyW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "Papo";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open(access: windows::Win32::System::Registry::REG_SAM_FLAGS, create: bool) -> Result<HKEY, String> {
        let key = wide(RUN_KEY);
        let mut handle = HKEY::default();
        let status = unsafe {
            if create {
                RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), &mut handle)
            } else {
                RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), None, access, &mut handle)
            }
        };
        if status.0 != 0 {
            return Err(format!("registro de início automático: {}", status.0));
        }
        Ok(handle)
    }

    pub fn is_enabled() -> bool {
        let Ok(key) = open(KEY_READ, false) else {
            return false;
        };
        let name = wide(VALUE_NAME);
        let mut bytes = 0u32;
        let status = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                None,
                None,
                Some(&mut bytes),
            )
        };
        unsafe { let _ = RegCloseKey(key); }
        status.0 == 0 && bytes > 2
    }

    pub fn set(enabled: bool) -> Result<(), String> {
        let key = open(KEY_SET_VALUE, enabled)?;
        let name = wide(VALUE_NAME);
        let status = if enabled {
            let exe = std::env::current_exe().map_err(|error| error.to_string())?;
            let command = format!("\"{}\"", exe.display());
            let value = wide(&command);
            let bytes = unsafe {
                std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2)
            };
            unsafe {
                RegSetValueExW(
                    key,
                    PCWSTR(name.as_ptr()),
                    None,
                    REG_SZ,
                    Some(bytes),
                )
            }
        } else {
            unsafe { RegDeleteValueW(key, PCWSTR(name.as_ptr())) }
        };
        unsafe { let _ = RegCloseKey(key); }
        // ERROR_FILE_NOT_FOUND while disabling is already the desired state.
        if status.0 == 0 || (!enabled && status.0 == 2) {
            Ok(())
        } else {
            Err(format!("registro de início automático: {}", status.0))
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::PathBuf;

    const LABEL: &str = "io.github.chriexpe.Papo";
    const PLIST_NAME: &str = "io.github.chriexpe.Papo.plist";

    fn plist_path() -> Result<PathBuf, String> {
        let home = std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .ok_or("macOS home directory unavailable")?;
        Ok(PathBuf::from(home).join("Library/LaunchAgents").join(PLIST_NAME))
    }

    fn xml_escape(value: &str) -> String {
        value.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    fn executable() -> Result<PathBuf, String> {
        std::env::current_exe().map_err(|error| error.to_string())
    }

    fn entry(executable: &std::path::Path) -> String {
        // LaunchAgents are per-user; RunAtLoad starts Papo on the next login.
        // Pass arguments separately, never through a shell or string parsing.
        let executable = xml_escape(&executable.to_string_lossy());
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\"><dict>\n\
             <key>Label</key><string>{LABEL}</string>\n\
             <key>ProgramArguments</key><array><string>{executable}</string></array>\n\
             <key>RunAtLoad</key><true/>\n\
             <key>KeepAlive</key><false/>\n\
             </dict></plist>\n"
        )
    }

    pub fn is_enabled() -> bool {
        plist_path().is_ok_and(|path| path.is_file())
    }

    pub fn set(enabled: bool) -> Result<(), String> {
        let path = plist_path()?;
        if !enabled {
            if path.exists() {
                std::fs::remove_file(&path).map_err(|error| error.to_string())?;
            }
            return Ok(());
        }
        let executable = executable()?;
        let parent = path.parent().ok_or("invalid LaunchAgents path")?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        // Atomic replacement prevents partially written launchd property lists.
        let staging = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&staging, entry(&executable)).map_err(|error| error.to_string())?;
        if let Err(error) = std::fs::rename(&staging, &path) {
            let _ = std::fs::remove_file(&staging);
            return Err(error.to_string());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn launchagent_escapes_paths_and_uses_array_arguments() {
            let value = entry(std::path::Path::new("/Applications/Papo & Friends.app/Contents/MacOS/papo"));
            assert!(value.contains("Papo &amp; Friends.app"));
            assert!(value.contains("<key>RunAtLoad</key><true/>"));
            assert!(value.contains("<key>ProgramArguments</key><array>"));
            assert!(!value.contains("<key>KeepAlive</key><true/>"));
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
mod imp {
    pub fn is_enabled() -> bool { false }
    pub fn set(_enabled: bool) -> Result<(), String> {
        Err("início automático indisponível nesta plataforma".to_owned())
    }
}

pub use imp::{is_enabled, set};
