//! Global notification inbox. Each account keeps its own shared pager and exact cursor.
use super::*;

pub(super) fn load(ws: &mut Workspace, more: bool) {
    if ws.runtime.store.screen != Screen::Chat
        || ws.runtime.store.me.is_empty()
        || ws.runtime.store.connection != crate::api::ws::Connection::Online
    {
        return;
    }
    if let Ok(flight) = ws.runtime.store.inbox.begin(more) {
        ws.runtime.store.inbox_error = None;
        ws.runtime.net.send(Command::LoadNotificationInbox {
            owner: ws.runtime.store.me.clone(),
            epoch: ws.runtime.store.cache_restore_epoch(),
            flight,
        });
    }
}

pub(super) fn draw(
    ctx: &egui::Context,
    open: &mut bool,
    workspaces: &mut [Workspace],
    s: &crate::i18n::Strings,
) -> Option<(usize, String, String)> {
    let mut navigate = None;
    egui::Window::new(s.notification_inbox)
        .open(open)
        .collapsible(false)
        .default_width(540.0)
        .max_width((ctx.content_rect().width() - 24.0).max(200.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(ctx.content_rect().height() * 0.72)
                .show(ui, |ui| {
                    for (index, ws) in workspaces.iter_mut().enumerate() {
                        if ws.runtime.store.screen != Screen::Chat || ws.runtime.store.me.is_empty()
                        {
                            continue;
                        }
                        ui.push_id(ws.runtime.server_key.clone(), |ui| {
                            ui.horizontal(|ui| {
                                ui.heading(&ws.label);
                                if ui.button(s.inbox_refresh).clicked() {
                                    load(ws, false);
                                }
                                if ws.runtime.store.inbox.loading() {
                                    ui.spinner();
                                }
                            });
                            let store = &mut ws.runtime.store;
                            let read_ids: Vec<_> = store
                                .inbox
                                .items()
                                .iter()
                                .filter(|n| store.notification_was_read(n))
                                .map(|n| n.id.clone())
                                .collect();
                            store.inbox.mark_read(&read_ids);
                            let notices = store.inbox.items();
                            let mut shown = 0;
                            for notice in notices {
                                if notice.author_id.as_deref() == Some(&store.me) {
                                    continue;
                                }
                                shown += 1;
                                let author = notice
                                    .author_id
                                    .as_deref()
                                    .and_then(|id| store.member(id))
                                    .map(|m| m.name.as_str())
                                    .unwrap_or("");
                                let channel = notice
                                    .channel_id
                                    .as_deref()
                                    .and_then(|id| store.channel(id))
                                    .map(|c| c.name.as_str())
                                    .unwrap_or("");
                                let read = notice.read || store.notification_was_read(&notice);
                                let heading = format!(
                                    "{}  ·  {}{}",
                                    author,
                                    channel,
                                    if read { "" } else { "  •" }
                                );
                                let body = store.display_mentions(
                                    notice.message_content.as_deref().unwrap_or_default(),
                                );
                                let label = egui::RichText::new(format!("{heading}\n{body}"));
                                let available =
                                    notice.channel_id.is_some() && notice.message_id.is_some();
                                if ui
                                    .add_enabled(
                                        available,
                                        egui::Button::new(label)
                                            .wrap()
                                            .min_size(egui::vec2(ui.available_width(), 52.0)),
                                    )
                                    .clicked()
                                {
                                    navigate = Some((
                                        index,
                                        notice.channel_id.clone().unwrap(),
                                        notice.message_id.clone().unwrap(),
                                    ));
                                }
                            }
                            if shown == 0 && !store.inbox.loading() {
                                ui.label(s.inbox_empty);
                            }
                            if let Some(error) = &store.inbox_error {
                                ui.colored_label(ui.visuals().error_fg_color, error);
                            }
                            if store.inbox.has_more
                                && ui
                                    .add_enabled(
                                        !store.inbox.loading(),
                                        egui::Button::new(s.inbox_more),
                                    )
                                    .clicked()
                            {
                                load(ws, true);
                            }
                            ui.separator();
                        });
                    }
                });
        });
    navigate
}
