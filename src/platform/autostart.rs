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

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod imp {
    pub fn is_enabled() -> bool { false }
    pub fn set(_enabled: bool) -> Result<(), String> {
        Err("início automático indisponível nesta plataforma".to_owned())
    }
}

pub use imp::{is_enabled, set};
