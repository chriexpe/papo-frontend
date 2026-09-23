//! Os comandos de linha e a abertura da janela na área de trabalho.

use papo::{APP_ID, api, app, media, platform, storage, voice};

pub fn main() -> eframe::Result<()> {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("warn,papo=debug"));
    install_panic_hook();

    // `papo selftest <usuário> <senha>` exercita o cliente REST contra o
    // backend sem abrir janela — útil para conferir o contrato.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("selftest") {
        selftest(args.get(2).cloned(), args.get(3).cloned());
        return Ok(());
    }

    // `papo poster <arquivo>` tira a capa de um vídeo e diz o que saiu. É
    // como se confere a extração sem passar pela interface.
    if args.get(1).map(String::as_str) == Some("poster") {
        let Some(file) = args.get(2) else {
            println!("uso: papo poster <arquivo>");
            return Ok(());
        };
        match media::player::poster(std::path::Path::new(file)) {
            Some(image) => println!("capa: {}x{}", image.size[0], image.size[1]),
            None => println!("capa: não saiu"),
        }
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
        // SAFETY: ainda estamos no início de `main`, antes de criar as
        // threads de rede, mídia, tray ou diálogos que poderiam ler o ambiente.
        unsafe {
            std::env::set_var("PAPO_DEMO", "1");
        }
    }

    // `papo pick-test` abre o seletor **duas vezes seguidas**, pelo mesmo
    // caminho que a janela usa. Duas é o número que importa: com um runtime
    // por diálogo, a primeira abria e a segunda nunca respondia.
    if args.get(1).map(String::as_str) == Some("pick-test") {
        let ctx = egui::Context::default();
        let mut dialogs = platform::files::Dialogs::default();
        for round in 1..=2 {
            println!("abrindo o seletor ({round} de 2)…");
            dialogs.pick_files(ctx.clone());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            loop {
                let answers = dialogs.poll();
                if let Some(answer) = answers.into_iter().next() {
                    // Sem `{:?}`: o anexo escolhido pode trazer um blob em
                    // base64 e o terminal viraria sopa.
                    let summary = match answer {
                        platform::files::Chosen::Files(files) => format!(
                            "{} arquivo(s): {}",
                            files.len(),
                            files
                                .iter()
                                .map(|file| file.name.clone())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        platform::files::Chosen::Folder(path) => format!("pasta {}", path.display()),
                        platform::files::Chosen::Image { format, blob, .. } => {
                            format!("imagem {format}, {} bytes em base64", blob.len())
                        }
                        platform::files::Chosen::SaveAs { dest, .. } => {
                            format!("salvar em {}", dest.display())
                        }
                        platform::files::Chosen::Cancelled => "cancelado".to_owned(),
                    };
                    println!("resultado {round}: {summary}");
                    break;
                }
                if std::time::Instant::now() > deadline {
                    println!("resultado {round}: o seletor não respondeu em 60s");
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        return Ok(());
    }

    // `papo voice-test <usuário> <senha> [canal]` entra de verdade numa call
    // do servidor em `PAPO_SERVER`, sem janela: é como se confere a
    // sinalização e o ICE contra uma instância viva.
    if args.get(1).map(String::as_str) == Some("voice-test") {
        voice_test(args.get(2).cloned(), args.get(3).cloned(), args.get(4).cloned());
        return Ok(());
    }

    // `papo voice-check` confere as peças da call — plugins, microfone,
    // alto-falante, câmera e o transporte — e diz o que falta. Dentro do
    // Flatpak:
    //   flatpak run --command=papo io.github.chriexpe.Papo voice-check
    if args.get(1).map(String::as_str) == Some("voice-check") {
        voice::check();
        return Ok(());
    }

    // `papo voice-sdp` monta o pipeline da call, imprime a oferta e sai.
    // É a conferência barata do contrato de mídia: quantas linhas `m=`,
    // quais codecs e que extensões vão no cabeçalho — sem servidor nenhum.
    if args.get(1).map(String::as_str) == Some("voice-sdp") {
        voice_sdp();
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

    // Onde não há menu global, a janela ganha a barra que desenhamos; onde
    // há (Plasma), quem desenha é o compositor.
    let own_chrome = !platform::desktop::uses_global_menu();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Papo")
            .with_app_id(APP_ID)
            .with_decorations(!own_chrome)
            .with_inner_size([1160.0, 740.0])
            .with_min_inner_size([360.0, 480.0])
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

/// Entra numa call de verdade e conta o que acontece: entrada, oferta,
/// resposta, ICE e o estado final da conexão. Sai sozinho em 25 segundos.
fn voice_test(username: Option<String>, password: Option<String>, channel: Option<String>) {
    use api::net::{Command, Net, Update, Wake};
    use api::ws::Event;

    let base = std::env::var("PAPO_SERVER")
        .unwrap_or_else(|_| "http://localhost:8080".to_owned());
    println!("servidor: {base}");

    let ctx = egui::Context::default();
    let net = Net::spawn(base, Wake::noop(), std::sync::Arc::new(storage::FileSecretStore::new()));
    // Sem credenciais, vale a sessão já guardada em disco — que é o caminho
    // preferido: senha no argv fica no histórico do shell e aparece para
    // quem listar os processos.
    match (username, password) {
        (Some(username), Some(password)) => net.send(Command::Login { username, password }),
        _ => println!("sem credenciais: usando a sessão já guardada"),
    }

    let mut call: Option<voice::Call> = None;
    let mut wanted = String::new();
    let mut asked = false;
    let mut live = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    let mut unmute = None;

    while std::time::Instant::now() < deadline {
        while let Some(update) = net.try_recv() {
            match update {
                Update::Session(Some(whoami)) => {
                    println!("entrou como {} ({})", whoami.username, whoami.id);
                }
                Update::Session(None) => println!("sem sessão"),
                Update::AuthFailed(error) => {
                    println!("entrada recusada: {error}");
                    return;
                }
                Update::Channels(channels) => {
                    if asked {
                        continue;
                    }
                    let voice_channel = channels.iter().find(|item| {
                        item.kind == "voice"
                            && channel.as_ref().is_none_or(|name| item.name == *name)
                    });
                    match voice_channel {
                        Some(item) => {
                            println!("canal de voz: {} ({})", item.name, item.id);
                            wanted = item.id.clone();
                            asked = true;
                            net.send(Command::JoinVoice {
                                channel_id: item.id.clone(),
                                attempt: 1,
                            });
                        }
                        None => println!("nenhum canal de voz neste servidor"),
                    }
                }
                Update::VoiceReady {
                    channel_id,
                    servers,
                    ..
                } => {
                    println!("ice: {} servidor(es)", servers.len());
                    call = voice::Call::start(
                        channel_id,
                        voice::IceConfig::from_servers(&servers),
                        ctx.clone(),
                        net.sender(),
                    );
                    if call.is_none() {
                        println!("a call não abriu");
                        return;
                    }
                }
                Update::Event(event) => match *event {
                    Event::VoiceJoined { members, .. } => {
                        println!("entrou na sala ({} pessoa(s))", members.len());
                        if let Some(call) = &call {
                            call.ready();
                        }
                        unmute = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
                    }
                    Event::VoiceAnswer { sdp, .. } => {
                        println!("resposta do servidor: {} bytes", sdp.len());
                        if let Some(call) = &call {
                            call.answer(sdp);
                        }
                    }
                    Event::VoiceCandidate {
                        candidate,
                        sdp_mline_index,
                        ..
                    } => {
                        if let Some(call) = &call {
                            call.candidate(candidate, sdp_mline_index.unwrap_or(0));
                        }
                    }
                    Event::VoiceState { state, .. } => println!(
                        "estado: {} mudo={} câmera={}",
                        state.user_id, state.muted, state.camera_on
                    ),
                    Event::ActiveSpeakers { user_ids, .. } => {
                        println!("falando: {user_ids:?}");
                    }
                    Event::Failure { message, code } => {
                        println!("erro do servidor: {message} ({code:?})");
                    }
                    _ => {}
                },
                Update::Error(error) => println!("rede: {error}"),
                _ => {}
            }
        }

        if let Some(call) = &call {
            if !live && call.is_live() {
                live = true;
                println!("conexão de mídia estabelecida");
            }
            if let Some(at) = unmute
                && std::time::Instant::now() > at
            {
                unmute = None;
                println!("abrindo o microfone");
                call.set_muted(false);
            }
            if let Some(warning) = call.take_warning() {
                println!("aviso: {warning}");
            }
            if let Some(error) = call.error() {
                println!("call: {error}");
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    if !wanted.is_empty() {
        net.send(Command::VoiceSignal(format!(
            r#"{{"type":"voice_leave","channel_id":"{wanted}"}}"#
        )));
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    println!(
        "fim: mídia {}",
        if live { "conectada" } else { "NÃO conectada" }
    );
}

/// Imprime a oferta que a call mandaria, sem rede e sem janela.
fn voice_sdp() {
    let (sender, mut commands) = api::net::NetSender::channel();
    let Some(call) = voice::Call::start(
        "canal-de-teste".to_owned(),
        voice::IceConfig::default(),
        egui::Context::default(),
        sender,
    ) else {
        println!("a call não abriu (falta GStreamer ou o webrtcbin)");
        return;
    };
    call.ready();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        while let Ok(command) = commands.try_recv() {
            let Command::VoiceSignal(signal) = command else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&signal) else {
                continue;
            };
            let kind = value.get("type").and_then(serde_json::Value::as_str);
            if kind == Some("voice_offer") {
                let sdp = value.get("sdp").and_then(serde_json::Value::as_str).unwrap_or("");
                let lines = sdp.lines().filter(|line| line.starts_with("m=")).count();
                println!("{sdp}");
                println!("— {lines} linhas de mídia —");
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    println!("a oferta não saiu em 10s: {:?}", call.error());
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
