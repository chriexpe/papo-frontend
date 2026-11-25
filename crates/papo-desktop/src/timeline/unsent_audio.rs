//! Your unsent message's audio (`docs/design.md` §14.3, §14.5): the player a sent one has,
//! playing its local file once staged; while it uploads, its size after its name, ✕ in its top
//! corner (Christian, 2026-11-25) and the accent bar along its foot. Once sent it is the
//! server's attachment's player, so it goes on as it was. Drawn by `message_media::audio`,
//! played by the shell (`shell/unsent_audio`).

use papo_sdk::Uuid;
use rune::El;

use super::TimelineKey;
use super::unsent_attachments::{Item, Unsent, cancel_button, progress_label, upload_line};
use super::view::TimelineHost;
use crate::audio_store::{AudioKey, AudioShown};
use crate::design::{AUDIO_CANCEL_ICON_PT, Look};
use crate::message_media::audio::{self, Player, Uploading};
use crate::message_media::received::{MediaAt, MediaSurface};

/// Unsent audio `item` of `unsent` as its player; `cancel` is the message to cancel with ✕,
/// while it can be.
pub(super) fn audio_player<S: TimelineHost>(
    unsent: &Unsent<'_>,
    item: &Item<'_>,
    cancel: Option<(TimelineKey, Uuid)>,
    look: &Look,
) -> El {
    let key = AudioKey::of_operation(unsent.key.account, unsent.operation, item.index);
    let shown = unsent
        .media
        .audio
        .get(key)
        .cloned()
        .unwrap_or_else(AudioShown::unread);
    let (size_line, share) = upload_line(item);
    let uploading = share.map(|share| Uploading {
        size_line: size_line.unwrap_or_default(),
        cancel: cancel.map(|(key, local)| {
            cancel_button::<S>(
                key,
                local,
                item.index,
                look.palette.text_secondary,
                AUDIO_CANCEL_ICON_PT,
                look,
            )
        }),
        bar: (share, progress_label(item)),
    });
    let player = Player {
        // Its row is the message's local ID until history has the server's.
        at: MediaAt {
            surface: MediaSurface::Timeline,
            account: unsent.key.account,
            channel: unsent.key.channel,
            message: unsent.local,
            index: item.index,
        },
        key,
        name: item.name,
        shown: &shown,
        interactive: unsent.media.interactive,
        hover: audio::hover_of(unsent.media, key),
    };
    audio::player::<S>(player, uploading, look)
}
