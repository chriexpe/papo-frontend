//! Runs the real app against a local plugin backend, with isolated UI preferences.
//! No alternate surface UI or tab controls; use the production server settings.
use std::collections::HashMap;
#[derive(Default)]
struct Seed(HashMap<String, String>);
impl eframe::Storage for Seed {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }
    fn flush(&mut self) {}
    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }
}
fn main() -> eframe::Result<()> {
    let _ = env_logger::try_init();
    let server = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:18087".into());
    let settings = papo::app::Settings {
        server_url: server.clone(),
        servers: vec![papo::app::ServerEntry {
            url: server,
            label: "Plugin smoke".into(),
        }],
        close_to_tray: false,
        ..Default::default()
    };
    let mut seed = Seed::default();
    eframe::set_value(&mut seed, eframe::APP_KEY, &settings);
    let path = std::env::temp_dir().join(format!(
        "papo-app-surfaces-smoke-{}.ron",
        std::process::id()
    ));
    std::fs::write(&path, serde_json::to_string(&seed.0).expect("preferences"))
        .expect("smoke preferences");
    let result = eframe::run_native(
        "Papo app surfaces smoke",
        eframe::NativeOptions {
            persistence_path: Some(path.clone()),
            renderer: papo::render::desktop_renderer(),
            viewport: egui::ViewportBuilder::default()
                .with_title("Papo app surfaces smoke")
                .with_inner_size([1160.0, 800.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(papo::app::PapoApp::new(cc)))),
    );
    let _ = std::fs::remove_file(path);
    result
}
