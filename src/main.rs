#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod i18n;
mod media;
mod platform;
mod state;
mod ui;

/// Identificador do aplicativo na área de trabalho.
///
/// É o `app_id` do Wayland, o nome do `.desktop` e o do ícone. O compositor
/// liga a janela ao lançador por este nome, então os três têm de ser o mesmo
/// — inclusive dentro do Flatpak, onde o identificador é o do pacote.
pub const APP_ID: &str = "io.github.chriexpe.Papo";

fn main() -> eframe::Result<()> {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("warn,papo=debug"));
    install_panic_hook();

    // `papo selftest <usuário> <senha>` exercita o cliente REST contra o
    // backend sem abrir janela — útil para conferir o contrato.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("selftest") {
        selftest(args.get(2).cloned(), args.get(3).cloned());
        return Ok(());
    }

    // `papo media-test` gera um vídeo e um áudio de teste e os reproduz sem
    // abrir janela — é como se confere o motor de mídia sem depender do
    // backend.
    if args.get(1).map(String::as_str) == Some("media-test") {
        media_test();
        return Ok(());
    }

    // `papo demo` abre a janela com um servidor de mentira: é como se vê a
    // interface inteira enquanto o backend não responde.
    if args.get(1).map(String::as_str) == Some("demo") {
        std::env::set_var("PAPO_DEMO", "1");
    }

    // `papo pick-test` abre o seletor de arquivos e imprime o que voltou.
    if args.get(1).map(String::as_str) == Some("pick-test") {
        println!("abrindo o seletor…");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let picked = runtime.block_on(async {
            rfd::AsyncFileDialog::new()
                .set_title("Anexar")
                .pick_files()
                .await
        });
        println!(
            "resultado: {:?}",
            picked.map(|handles| handles
                .iter()
                .map(|handle| handle.path().to_path_buf())
                .collect::<Vec<_>>())
        );
        return Ok(());
    }

    // `papo notify-test` dispara uma notificação de exemplo e sai.
    #[cfg(target_os = "linux")]
    if args.get(1).map(String::as_str) == Some("notify-test") {
        if let Some(notifier) = platform::notify::Notifier::spawn() {
            notifier.show(platform::notify::Notification {
                summary: "ana · #geral".to_owned(),
                body: "bom dia, pessoal".to_owned(),
                tag: Some("teste".to_owned()),
            });
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Papo")
            .with_app_id(APP_ID)
            .with_inner_size([1160.0, 740.0])
            .with_min_inner_size([760.0, 480.0])
            .with_icon(window_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Papo",
        options,
        Box::new(|cc| Ok(Box::new(app::PapoApp::new(cc)))),
    )
}

/// Espera a thread do player publicar a duração.
fn wait_for_duration(player: &media::player::Player) -> f64 {
    for _ in 0..60 {
        let duration = player.duration();
        if duration > 0.0 {
            return duration;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    0.0
}

/// Guarda o pânico em disco antes de a janela sumir: sem isso, a única pista
/// vai embora junto com o terminal que lançou o programa.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let when = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        log::error!("pânico: {info}");
        if let Some(dirs) = directories::ProjectDirs::from("", "", "papo") {
            let dir = dirs.cache_dir();
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(
                dir.join("ultimo-panico.txt"),
                format!("{when}\n{info}\n\n{backtrace}\n"),
            );
        }
        previous(info);
    }));
}

/// Ícone da janela, o mesmo usado na bandeja.
fn window_icon() -> egui::IconData {
    let Ok(image) = image::load_from_memory(include_bytes!("../assets/icon.png")) else {
        return egui::IconData::default();
    };
    let image = image.to_rgba8();
    egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

/// Exercita o GStreamer: gera mídia, abre o player e lê um quadro.
fn media_test() {
    use gstreamer as gst;

    if !media::player::init() {
        println!("gstreamer: indisponível");
        return;
    }
    println!("gstreamer: {}", gst::version_string());

    let dir = std::env::temp_dir().join("papo-media-test");
    let _ = std::fs::create_dir_all(&dir);
    let video = dir.join("teste.ogv");
    let audio = dir.join("teste.ogg");

    // Três segundos de vídeo em Theora/Ogg: é o que dá para gerar sem
    // depender de codecs que podem não estar instalados.
    let pipeline = format!(
        "videotestsrc num-buffers=90 ! video/x-raw,width=640,height=360,framerate=30/1 ! \
         theoraenc ! oggmux ! filesink location=\"{}\"",
        video.display()
    );
    run_pipeline(&pipeline, "vídeo de teste");

    let pipeline = format!(
        "audiotestsrc num-buffers=140 wave=sine ! audioconvert ! vorbisenc ! oggmux ! \
         filesink location=\"{}\"",
        audio.display()
    );
    run_pipeline(&pipeline, "áudio de teste");

    let ctx = egui::Context::default();
    match media::player::Player::open(&video, true, ctx.clone()) {
        Some(mut player) => {
            player.play();
            std::thread::sleep(std::time::Duration::from_millis(1200));
            let duration = player.duration();
            let position = player.position();
            match player.frame(&ctx) {
                Some(texture) => {
                    let size = texture.size();
                    println!(
                        "vídeo: {}x{} · {position:.1}s de {duration:.1}s",
                        size[0], size[1]
                    );
                }
                None => println!("vídeo: abriu mas nenhum quadro chegou"),
            }
        }
        None => println!("vídeo: não deu para abrir"),
    }

    // Um player só, passando por tudo: tocar, pausar, buscar parado e voltar
    // a tocar. Buscar parado era o que congelava a janela.
    match media::player::Player::open(&audio, false, ctx.clone()) {
        Some(mut player) => {
            let duration = wait_for_duration(&player);
            player.play();
            std::thread::sleep(std::time::Duration::from_millis(700));
            let tocando = player.position();

            player.pause();
            let started = std::time::Instant::now();
            player.seek(duration * 0.5);
            let busca = started.elapsed();
            std::thread::sleep(std::time::Duration::from_millis(400));

            println!(
                "áudio: {duration:.1}s · tocando {tocando:.2}s · busca parada devolveu em \
                 {busca:?} → {:.2}s{}",
                player.position(),
                match player.error().as_deref() {
                    Some(error) => format!(" · erro: {error}"),
                    None => String::new(),
                }
            );
        }
        None => println!("áudio: não deu para abrir"),
    }

    match media::player::waveform(&audio) {
        Some(peaks) => println!(
            "forma de onda: {} picos (máximo {:.2})",
            peaks.len(),
            peaks.iter().copied().fold(0.0_f32, f32::max)
        ),
        None => println!("forma de onda: falhou"),
    }
}

fn run_pipeline(description: &str, label: &str) {
    use gstreamer as gst;
    use gstreamer::prelude::*;

    let Ok(pipeline) = gst::parse::launch(description) else {
        println!("{label}: pipeline inválido");
        return;
    };
    if pipeline.set_state(gst::State::Playing).is_err() {
        println!("{label}: não iniciou");
        return;
    }
    if let Some(bus) = pipeline.bus() {
        for message in bus.iter_timed(gst::ClockTime::from_seconds(30)) {
            match message.view() {
                gst::MessageView::Eos(_) => break,
                gst::MessageView::Error(error) => {
                    println!("{label}: {}", error.error());
                    break;
                }
                _ => {}
            }
        }
    }
    let _ = pipeline.set_state(gst::State::Null);
    println!("{label}: gerado");
}

/// Percorre registro, login e carga inicial, imprimindo o resultado.
fn selftest(username: Option<String>, password: Option<String>) {
    use api::client::{Api, Session};
    use std::sync::Arc;

    let base = std::env::var("PAPO_SERVER")
        .unwrap_or_else(|_| "https://papo-backend.onrender.com".to_owned());
    let runtime = tokio::runtime::Runtime::new().expect("runtime");

    runtime.block_on(async move {
        let session = Arc::new(Session::default());
        let api = Api::new(&base, Arc::clone(&session)).expect("api");
        println!("servidor: {base}");
        println!("websocket: {}", api.websocket_url());

        match api.server().await {
            Ok(Some(server)) => println!("servidor: {} ({} membros)", server.name, server.member_count),
            Ok(None) => println!("servidor: ainda não criado (GET /server -> 404)"),
            Err(error) => println!("servidor: erro {error}"),
        }

        let (Some(username), Some(password)) = (username, password) else {
            println!("sem credenciais: pare por aqui");
            return;
        };

        match api.login(&username, &password).await {
            Ok(_) => println!("login: ok"),
            Err(error) => {
                println!("login falhou ({error}); tentando registrar");
                match api.register(&username, &password).await {
                    Ok(_) => println!("registro: ok"),
                    Err(error) => {
                        println!("registro falhou: {error}");
                        return;
                    }
                }
                match api.login(&username, &password).await {
                    Ok(_) => println!("login: ok"),
                    Err(error) => {
                        println!("login falhou de novo: {error}");
                        return;
                    }
                }
            }
        }

        println!("cookie de sessão: {}", if session.is_authenticated() { "recebido" } else { "AUSENTE" });

        match api.whoami().await {
            Ok(me) => println!("whoami: {} ({})", me.display_name(), me.id),
            Err(error) => println!("whoami: erro {error}"),
        }
        match api.channels().await {
            Ok(channels) => println!("canais: {}", channels.len()),
            Err(error) => println!("canais: erro {error}"),
        }
        match api.users().await {
            Ok(users) => println!(
                "pessoas: {} ({})",
                users.len(),
                users.iter().map(|u| u.username.clone()).collect::<Vec<_>>().join(", ")
            ),
            Err(error) => println!("pessoas: erro {error}"),
        }
    });
}
