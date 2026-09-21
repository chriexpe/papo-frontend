//! Iniciar o Papo junto com a sessão.
//!
//! Fora do Flatpak basta um `.desktop` em `~/.config/autostart`, que é onde
//! o GNOME, o Plasma e os demais procuram. Dentro do Flatpak o caminho é o
//! mesmo, mas o comando não pode ser o binário do sandbox — lá fora ele não
//! existe —, então quem inicia é o próprio `flatpak run`.

use std::path::PathBuf;

/// Nome do arquivo de autostart. Igual ao do aplicativo, para o ambiente
/// reconhecer a entrada como sendo dele.
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

/// O comando que a sessão deve executar.
fn exec_command() -> String {
    if super::in_flatpak() {
        format!("flatpak run {}", crate::APP_ID)
    } else {
        // O caminho pode ter espaços; a especificação do `.desktop` pede
        // aspas nesse caso.
        let exe = std::env::current_exe()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "papo".to_owned());
        format!("\"{exe}\"")
    }
}

/// O Papo já está marcado para iniciar com a sessão?
pub fn is_enabled() -> bool {
    entry_path().map(|path| path.is_file()).unwrap_or(false)
}

/// Liga ou desliga o início automático. Devolve o motivo da falha, se houver.
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

    /// O ambiente lê o `.desktop` por chaves; sem elas a entrada é ignorada
    /// em silêncio e o aplicativo só não abre no login, sem erro nenhum.
    #[test]
    fn entrada_tem_as_chaves_que_o_ambiente_le() {
        let entry = desktop_entry();
        assert!(entry.starts_with("[Desktop Entry]"));
        assert!(entry.contains("Type=Application"));
        assert!(entry.contains("\nExec="));
        assert!(entry.contains("X-GNOME-Autostart-enabled=true"));
    }
}
