//! Seletor de arquivos, pasta de downloads e "abrir com o aplicativo padrão".
//!
//! Tudo passa pelo xdg-desktop-portal quando ele existe, então quem aparece é
//! o diálogo do próprio sistema — não um desenhado por nós.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
#[cfg(target_os = "android")]
use std::sync::Mutex;

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
        /// O arquivo precisou ser reduzido para caber.
        shrunk: bool,
        /// Nome dado no editor (a figurinha sobe já com ele).
        name: Option<String>,
    },
    /// Imagem escolhida que ainda passa pelo editor de recorte: o arquivo
    /// original e uma prévia reduzida para desenhar.
    Crop {
        purpose: ImagePick,
        path: PathBuf,
        name: String,
        /// Tamanho do original, em pixels.
        size: [u32; 2],
        preview: egui::ColorImage,
        /// GIF animado: os quadros reduzidos e a espera de cada um, em
        /// segundos, para o editor mostrar a animação. Vazio numa imagem
        /// parada.
        frames: Vec<(egui::ColorImage, f32)>,
    },
    /// A imagem não abriu (arquivo que não é imagem, corrompido...).
    Unreadable,
    Folder(PathBuf),
    /// Destino de um anexo que estava esperando o "salvar como".
    SaveAs { id: String, name: String, dest: PathBuf },
    /// Destino escolhido para uma imagem de link/embed que já existe no cache.
    SaveCached { source: PathBuf, dest: PathBuf },
    Cancelled,
}

/// Para que serve a imagem que está sendo escolhida. Cada uma tem o seu
/// limite, que é o do endpoint correspondente do backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImagePick {
    Avatar,
    /// Banner do perfil, sempre 3:1.
    Banner,
    /// Ícone do servidor.
    ServerIcon,
    /// Figurinha do servidor. O nome é pedido depois de escolher o arquivo:
    /// digitar o nome antes de ver a imagem era pedir na ordem errada.
    Sticker,
    /// Arte da atividade manual do Rich Presence. Fica só neste aparelho.
    ActivityArt,
}

impl ImagePick {
    /// Caixa de saída e peso máximo. Os números são os que o backend confere
    /// (`services/users.go`, `servers.go`, `emojis.go`): 2 MB e 512 px para
    /// foto e ícone, 2 MB e 2048 px para banner, 256 KB e 512 px para
    /// figurinha. O banner sai em 1536 × 512, bem dentro do teto e nítido
    /// em telas 3x.
    pub fn limits(self) -> (u32, u32, usize) {
        match self {
            Self::Avatar | Self::ServerIcon | Self::ActivityArt => (512, 512, 2 * 1024 * 1024),
            Self::Banner => (1536, 512, 2 * 1024 * 1024),
            Self::Sticker => (512, 512, 256 * 1024),
        }
    }

    /// Proporção do recorte (largura ÷ altura).
    pub fn aspect(self) -> Option<f32> {
        match self {
            // Figurinha começa quadrada; o editor deixa trocar por "Original".
            Self::Avatar | Self::ServerIcon | Self::Sticker | Self::ActivityArt => Some(1.0),
            Self::Banner => Some(3.0),
        }
    }
}

/// Lado maior da prévia do editor. O recorte de verdade é feito no original.
const PREVIEW_MAX: u32 = 1600;

/// Depois de escolher: figurinha vai direto para o encolhimento; o resto
/// abre o editor com uma prévia.
fn after_pick(path: &Path, name: String, purpose: ImagePick) -> Chosen {
    let image = match image::open(path) {
        Ok(image) => image,
        Err(error) => {
            log::warn!("imagem não abriu: {error}");
            return Chosen::Unreadable;
        }
    };
    let size = [image.width(), image.height()];
    let small = image.thumbnail(PREVIEW_MAX, PREVIEW_MAX).to_rgba8();
    let preview = egui::ColorImage::from_rgba_unmultiplied(
        [small.width() as usize, small.height() as usize],
        small.as_raw(),
    );
    Chosen::Crop {
        purpose,
        path: path.to_path_buf(),
        name,
        size,
        preview,
        frames: animation_frames(path),
    }
}

/// Pixels somados de todos os quadros da prévia animada (~48 MB em RGBA).
/// Um GIF longo sai com quadros menores em vez de estourar a memória.
const ANIMATION_BUDGET: f32 = 12_000_000.0;

/// Quadros de um GIF animado, reduzidos, para a prévia do editor. Vazio para
/// qualquer outra coisa (inclusive GIF de um quadro só).
fn animation_frames(path: &Path) -> Vec<(egui::ColorImage, f32)> {
    use image::AnimationDecoder as _;
    let Ok(raw) = std::fs::read(path) else {
        return Vec::new();
    };
    if image::guess_format(&raw).ok() != Some(image::ImageFormat::Gif) {
        return Vec::new();
    }
    let Ok(decoder) = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(raw)) else {
        return Vec::new();
    };
    let Ok(frames) = decoder.into_frames().collect_frames() else {
        return Vec::new();
    };
    if frames.len() <= 1 {
        return Vec::new();
    }
    let side = (ANIMATION_BUDGET / frames.len() as f32).sqrt().clamp(96.0, 480.0) as u32;
    frames
        .into_iter()
        .map(|frame| {
            let (numer, denom) = frame.delay().numer_denom_ms();
            // Navegadores tratam espera abaixo de 20 ms como 100 ms; sem
            // isso, GIF "rápido demais" vira borrão.
            let mut delay = numer as f32 / denom.max(1) as f32 / 1000.0;
            if delay < 0.02 {
                delay = 0.1;
            }
            let small = image::DynamicImage::ImageRgba8(frame.into_buffer())
                .thumbnail(side, side)
                .to_rgba8();
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [small.width() as usize, small.height() as usize],
                small.as_raw(),
            );
            (image, delay)
        })
        .collect()
}

/// O que vale nos dois lados: recortar e preparar a imagem que saiu do
/// editor, numa thread à parte, com a resposta pela mesma fila dos diálogos.
impl Dialogs {
    pub fn prepare_crop(
        &mut self,
        repaint: egui::Context,
        purpose: ImagePick,
        path: PathBuf,
        crop: crate::media::prepare::Crop,
        name: Option<String>,
    ) {
        let (tx, rx) = mpsc::channel();
        self.pending.push(rx);
        std::thread::spawn(move || {
            let (max_width, max_height, max_bytes) = purpose.limits();
            let answer = match crate::media::prepare::fit_cropped(
                &path,
                Some(crop),
                max_width,
                max_height,
                max_bytes,
            ) {
                Ok(prepared) => {
                    log::info!(
                        "imagem pronta: {}x{} {} · {} KB",
                        prepared.width,
                        prepared.height,
                        prepared.format,
                        prepared.bytes / 1024
                    );
                    Chosen::Image {
                        purpose,
                        blob: prepared.blob,
                        format: prepared.format.to_owned(),
                        shrunk: prepared.shrunk,
                        name,
                    }
                }
                Err(error) => {
                    log::warn!("imagem recusada: {error}");
                    Chosen::Unreadable
                }
            };
            let _ = tx.send(answer);
            repaint.request_repaint();
        });
    }
}

/// Diálogos abertos, à espera de resposta.
#[derive(Default)]
pub struct Dialogs {
    pending: Vec<mpsc::Receiver<Chosen>>,
}

/// O que vale nos dois lados: recolher as respostas que já chegaram.
impl Dialogs {
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

// Os diálogos de verdade são os do sistema, entregues pelo
// xdg-desktop-portal. No Android esse portal não existe: o seletor de lá é
// uma Intent, e ela é assunto do PR de integração com a plataforma.
#[cfg(not(target_os = "android"))]
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

    /// Mídia visual para o compositor. O desktop não usa este caminho hoje,
    /// mas mantê-lo real deixa o ChatAction exaustivo sem um fallback surpresa.
    pub fn pick_gallery(&mut self, repaint: egui::Context) {
        self.spawn(repaint, |dialog| async move {
            match dialog
                .set_title("Galeria")
                .add_filter(
                    "Fotos e vídeos",
                    &["png", "jpg", "jpeg", "gif", "webp", "avif", "mp4", "webm", "mov", "m4v"],
                )
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

    /// O mesmo seletor, filtrado nos formatos que animam. Uma busca de GIF
    /// de verdade (Tenor, Giphy) precisa de chave e de um proxy no backend;
    /// até lá, o arquivo vem do disco.
    pub fn capture_media(&mut self, repaint: egui::Context) {
        self.pick_gallery(repaint);
    }

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

    pub fn save_cached_as(
        &mut self,
        repaint: egui::Context,
        source: PathBuf,
        name: String,
        start: PathBuf,
    ) {
        self.spawn(repaint, move |dialog| async move {
            match dialog
                .set_directory(start)
                .set_file_name(&name)
                .set_title("Salvar como")
                .save_file()
                .await
            {
                Some(handle) => Chosen::SaveCached {
                    source,
                    dest: handle.path().to_path_buf(),
                },
                None => Chosen::Cancelled,
            }
        });
    }

    /// Imagem para foto de perfil ou figurinha.
    ///
    /// O arquivo é encolhido e convertido aqui, antes de subir: o servidor
    /// recusa o que passa do limite, e mandar o usuário achar uma imagem
    /// menor sozinho é empurrar para ele um trabalho que a máquina faz.
    pub fn pick_image(&mut self, repaint: egui::Context, purpose: ImagePick) {
        self.spawn(repaint, move |dialog| async move {
            let Some(handle) = dialog
                .set_title("Imagem")
                .add_filter("Imagens", &["png", "jpg", "jpeg", "gif", "webp"])
                .pick_file()
                .await
            else {
                return Chosen::Cancelled;
            };
            // Decodificar uma foto de câmera leva uns décimos: fora da thread
            // que conversa com o portal.
            let path = handle.path().to_path_buf();
            let name = handle.file_name();
            tokio::task::spawn_blocking(move || after_pick(&path, name, purpose))
                .await
                .unwrap_or(Chosen::Unreadable)
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

}

/// Por onde a resposta do seletor volta.
///
/// A escolha acontece noutra Activity e a resposta chega numa thread do
/// Java, muito depois de `pick_files` ter voltado. O canal fica guardado
/// aqui até lá; o `poll` do lado da janela não muda em nada.
#[cfg(target_os = "android")]
static ANSWER: Mutex<Option<mpsc::Sender<Chosen>>> = Mutex::new(None);
#[cfg(target_os = "android")]
static CLIPBOARD_ATTACHMENTS: Mutex<Vec<Upload>> = Mutex::new(Vec::new());

/// Recebe os anexos escolhidos, já copiados para o cache pela Activity.
///
/// # Safety
/// Chamada pelo JNI.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeFilesPicked(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    paths: jni::objects::JObjectArray,
    names: jni::objects::JObjectArray,
) {
    let count = env.get_array_length(&paths).unwrap_or(0);
    let mut files = Vec::new();
    for index in 0..count {
        let Ok(path) = env.get_object_array_element(&paths, index) else {
            continue;
        };
        let Ok(name) = env.get_object_array_element(&names, index) else {
            continue;
        };
        let (path, name): (jni::objects::JString, jni::objects::JString) =
            (path.into(), name.into());
        let (Ok(path), Ok(name)) = (env.get_string(&path), env.get_string(&name)) else {
            continue;
        };
        let path = PathBuf::from(String::from(path));
        // O nome vem do provedor, e é o que a pessoa reconhece; o do arquivo
        // no cache leva um carimbo de tempo na frente para não colidir.
        let mut upload = describe(&path);
        upload.name = String::from(name);
        files.push(upload);
    }

    log::info!("seletor: {} arquivo(s)", files.len());
    let answer = if files.is_empty() {
        Chosen::Cancelled
    } else {
        Chosen::Files(files)
    };
    if let Ok(mut slot) = ANSWER.lock()
        && let Some(sender) = slot.take()
    {
        let _ = sender.send(answer);
    }
    super::wake::request();
}

/// Recebe arquivos/imagens colados no compositor Android.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeClipboardFilesPasted(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    paths: jni::objects::JObjectArray,
    names: jni::objects::JObjectArray,
) {
    let count = env.get_array_length(&paths).unwrap_or(0);
    let mut uploads = Vec::new();
    for index in 0..count {
        let Ok(path) = env.get_object_array_element(&paths, index) else {
            continue;
        };
        let Ok(name) = env.get_object_array_element(&names, index) else {
            continue;
        };
        let (path, name): (jni::objects::JString, jni::objects::JString) =
            (path.into(), name.into());
        let (Ok(path), Ok(name)) = (env.get_string(&path), env.get_string(&name)) else {
            continue;
        };
        let path = PathBuf::from(String::from(path));
        let mut upload = describe(&path);
        upload.name = String::from(name);
        uploads.push(upload);
    }
    if !uploads.is_empty() {
        if let Ok(mut pending) = CLIPBOARD_ATTACHMENTS.lock() {
            pending.extend(uploads);
        }
        super::wake::request();
    }
}

#[cfg(target_os = "android")]
pub fn take_clipboard_attachments() -> Vec<Upload> {
    CLIPBOARD_ATTACHMENTS
        .lock()
        .map(|mut pending| std::mem::take(&mut *pending))
        .unwrap_or_default()
}

#[cfg(target_os = "android")]
impl Dialogs {
    /// Os diálogos que ainda não existem no Android. Responder `Cancelled`
    /// na hora é o que mantém a interface honesta: quem pediu recebe a
    /// recusa no mesmo quadro em vez de esperar para sempre.
    fn unavailable(&mut self, repaint: egui::Context) {
        log::warn!("este diálogo ainda não existe no Android");
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(Chosen::Cancelled);
        self.pending.push(rx);
        repaint.request_repaint();
    }

    pub fn pick_files(&mut self, _repaint: egui::Context) {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut slot) = ANSWER.lock() {
            // Uma escolha de cada vez: a anterior, se ficou pendurada, some
            // aqui e o receptor dela morre sozinho no `poll`.
            *slot = Some(tx);
        }
        self.pending.push(rx);
        if !super::jvm::call_activity("pickFiles", "()V", None) {
            log::warn!("o seletor de arquivos não abriu");
        }
    }

    pub fn pick_gallery(&mut self, _repaint: egui::Context) {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut slot) = ANSWER.lock() {
            *slot = Some(tx);
        }
        self.pending.push(rx);
        if !super::jvm::call_activity("pickGallery", "()V", None) {
            log::warn!("o seletor da galeria não abriu");
        }
    }

    pub fn capture_media(&mut self, _repaint: egui::Context) {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut slot) = ANSWER.lock() {
            *slot = Some(tx);
        }
        self.pending.push(rx);
        if !super::jvm::call_activity("captureMedia", "()V", None) {
            log::warn!("a câmera não abriu");
        }
    }

    pub fn pick_animations(&mut self, repaint: egui::Context) {
        self.unavailable(repaint);
    }

    pub fn pick_folder(&mut self, repaint: egui::Context, _start: PathBuf) {
        self.unavailable(repaint);
    }

    pub fn save_as(
        &mut self,
        repaint: egui::Context,
        _id: String,
        _name: String,
        _start: PathBuf,
    ) {
        self.unavailable(repaint);
    }

    /// Imagem pelo mesmo seletor dos anexos: a Activity já copia o arquivo
    /// para o cache, e daí em diante o caminho é o do desktop — prévia,
    /// editor de recorte, preparo.
    pub fn pick_image(&mut self, repaint: egui::Context, purpose: ImagePick) {
        let (picked_tx, picked_rx) = mpsc::channel();
        if let Ok(mut slot) = ANSWER.lock() {
            *slot = Some(picked_tx);
        }
        let (tx, rx) = mpsc::channel();
        self.pending.push(rx);
        if !super::jvm::call_activity("pickFiles", "()V", None) {
            log::warn!("o seletor de arquivos não abriu");
        }
        std::thread::spawn(move || {
            let answer = match picked_rx.recv() {
                Ok(Chosen::Files(files)) => match files.into_iter().next() {
                    Some(upload) => after_pick(&upload.path, upload.name, purpose),
                    None => Chosen::Cancelled,
                },
                _ => Chosen::Cancelled,
            };
            let _ = tx.send(answer);
            repaint.request_repaint();
        });
    }

    pub fn save_cached_as(
        &mut self,
        repaint: egui::Context,
        _source: PathBuf,
        _name: String,
        _start: PathBuf,
    ) {
        self.unavailable(repaint);
    }
}


/// O runtime que conduz o D-Bus dos diálogos, criado uma vez e nunca
/// derrubado. Ver a explicação em `Dialogs::spawn`.
#[cfg(not(target_os = "android"))]
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
        spoiler: false,
        anonymize: false,
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
    #[cfg(not(target_os = "android"))]
    {
        directories::UserDirs::new()
            .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
            .or_else(|| directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()))
            .unwrap_or_else(std::env::temp_dir)
    }
    // No Android a pasta pública de downloads exige permissão e passa pelo
    // MediaStore. Até o PR de plataforma, o destino fica dentro da própria
    // área do aplicativo, que não precisa pedir nada a ninguém.
    #[cfg(target_os = "android")]
    {
        let dir = crate::platform::dirs::data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("downloads");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }
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
    #[cfg(not(target_os = "android"))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    // Abrir um arquivo no Android é uma Intent, e para isso é preciso a
    // Activity — assunto do PR de integração com a plataforma.
    #[cfg(target_os = "android")]
    log::warn!("abrir {} ainda não existe no Android", path.display());
}
