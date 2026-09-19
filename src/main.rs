#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod i18n;
mod platform;
mod state;
mod ui;

fn main() -> eframe::Result<()> {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info,papo=debug"));

    // `papo selftest <usuário> <senha>` exercita o cliente REST contra o
    // backend sem abrir janela — útil para conferir o contrato.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("selftest") {
        selftest(args.get(2).cloned(), args.get(3).cloned());
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
            .with_app_id("papo")
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
