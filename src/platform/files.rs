//! Seletor de arquivos, pasta de downloads e "abrir com o aplicativo padrão".
//!
//! Tudo passa pelo xdg-desktop-portal quando ele existe, então quem aparece é
//! o diálogo do próprio sistema — não um desenhado por nós.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use crate::api::client::Upload;

/// Resultado de um diálogo, entregue quando o usuário decide.
pub enum Chosen {
    Files(Vec<Upload>),
    /// Imagem escolhida para foto de perfil ou figurinha, já em base64 e com
    /// o formato que o backend espera (`PNG`, `GIF`, `JPEG`, `WEBP`).
    Image {
        purpose: ImagePick,
        blob: String,
        format: String,
    },
    Folder(PathBuf),
    /// Destino de um anexo que estava esperando o "salvar como".
    SaveAs { id: String, name: String, dest: PathBuf },
    Cancelled,
}

/// Para que serve a imagem que está sendo escolhida.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImagePick {
    Avatar,
    /// Figurinha do servidor, com o nome que ela vai ter.
    Emoji(String),
}

/// Diálogos abertos, à espera de resposta.
#[derive(Default)]
pub struct Dialogs {
    pending: Vec<mpsc::Receiver<Chosen>>,
}

impl Dialogs {
    /// Anexos para a mensagem.
    pub fn pick_files(&mut self, repaint: egui::Context) {
        self.spawn(repaint, |dialog| async move {
            match dialog.set_title("Anexar").pick_files().await {
                Some(handles) => Chosen::Files(
                    handles
                        .iter()
                        .map(|handle| describe(&handle.path().to_path_buf()))
                        .collect(),
                ),
                None => Chosen::Cancelled,
            }
        });
    }

    /// O mesmo seletor, filtrado nos formatos que animam. Uma busca de GIF
    /// de verdade (Tenor, Giphy) precisa de chave e de um proxy no backend;
    /// até lá, o arquivo vem do disco.
    pub fn pick_animations(&mut self, repaint: egui::Context) {
        self.spawn(repaint, |dialog| async move {
            match dialog
                .set_title("GIF")
                .add_filter("Animações", &["gif", "webp", "mp4", "webm"])
                .pick_files()
                .await
            {
                Some(handles) => Chosen::Files(
                    handles
                        .iter()
                        .map(|handle| describe(&handle.path().to_path_buf()))
                        .collect(),
                ),
                None => Chosen::Cancelled,
            }
        });
    }

    /// Pasta padrão de downloads.
    pub fn pick_folder(&mut self, repaint: egui::Context, start: PathBuf) {
        self.spawn(repaint, move |dialog| async move {
            match dialog
                .set_directory(start)
                .set_title("Salvar em")
                .pick_folder()
                .await
            {
                Some(handle) => Chosen::Folder(handle.path().to_path_buf()),
                None => Chosen::Cancelled,
            }
        });
    }

    /// Destino de um anexo, quando o ajuste é perguntar sempre.
    pub fn save_as(&mut self, repaint: egui::Context, id: String, name: String, start: PathBuf) {
        self.spawn(repaint, move |dialog| async move {
            match dialog
                .set_directory(start)
                .set_file_name(&name)
                .set_title("Salvar como")
                .save_file()
                .await
            {
                Some(handle) => Chosen::SaveAs {
                    id,
                    name,
                    dest: handle.path().to_path_buf(),
                },
                None => Chosen::Cancelled,
            }
        });
    }

    /// Imagem para foto de perfil ou figurinha. O backend recebe base64 e
    /// aceita no máximo 2 MB, então o arquivo é conferido aqui antes de
    /// subir — a alternativa é um 400 depois da espera.
    pub fn pick_image(&mut self, repaint: egui::Context, purpose: ImagePick) {
        self.spawn(repaint, |dialog| async move {
            let Some(handle) = dialog
                .set_title("Imagem")
                .add_filter("Imagens", &["png", "jpg", "jpeg", "gif", "webp"])
                .pick_file()
                .await
            else {
                return Chosen::Cancelled;
            };
            let path = handle.path().to_path_buf();
            let Ok(bytes) = std::fs::read(&path) else {
                return Chosen::Cancelled;
            };
            if bytes.len() > 2 * 1024 * 1024 {
                log::warn!("imagem de {} bytes: o limite do servidor é 2 MB", bytes.len());
                return Chosen::Cancelled;
            }
            let format = match path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("")
                .to_lowercase()
                .as_str()
            {
                "gif" => "GIF",
                "jpg" => "JPG",
                "jpeg" => "JPEG",
                "webp" => "WEBP",
                _ => "PNG",
            };
            use base64::Engine as _;
            Chosen::Image {
                purpose,
                blob: base64::engine::general_purpose::STANDARD.encode(&bytes),
                format: format.to_owned(),
            }
        });
    }

    /// O diálogo fala com o xdg-desktop-portal por D-Bus, e o zbus embaixo
    /// dele exige um runtime tokio de verdade. O runtime é **um só, vivo
    /// enquanto o programa estiver de pé**: o zbus guarda a conexão com o
    /// barramento da sessão num cache global, e essa conexão só continua
    /// funcionando enquanto o runtime que a criou segue rodando. Com um
    /// runtime por diálogo, o primeiro seletor abria, o runtime morria junto
    /// com a resposta e a conexão em cache ficava sem ninguém para bombeá-la
    /// — do segundo clique em diante o portal não respondia mais nada.
    fn spawn<F, Fut>(&mut self, repaint: egui::Context, build: F)
    where
        F: FnOnce(rfd::AsyncFileDialog) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Chosen>,
    {
        let Some(handle) = dialog_runtime() else {
            log::warn!("sem seletor de arquivos: o runtime não subiu");
            return;
        };
        let (tx, rx) = mpsc::channel();
        // A thread existe só para poder bloquear à espera da resposta; quem
        // conduz o D-Bus é o runtime compartilhado.
        if std::thread::Builder::new()
            .name("papo-dialog".into())
            .spawn(move || {
                let chosen = handle.block_on(build(rfd::AsyncFileDialog::new()));
                let _ = tx.send(chosen);
                repaint.request_repaint();
            })
            .is_ok()
        {
            self.pending.push(rx);
        }
    }

    /// Respostas que chegaram desde o último quadro.
    pub fn poll(&mut self) -> Vec<Chosen> {
        let mut out = Vec::new();
        self.pending.retain(|rx| match rx.try_recv() {
            Ok(chosen) => {
                out.push(chosen);
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => false,
        });
        out
    }
}

/// O runtime que conduz o D-Bus dos diálogos, criado uma vez e nunca
/// derrubado. Ver a explicação em `Dialogs::spawn`.
fn dialog_runtime() -> Option<tokio::runtime::Handle> {
    static RUNTIME: std::sync::OnceLock<Option<tokio::runtime::Runtime>> =
        std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("papo-portal")
                .enable_all()
                .build()
                .map_err(|error| log::warn!("runtime dos diálogos: {error}"))
                .ok()
        })
        .as_ref()
        .map(|runtime| runtime.handle().clone())
}

/// Monta o anexo a partir do caminho, adivinhando o mime pela extensão.
pub fn describe(path: &PathBuf) -> Upload {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "arquivo".into());
    let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    Upload {
        mime: mime_for(path),
        path: path.clone(),
        name,
        size,
    }
}

/// O backend confere o mime; aqui basta acertar a família.
pub fn mime_for(path: &Path) -> String {
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "pdf" => "application/pdf",
        "txt" | "md" | "log" => "text/plain",
        "zip" => "application/zip",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
    .to_owned()
}

/// Pasta de downloads do usuário, com o diretório pessoal como reserva.
pub fn downloads_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
        .or_else(|| directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()))
        .unwrap_or_else(std::env::temp_dir)
}

/// Nome livre dentro da pasta: `foto.png`, `foto (1).png`, …
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_owned());
    let extension = path
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    for index in 1..1000 {
        let candidate = dir.join(format!("{stem} ({index}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    candidate
}

/// Entrega o arquivo ao aplicativo padrão do sistema.
pub fn open_path(path: &Path) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}
