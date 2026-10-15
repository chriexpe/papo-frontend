//! Manual native UI fixture: cargo run --example channel-views-smoke.
//! Uses the real shell, composer and view implementation, with no Papo login.
use papo::{
    api::{
        models::ChannelView,
        net::Update,
        ws::{Connection, Event},
    },
    i18n::Lang,
    state::{Channel, ChannelKind, Screen, Server, Store},
    ui::{
        shell::{self, UiState},
        theme::{self, Appearance, Tokens},
    },
};

fn channel(id: &str, name: &str, url: &str, position: i32) -> Channel {
    Channel {
        id: id.into(),
        name: name.into(),
        kind: ChannelKind::Text,
        topic: None,
        position,
        permissions: Vec::new(),
        notification_settings: "off".into(),
        parent_id: None,
        unread: false,
        mentions: 0,
        views: vec![ChannelView {
            id: format!("view-{id}"),
            app_installation_id: "fixture".into(),
            channel_id: id.into(),
            url: url.into(),
            position: 0,
            name: Some(name.into()),
        }],
    }
}
struct Smoke {
    store: Store,
    ui: UiState,
    tokens: Tokens,
    overlay: bool,
}
impl Smoke {
    #[allow(clippy::field_reassign_with_default)] // Store/UiState also contain private fields.
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx, None);
        let tokens = Tokens::new(Appearance::Dark, None);
        theme::apply(&cc.egui_ctx, &tokens, false, 0.0);
        let mut store = Store::default();
        store.screen = Screen::Chat;
        store.connection = Connection::Online;
        store.me = "fixture-user".into();
        store.my_name = "Fixture".into();
        store.my_username = "fixture".into();
        store.selected_channel = "a".into();
        store.server = Some(Server {
            name: "Channel views smoke".into(),
            description: None,
            public: true,
            owner_id: Some("fixture-user".into()),
            icon: None,
        });
        store.channels = vec![
            channel("a", "Public", "https://example.com/", 0),
            channel("b", "Frame refusal", "https://github.com/", 1),
        ];
        let mut ui = UiState::default();
        ui.server_url = "https://papo.example".into();
        ui.last_channel = "a".into();
        ui.composer = "draft-preserved".into();
        Self {
            store,
            ui,
            tokens,
            overlay: false,
        }
    }
}
impl eframe::App for Smoke {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ui.channel_view.begin_frame(frame, &ctx);
        self.ui.webembed.set_context(&ctx);
        self.ui.webembed.begin_frame();
        self.ui.webembed.pump_events(&ctx);
        self.ui.webembed.prepare_render(frame);
        egui::Panel::top("fixture-controls").show(ui, |ui| {
            ui.horizontal(|ui| {
                for id in ["a", "b"] {
                    if ui.button(format!("Channel {id}")).clicked() {
                        self.store.selected_channel = id.into();
                    }
                }
                if ui.button("Revoke (channel_update)").clicked() {
                    self.store
                        .apply(Update::Event(Box::new(Event::ChannelUpdated {
                            id: self.store.selected_channel.clone(),
                            name: None,
                            topic: None,
                            views: Some(Vec::new()),
                        })));
                }
                if ui.button("Restore").clicked() {
                    self.store.channels = vec![
                        channel("a", "Public", "https://example.com/", 0),
                        channel("b", "Frame refusal", "https://github.com/", 1),
                    ];
                }
                ui.checkbox(&mut self.overlay, "Overlay");
            });
        });
        self.ui.webembed_blocked = self.overlay;
        shell::draw(
            ui,
            &mut self.store,
            &mut self.ui,
            None,
            &self.tokens,
            Lang::En.strings(),
            None,
            &[],
            0,
        );
        if self.overlay {
            egui::Window::new("Overlay fixture").show(&ctx, |ui| {
                ui.label("The native view must be hidden.");
            });
        }
        self.ui.channel_view.end_frame();
        self.ui.webembed.end_frame(
            papo::webembed::OffscreenBehavior::Float,
            papo::webembed::FloatScope::CurrentChannel,
            self.overlay,
        );
    }
}
fn main() -> eframe::Result<()> {
    eframe::run_native(
        "Channel views smoke",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 760.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(Smoke::new(cc)))),
    )
}
