//! Porta de entrada do Android.
//!
//! O sistema abre a Activity, ela faz `dlopen` neste `.so` e chama
//! `android_main`. Daqui para dentro é o mesmo Papo da área de trabalho: a
//! interface compacta que já existe, sem nada reinventado para o celular.

use android_activity::AndroidApp;

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    // `println!` não sai em lugar nenhum no Android; o que se lê é o
    // `adb logcat -s papo`.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Debug)
            .with_tag("papo"),
    );
    install_panic_hook();

    // A pasta privada do aplicativo é a única que o sistema garante que
    // existe e que ninguém mais lê — é lá que fica a sessão.
    match app.internal_data_path() {
        Some(root) => crate::platform::dirs::set_root(root),
        None => log::error!("sem pasta privada: a sessão não vai sobreviver ao fechamento"),
    }

    // Campos egui que ainda usam GameTextInput.
    crate::platform::ime::install(app.clone());
    // Compositor e edição de mensagem usam um EditText Android de verdade.
    crate::platform::native_text::install(app.clone());
    // A ponte de volta: permissão e seletor de arquivos partem daqui.
    crate::platform::jvm::install(app.clone());

    // O eframe procura onde gravar pelas pastas do XDG, que no Android não
    // existem — sem isto ele desliga a persistência e os ajustes (servidores,
    // tema, idioma) se perdem a cada fechamento.
    let persistence_path = crate::platform::dirs::data_dir().map(|dir| {
        let _ = std::fs::create_dir_all(&dir);
        dir.join("app.ron")
    });
    if persistence_path.is_none() {
        log::error!("sem onde gravar: os ajustes não vão sobreviver ao fechamento");
    }

    // O GStreamer já foi iniciado pela Activity, antes de o Rust começar a
    // andar; aqui só se confere o que ele trouxe. Sai tudo no logcat:
    //   adb logcat -s papo
    crate::media::gst_check::run();

    let options = eframe::NativeOptions {
        android_app: Some(app),
        persistence_path,
        ..Default::default()
    };

    if let Err(error) = eframe::run_native(
        "Papo",
        options,
        Box::new(|cc| Ok(Box::new(crate::app::PapoApp::new(cc)))),
    ) {
        log::error!("a janela não abriu: {error}");
    }
}

/// No Android o pânico vai para o logcat: não há terminal para onde escrever
/// nem pasta de cache garantida no momento em que ele acontece.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!(
            "pânico: {info}\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        previous(info);
    }));
}
