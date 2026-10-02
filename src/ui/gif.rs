//! Native Papo UI for KLIPY GIFs.

use egui::{Align, Color32, CornerRadius, Id, Layout, Rect, RichText, Sense, Stroke, UiBuilder, Vec2};
use egui_phosphor::regular as icon;

use crate::klipy::{BrowseMode, Category, GifItem};
use crate::i18n::Strings;

use super::emoji;
use super::shell::{ChatAction, LinkViewer, UiState};
use super::theme::{radius, space, text, Tokens};

const PICKER_W: f32 = 430.0;
const PICKER_H: f32 = 520.0;
const SEARCH_H: f32 = 38.0;
const TILE_GAP: f32 = 8.0;
const RESULT_H: f32 = 126.0;
const CATEGORY_H: f32 = 104.0;

pub fn picker_popup(ui: &mut egui::Ui, state: &mut UiState, t: &Tokens, s: &Strings) {
    let Some(opened) = state.gif_picker_opened else {
        return;
    };
    let Some(anchor) = state.gif_picker_anchor else {
        state.gif_picker_opened = None;
        return;
    };
    if state.klipy.is_none() {
        state.klipy = Some(crate::klipy::Store::new(ui.ctx().clone()));
    }

    let safe = ui.ctx().content_rect();
    let size = Vec2::new(
        PICKER_W.min((safe.width() - space::XL).max(260.0)),
        PICKER_H.min((safe.height() - space::XL).max(300.0)),
    );
    let rect = emoji::popup_area(ui, anchor, size);
    let backdrop = ui.interact(
        safe,
        Id::new("klipy-picker-backdrop"),
        Sense::click(),
    );

    ui.painter().rect(
        rect,
        CornerRadius::same(radius::SHEET),
        t.elevated_bg,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink(space::SM);
    let search_rect = Rect::from_min_max(
        egui::pos2(inner.min.x, inner.max.y - SEARCH_H),
        inner.max,
    );
    let body_rect = Rect::from_min_max(
        inner.min,
        egui::pos2(inner.max.x, search_rect.min.y - space::SM),
    );

    let mut klipy = state.klipy.take().expect("KLIPY store initialized");
    klipy.pump(ui.ctx());

    let mut body = ui.new_child(
        UiBuilder::new()
            .max_rect(body_rect)
            .layout(Layout::top_down(Align::Min)),
    );

    if !crate::klipy::available() {
        body.add_space(space::LG);
        body.label(
            RichText::new(s.gif_unavailable)
                .font(text::body())
                .color(t.label_secondary),
        );
    } else {
        match klipy.browser.mode.clone() {
            BrowseMode::Home => {
                draw_home(&mut body, state, t, s, &mut klipy);
            }
            BrowseMode::Favourites => {
                draw_results_header(&mut body, t, s.gif_favourites, &mut klipy);
                draw_favourites(&mut body, state, t, s, &mut klipy);
            }
            BrowseMode::Trending => {
                draw_results_header(&mut body, t, s.gif_trending, &mut klipy);
                draw_results(&mut body, state, t, s, &mut klipy);
            }
            BrowseMode::Category { name, .. } => {
                draw_results_header(&mut body, t, &name, &mut klipy);
                draw_results(&mut body, state, t, s, &mut klipy);
            }
            BrowseMode::Search { query } => {
                let title = if query.is_empty() { s.gif_results } else { &query };
                draw_results_header(&mut body, t, title, &mut klipy);
                draw_results(&mut body, state, t, s, &mut klipy);
            }
        }
    }

    draw_search(ui, state, t, s, &mut klipy, search_rect);

    if let Some(error) = klipy.browser.error.as_deref() {
        let error_rect = Rect::from_min_max(
            egui::pos2(rect.min.x + space::LG, search_rect.min.y - 30.0),
            egui::pos2(rect.max.x - space::LG, search_rect.min.y - 4.0),
        );
        ui.painter().text(
            error_rect.left_center(),
            egui::Align2::LEFT_CENTER,
            error,
            text::footnote(),
            t.danger,
        );
    }

    state.klipy = Some(klipy);

    let now = ui.input(|input| input.time);
    let outside = now > opened + 0.05
        && backdrop.clicked()
        && backdrop
            .interact_pointer_pos()
            .is_some_and(|position| !rect.contains(position));
    let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
    if outside || escape {
        state.gif_picker_opened = None;
        state.gif_picker_anchor = None;
    }
}

fn draw_results_header(
    ui: &mut egui::Ui,
    t: &Tokens,
    title: &str,
    klipy: &mut crate::klipy::Store,
) {
    ui.horizontal(|ui| {
        let (back_rect, back) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
        if back.hovered() {
            ui.painter()
                .rect_filled(back_rect, CornerRadius::same(radius::CONTROL), t.fill_soft);
        }
        ui.painter().text(
            back_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon::CARET_LEFT,
            text::icon(15.0),
            t.label_secondary,
        );
        if back.clicked() {
            klipy.show_home();
        }
        ui.label(RichText::new(title).font(text::headline()).color(t.label));
    });
    ui.add_space(space::SM);
}

fn draw_home(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    klipy: &mut crate::klipy::Store,
) {
    ui.label(
        RichText::new(s.gif_categories.to_uppercase())
            .font(text::caption())
            .color(t.label_tertiary),
    );
    ui.add_space(space::SM);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width();
            let tile_w = ((width - TILE_GAP) / 2.0).max(100.0);

            let favourite_preview = state
                .gif_favourites
                .iter()
                .next()
                .cloned()
                .and_then(|slug| klipy.item(&slug, ui.ctx()));
            let trending_preview = klipy.browser.trending_cover.clone();

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = TILE_GAP;
                if category_tile(
                    ui,
                    state,
                    t,
                    "favourites",
                    s.gif_favourites,
                    favourite_preview.as_ref().map(|item| item.preview_url.as_str()),
                    tile_w,
                    CATEGORY_H,
                    icon::STAR,
                ) {
                    klipy.show_favourites();
                }
                if category_tile(
                    ui,
                    state,
                    t,
                    "trending",
                    s.gif_trending,
                    trending_preview.as_ref().map(|item| item.preview_url.as_str()),
                    tile_w,
                    CATEGORY_H,
                    icon::TREND_UP,
                ) {
                    klipy.show_trending();
                }
            });
            ui.add_space(TILE_GAP);

            let categories = klipy.browser.categories.clone();
            for row in categories.chunks(2) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = TILE_GAP;
                    for category in row {
                        if klipy_category_tile(ui, state, t, category, tile_w) {
                            klipy.show_category(category.name.clone(), category.query.clone());
                        }
                    }
                });
                ui.add_space(TILE_GAP);
            }
        });
}

fn category_tile(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    id: &str,
    label: &str,
    preview_url: Option<&str>,
    width: f32,
    height: f32,
    glyph: &str,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::CARD), t.fill_soft);

    if let Some(url) = preview_url
        && let Some(texture) = state
            .media
            .remote_ephemeral(&format!("klipy-category-{id}"), url)
            .and_then(|texture| texture.first_frame())
            .cloned()
    {
        paint_cover(ui, rect, &texture);
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CARD), Color32::from_black_alpha(90));
    }

    ui.painter().text(
        rect.center() - Vec2::new(0.0, 9.0),
        egui::Align2::CENTER_CENTER,
        glyph,
        text::icon(18.0),
        Color32::WHITE,
    );
    ui.painter().text(
        rect.center() + Vec2::new(0.0, 14.0),
        egui::Align2::CENTER_CENTER,
        label,
        text::subheadline(),
        Color32::WHITE,
    );
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(radius::CARD),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    response.clicked()
}

fn klipy_category_tile(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    category: &Category,
    width: f32,
) -> bool {
    category_tile(
        ui,
        state,
        t,
        &format!("category-{}", category.query),
        &category.name,
        Some(&category.preview_url),
        width,
        CATEGORY_H,
        icon::GIF,
    )
}

fn draw_favourites(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    klipy: &mut crate::klipy::Store,
) {
    let slugs: Vec<String> = state.gif_favourites.iter().cloned().collect();
    if slugs.is_empty() {
        ui.label(
            RichText::new(s.gif_no_favourites)
                .font(text::body())
                .color(t.label_secondary),
        );
        return;
    }

    let mut items = Vec::new();
    for slug in slugs {
        if let Some(item) = klipy.item(&slug, ui.ctx()) {
            items.push(item);
        }
    }
    draw_item_grid(ui, state, t, s, klipy, &items, false);
}

fn draw_results(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    klipy: &mut crate::klipy::Store,
) {
    let items = klipy.browser.results.clone();
    draw_item_grid(ui, state, t, s, klipy, &items, true);
}

fn draw_item_grid(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    klipy: &mut crate::klipy::Store,
    items: &[GifItem],
    paged: bool,
) {
    let mut chosen: Option<GifItem> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width();
            let tile_w = ((width - TILE_GAP) / 2.0).max(100.0);
            for row in items.chunks(2) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = TILE_GAP;
                    for item in row {
                        if gif_tile(ui, state, t, item, tile_w, RESULT_H) {
                            chosen = Some(item.clone());
                        }
                    }
                });
                ui.add_space(TILE_GAP);
            }
            if paged && klipy.browser.has_more {
                let loading = klipy.browser.loading;
                if ui
                    .add_enabled(
                        !loading,
                        egui::Button::new(if loading { s.downloading } else { s.gif_more }),
                    )
                    .clicked()
                {
                    klipy.load_more();
                }
            }
        });

    if let Some(item) = chosen {
        send_item(state, klipy, item);
    }
}

fn gif_tile(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    item: &GifItem,
    width: f32,
    height: f32,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::CARD), t.fill_soft);

    if let Some(texture) = state
        .media
        .remote_ephemeral(&format!("klipy-picker-{}", item.slug), &item.preview_url)
        .and_then(|texture| texture.frame(ui.ctx()))
        .cloned()
    {
        paint_cover(ui, rect, &texture);
    } else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            icon::GIF,
            text::icon(24.0),
            t.label_tertiary,
        );
    }
    if response.hovered() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius::CARD),
            Stroke::new(1.0, t.accent),
            egui::StrokeKind::Inside,
        );
    }
    response.clicked()
}

fn paint_cover(ui: &egui::Ui, rect: Rect, texture: &egui::TextureHandle) {
    let size = texture.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let source_ratio = size.x / size.y;
    let target_ratio = rect.width() / rect.height();
    let uv = if source_ratio > target_ratio {
        let visible = target_ratio / source_ratio;
        let edge = (1.0 - visible) / 2.0;
        Rect::from_min_max(egui::pos2(edge, 0.0), egui::pos2(1.0 - edge, 1.0))
    } else {
        let visible = source_ratio / target_ratio;
        let edge = (1.0 - visible) / 2.0;
        Rect::from_min_max(egui::pos2(0.0, edge), egui::pos2(1.0, 1.0 - edge))
    };
    ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
}

fn draw_search(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    klipy: &mut crate::klipy::Store,
    rect: Rect,
) {
    ui.painter().rect(
        rect,
        CornerRadius::same(radius::FIELD),
        t.fill_soft,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    let before = klipy.browser.query.clone();

    #[cfg(target_os = "android")]
    {
        let key = "klipy-search";
        let _ = crate::platform::native_field::show(
            ui.ctx(),
            key,
            &mut klipy.browser.query,
            rect.shrink2(Vec2::new(space::MD, space::SM)),
            s.gif_search_klipy,
            crate::platform::native_field::Mode::Search,
            0,
            false,
            t.label,
            t.label_tertiary,
            text::body().size,
        );
    }

    #[cfg(not(target_os = "android"))]
    {
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect.shrink2(Vec2::new(space::XS, space::XXS)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        child.add(
            egui::TextEdit::singleline(&mut klipy.browser.query)
                .hint_text(s.gif_search_klipy)
                .desired_width(f32::INFINITY)
                .frame(false)
                .margin(egui::Margin::symmetric(space::MD as i8, space::SM as i8)),
        );
    }

    if klipy.browser.query != before {
        klipy.query_changed(ui.input(|input| input.time));
    }
    klipy.maybe_submit_search(ui.input(|input| input.time));

    // Search debounce needs another frame even when nothing else animates.
    if !klipy.browser.query.is_empty() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
    }

    let _ = state;
}

fn send_item(state: &mut UiState, klipy: &mut crate::klipy::Store, item: GifItem) {
    let query = match &klipy.browser.mode {
        BrowseMode::Search { query } | BrowseMode::Category { query, .. } => query.as_str(),
        _ => "",
    };
    klipy.register_share(&item.slug, query);
    state.actions.push(ChatAction::Send {
        content: crate::klipy::encode_message(&item.slug),
        reply_to: state.replying.take(),
        notify_reply: state.reply_notify,
        attachments: Vec::new(),
    });
    state.gif_picker_opened = None;
    state.gif_picker_anchor = None;
}

pub fn message(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    slug: &str,
    width: f32,
) {
    if state.klipy.is_none() {
        state.klipy = Some(crate::klipy::Store::new(ui.ctx().clone()));
    }
    let mut klipy = state.klipy.take().expect("KLIPY store initialized");
    klipy.pump(ui.ctx());
    let item = klipy.item(slug, ui.ctx());

    let max_w = width.min(420.0);
    let size = item
        .as_ref()
        .map(|item| {
            let natural = Vec2::new(item.width.max(1) as f32, item.height.max(1) as f32);
            let scale = (max_w / natural.x).min(300.0 / natural.y).min(1.0);
            Vec2::new((natural.x * scale).max(160.0), (natural.y * scale).max(90.0))
        })
        .unwrap_or_else(|| Vec2::new(max_w.min(300.0), 170.0));

    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::CARD), t.fill_soft);

    if let Some(item) = item.as_ref() {
        // Chat GIFs autoplay exactly like the picker. Keep the provider media
        // ephemeral, but advance animated frames while the message is visible.
        if let Some(texture) = state
            .media
            .remote_ephemeral(&format!("klipy-chat-{slug}"), &item.preview_url)
            .and_then(|texture| texture.frame(ui.ctx()))
            .cloned()
        {
            paint_cover(ui, rect, &texture);
        }

        let star_rect = Rect::from_min_size(
            egui::pos2(rect.max.x - 34.0, rect.min.y + 6.0),
            Vec2::splat(28.0),
        );
        let star = ui.interact(star_rect, Id::new(("klipy-star", slug)), Sense::click());
        if response.hovered() || star.hovered() {
            ui.painter().rect_filled(
                star_rect,
                CornerRadius::same(radius::CONTROL),
                Color32::from_black_alpha(145),
            );
            ui.painter().text(
                star_rect.center(),
                egui::Align2::CENTER_CENTER,
                icon::STAR,
                text::icon(15.0),
                if state.gif_favourites.contains(slug) {
                    t.away
                } else {
                    Color32::WHITE
                },
            );
        }
        if star.clicked() {
            toggle_favourite(state, slug);
        } else if response.clicked() {
            state.link_viewer = Some(LinkViewer {
                id: format!("klipy-viewer-{slug}"),
                url: item.gif_url.clone(),
                name: item.title.clone(),
                video: false,
                ephemeral: true,
                favourite_slug: Some(slug.to_owned()),
                zoom: 1.0,
                offset: Vec2::ZERO,
                fitted: true,
                opened: ui.input(|input| input.time),
            });
        }
    } else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            if crate::klipy::available() { s.downloading } else { s.gif_unavailable },
            text::body(),
            t.label_secondary,
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(120));
    }

    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(radius::CARD),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    state.klipy = Some(klipy);
}

pub fn toggle_favourite(state: &mut UiState, slug: &str) {
    if state.gif_favourites.remove(slug) {
        return;
    }
    state.gif_favourites.insert(slug.to_owned());
}
