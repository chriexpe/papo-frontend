//! Watch snapshot publication for saved reading positions.
//! Flush batching and revocation coordination live in `reading::positions`.

use std::sync::Arc;

use papo_api::Uuid;

use crate::{Generation, catch_up::ReadingPosition, session::SessionInner};

pub(crate) struct PositionUpdate {
    pub channel: Uuid,
    pub position: Option<ReadingPosition>,
    pub expected_epoch: Option<u64>,
}

pub(crate) fn publish_positions(
    session: &SessionInner,
    generation: Generation,
    updates: Vec<PositionUpdate>,
) {
    let _ = session.epochs.apply(generation, || {
        session.reading.position_image.send_if_modified(|current| {
            let mut next = (**current).clone();
            let mut changed = next.generation != generation;
            next.generation = generation;
            for update in updates {
                let effective = if update.expected_epoch.is_some_and(|epoch| {
                    session
                        .reading
                        .positions
                        .check_writable(update.channel, generation, epoch)
                        .is_some()
                }) {
                    None
                } else {
                    update.position
                };
                let prev = next.channels.get(&update.channel).copied();
                if prev != effective {
                    changed = true;
                    match effective {
                        Some(pos) => {
                            next.channels.insert(update.channel, pos);
                        }
                        None => {
                            next.channels.remove(&update.channel);
                        }
                    }
                }
            }
            if !changed {
                return false;
            }
            let Some(version) = next.version.checked_add(1) else {
                return false;
            };
            next.version = version;
            *current = Arc::new(next);
            true
        });
    });
}

pub(crate) fn remove_from_image(session: &SessionInner, channels: &[Uuid], generation: Generation) {
    let updates: Vec<_> = channels
        .iter()
        .map(|&channel| PositionUpdate {
            channel,
            position: None,
            expected_epoch: None,
        })
        .collect();
    if !updates.is_empty() {
        publish_positions(session, generation, updates);
    }
}
