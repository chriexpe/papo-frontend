//! A received file as a card (`docs/design.md` §14.6) and its save: Save (`download`) at its
//! end; while it saves, "4.1 of 17.8 MB", the accent bar along its bottom and ✕; once saved,
//! "17.8 MB · Saved" with Open (`external`) and Show in folder (`folder`); a failed save is one
//! `danger` line, "Couldn't save the file. Try again". A press on the card saves as Save does.
//!
//! Pure: the save itself is `saving`'s, run by the host.

use papo_sdk::Attachment;
use rune::{El, Icon, Key, Tx, el, icon, row, text};

use super::received::{
    MediaAt, MediaHost, MediaPress, MediaStyle, attachment_id, media_element_key,
};
use super::{FileCard, file_card, size_line};
use crate::design::{
    FILE_CARD_BUTTON_PT, FOCUS_RING_PT, Look, RADIUS_CONTROL, ROW_ICON_PT, TYPE_CAPTION,
    TYPE_CAPTION_MEDIUM,
};
use crate::icons::{DOWNLOAD, EXTERNAL, FOLDER, X};
use crate::media_store::MediaKey;
use crate::saving::SaveView;
use crate::strings;

/// Attachment `at` (a file, or a video or audio until they play) as a card.
pub(crate) fn file_card_of<S: MediaHost>(
    at: MediaAt,
    attachment: &Attachment,
    style: &MediaStyle,
    look: &Look,
) -> El {
    let name = attachment.original_file_name.as_deref().unwrap_or_default();
    let total = attachment
        .size_bytes
        .and_then(|size| u64::try_from(size).ok());
    let save = attachment_id(attachment)
        .map(|attachment| MediaKey::attachment(at.account, attachment))
        .and_then(|key| style.saves.get(&key));
    let (line, ends, bar) = save_parts::<S>(at, save, total, look);
    let label = match &total {
        Some(total) => strings::media_file(name, &strings::size(*total)),
        None => name.to_owned(),
    };
    let idle = matches!(save, None | Some(SaveView::Failed));
    let card = FileCard {
        name,
        size_line: line,
        label,
    };
    let mut built = file_card(card, ends, bar, look)
        .key(media_element_key(at))
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_secondary_press(move |host: &mut S, press, tx| {
            host.media_press(at, MediaPress::Menu(Some([press.x, press.y])), tx);
        });
    if idle {
        // A press anywhere on the card saves, as Save does; Enter too.
        built = built
            .on_press_tx(move |host: &mut S, tx| host.media_press(at, MediaPress::Save, tx))
            .on_key(move |host: &mut S, event, tx| {
                let plain = !(event.mods.ctrl || event.mods.alt || event.mods.cmd);
                if plain && event.key == Key::Enter {
                    host.media_press(at, MediaPress::Save, tx);
                    return true;
                }
                false
            });
    }
    built
}

/// The card's size line, its end buttons and its bar, as the save stands.
fn save_parts<S: MediaHost>(
    at: MediaAt,
    save: Option<&SaveView>,
    total: Option<u64>,
    look: &Look,
) -> (Option<El>, Vec<El>, Option<(f32, String)>) {
    let size = total.map(strings::size);
    match save {
        None => (
            size.map(|size| size_line(size, look)),
            vec![end_button::<S>(
                at,
                &DOWNLOAD,
                strings::file_save(),
                MediaPress::Save,
                look,
            )],
            None,
        ),
        Some(SaveView::Saving {
            saved_bytes,
            total_bytes,
        }) => {
            let total_bytes = total_bytes.or(total).unwrap_or(*saved_bytes);
            #[expect(clippy::cast_precision_loss, reason = "a share shown on a bar")]
            let share = if total_bytes == 0 {
                0.0
            } else {
                *saved_bytes as f32 / total_bytes as f32
            };
            let progress = strings::file_saving(*saved_bytes, total_bytes);
            (
                Some(size_line(progress.clone(), look)),
                vec![end_button::<S>(
                    at,
                    &X,
                    strings::file_cancel(),
                    MediaPress::CancelSave,
                    look,
                )],
                Some((share, progress)),
            )
        }
        Some(SaveView::Saved(_)) => (
            size.map(|size| size_line(strings::file_saved(&size), look)),
            vec![
                end_button::<S>(
                    at,
                    &EXTERNAL,
                    strings::file_open(),
                    MediaPress::OpenSaved,
                    look,
                ),
                end_button::<S>(
                    at,
                    &FOLDER,
                    strings::file_show(),
                    MediaPress::ShowSaved,
                    look,
                ),
            ],
            None,
        ),
        Some(SaveView::Failed) => (
            Some(failed_line::<S>(at, look)),
            vec![end_button::<S>(
                at,
                &DOWNLOAD,
                strings::file_save(),
                MediaPress::Save,
                look,
            )],
            None,
        ),
    }
}

/// A button at the card's end: `glyph` in `text.secondary`, named `label`.
fn end_button<S: MediaHost>(
    at: MediaAt,
    glyph: &'static Icon,
    label: &'static str,
    press: MediaPress,
    look: &Look,
) -> El {
    // Named by what it does, not by its words, which change with the language.
    el().key(format!("{}-{press:?}", media_element_key(at)))
        .size(FILE_CARD_BUTTON_PT, FILE_CARD_BUTTON_PT)
        .center()
        .radius(look.radius(RADIUS_CONTROL))
        .hover_fill(look.palette.surface_hover)
        .role("button")
        .label(label)
        .help(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(move |host: &mut S, tx: &mut Tx| host.media_press(at, press, tx))
        .child(icon(glyph, ROW_ICON_PT).color(look.palette.text_secondary))
}

/// "Couldn't save the file. Try again", in `danger`, Try again pressable.
fn failed_line<S: MediaHost>(at: MediaAt, look: &Look) -> El {
    let (said, again) = strings::file_failed();
    row()
        .gap(look.space(crate::design::SPACE_XS))
        .child(
            text(said)
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.danger),
        )
        .child(
            text(again)
                .font(look.font(TYPE_CAPTION_MEDIUM))
                .color(look.palette.danger)
                .role("button")
                .focusable(true)
                .on_press_tx(move |host: &mut S, tx| host.media_press(at, MediaPress::Save, tx)),
        )
}
