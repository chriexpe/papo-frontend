//! People's profile photos and banners (`docs/design.md` §8, §10): read
//! through the SDK for the Profile page and the profile cards. The photo
//! comes inline with the profile (`inline_avatar`), the banner through the
//! media cache.
//!
//! Each person is read once, again after a while (so another person's new
//! picture shows), at once after your own change, and on the next ask after
//! a read failed. A read that lands after a newer one is dropped. Changing
//! your own is `own_pictures`'; the rows are `settings::picture_row`'s, the
//! cards `profile_card`'s.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use papo_sdk::{AccountId, MediaSource, Session, Uuid, inline_avatar};
use rune::{Image, ImageCache};

use super::Shell;
use super::staged_files::decoded;
use crate::sdk_bridge::Updates;

/// The side a photo is decoded to fit, in points: the card's avatar and
/// the row's, at twice.
const AVATAR_DECODE_PT: f32 = 160.0;
/// The width a banner is decoded to fit, in points: the card's width.
const BANNER_DECODE_PT: f32 = 600.0;
/// How long a read stands before the next ask reads again.
const REFRESH_AFTER: Duration = Duration::from_mins(10);

/// One person's pictures, as read.
#[derive(Clone, Default)]
pub(crate) struct Pictures {
    pub(crate) avatar: Option<Image>,
    pub(crate) banner: Option<Image>,
}

/// Whose pictures: a person, as one account sees them.
type Person = (AccountId, Uuid);

/// One person's pictures and when they were read.
struct Read {
    pictures: Pictures,
    at: Instant,
}

/// The pictures read this run, by account and person.
#[derive(Default)]
pub(crate) struct ProfilePictures {
    read: HashMap<Person, Read>,
    /// People with a read running.
    running: HashSet<Person>,
    /// The last request handed out; requests count up.
    last_request: u64,
    /// The request each person's shown pictures came from.
    shown_from: HashMap<Person, u64>,
}

impl ProfilePictures {
    /// `user`'s pictures as seen from `account`, once read.
    pub(crate) fn get(&self, account: AccountId, user: Uuid) -> Option<&Pictures> {
        self.read.get(&(account, user)).map(|read| &read.pictures)
    }

    /// `user`'s pictures as `account` sees them, read: what the headless
    /// tests draw without an SDK.
    #[cfg(test)]
    pub(crate) fn read_for_test(&mut self, account: AccountId, user: Uuid, pictures: Pictures) {
        let request = self.request((account, user));
        self.landed((account, user), request, Some(pictures), Instant::now());
    }

    /// The request for a read of `person` asked at `now`, unless one runs or
    /// the last is recent.
    fn ask(&mut self, person: Person, now: Instant) -> Option<u64> {
        let recent = self
            .read
            .get(&person)
            .is_some_and(|read| now.saturating_duration_since(read.at) < REFRESH_AFTER);
        if recent || self.running.contains(&person) {
            return None;
        }
        Some(self.request(person))
    }

    /// The request for a read of `person` that goes whatever stands: after
    /// your own change.
    fn request(&mut self, person: Person) -> u64 {
        self.running.insert(person);
        self.last_request += 1;
        self.last_request
    }

    /// Read `request` of `person` landed at `now`: its pictures, or none when
    /// the read failed (the next ask reads again). Returns whether what
    /// shows changed; a read older than the shown one changes nothing.
    fn landed(
        &mut self,
        person: Person,
        request: u64,
        pictures: Option<Pictures>,
        now: Instant,
    ) -> bool {
        self.running.remove(&person);
        let newer = self
            .shown_from
            .get(&person)
            .is_none_or(|shown| request > *shown);
        let Some(pictures) = pictures.filter(|_| newer) else {
            return false;
        };
        self.shown_from.insert(person, request);
        self.read.insert(person, Read { pictures, at: now });
        true
    }
}

/// A read of one person's pictures, ready to run off the UI thread.
pub(super) struct PictureRead {
    person: Person,
    request: u64,
    updates: Updates<Shell>,
    images: Arc<ImageCache>,
    scale: f32,
}

impl PictureRead {
    /// Reads the pictures and shows them.
    pub(super) async fn run(self, session: &Session) {
        let PictureRead {
            person,
            request,
            updates,
            images,
            scale,
        } = self;
        let pictures = read_pictures(session, person.1, &images, scale).await;
        updates.send(move |shell| {
            shell
                .profile_pictures
                .landed(person, request, pictures, Instant::now())
        });
    }
}

impl Shell {
    /// `user`'s pictures are drawn from `account`: read unless a recent read
    /// stands or one runs.
    pub(crate) fn want_pictures(&mut self, account: AccountId, user: Uuid) {
        let Some(request) = self.profile_pictures.ask((account, user), Instant::now()) else {
            return;
        };
        let (Some(sdk), Some(read)) = (
            self.sdk.clone(),
            self.picture_read((account, user), request),
        ) else {
            return;
        };
        sdk.run(move |client| async move {
            if let Ok(session) = client.session(account) {
                read.run(&session).await;
            }
        });
    }

    /// A read of your own pictures that goes whatever stands, for after a
    /// change of yours lands.
    pub(super) fn own_picture_read(&mut self, account: AccountId) -> Option<PictureRead> {
        let person = (account, account.user());
        let request = self.profile_pictures.request(person);
        self.picture_read(person, request)
    }

    fn picture_read(&self, person: Person, request: u64) -> Option<PictureRead> {
        Some(PictureRead {
            person,
            request,
            updates: self.sdk.as_ref()?.updates(),
            images: Arc::clone(self.images.as_ref()?),
            scale: self.scale,
        })
    }
}

/// `user`'s photo and banner, read through the SDK: the photo inline in
/// the profile, the banner from the media. None when the read failed; a
/// picture the profile doesn't have is none inside.
async fn read_pictures(
    session: &Session,
    user: Uuid,
    images: &Arc<ImageCache>,
    scale: f32,
) -> Option<Pictures> {
    let profiles = match session.profiles(&[user]).await {
        Ok(profiles) => profiles.value,
        Err(error) => {
            log::debug!("a profile was not read for its pictures: {error}");
            return None;
        }
    };
    let profile = profiles.into_iter().find(|profile| {
        profile
            .id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok())
            == Some(user)
    })?;
    let avatar = match inline_avatar(&profile) {
        Ok(Some(bytes)) => Some(decoded(images.load_scaled(bytes, AVATAR_DECODE_PT, scale)).await),
        Ok(None) => None,
        Err(error) => {
            log::debug!("a profile's photo was not usable: {error}");
            None
        }
    };
    let banner = match profile.banner_media.filter(|media| !media.is_empty()) {
        Some(media) => match session.media().fetch(MediaSource::Blob(media), false).await {
            Ok(media) => {
                let banner = images.load_scaled(media.value.bytes, BANNER_DECODE_PT, scale);
                Some(decoded(banner).await)
            }
            Err(error) => {
                log::debug!("a profile's banner was not fetched: {error}");
                return None;
            }
        },
        None => None,
    };
    Some(Pictures { avatar, banner })
}

#[cfg(test)]
mod tests {
    //! When a person is read again, and which read shows.
    use super::*;
    use papo_sdk::ServerAddress;

    fn person() -> Person {
        let server = ServerAddress::parse("https://lumen.example.invalid")
            .expect("an address")
            .id();
        (
            AccountId::new(server, Uuid::from_u128(1)),
            Uuid::from_u128(7),
        )
    }

    #[test]
    fn a_person_is_read_once_then_again_after_a_while() {
        let mut store = ProfilePictures::default();
        let start = Instant::now();
        let first = store.ask(person(), start).expect("a first read");
        assert_eq!(store.ask(person(), start), None, "one at a time");
        assert!(store.landed(person(), first, Some(Pictures::default()), start));
        assert_eq!(store.ask(person(), start + Duration::from_secs(60)), None);
        assert!(store.ask(person(), start + REFRESH_AFTER).is_some());
    }

    #[test]
    fn a_failed_read_is_tried_again_at_the_next_ask() {
        let mut store = ProfilePictures::default();
        let start = Instant::now();
        let first = store.ask(person(), start).expect("a first read");
        assert!(!store.landed(person(), first, None, start));
        assert!(store.ask(person(), start).is_some());
    }

    #[test]
    fn a_read_landing_after_a_newer_one_is_dropped() {
        let mut store = ProfilePictures::default();
        let start = Instant::now();
        let old = store.ask(person(), start).expect("a first read");
        let after_change = store.request(person());
        assert!(store.landed(person(), after_change, Some(Pictures::default()), start));
        assert!(!store.landed(person(), old, Some(Pictures::default()), start));
    }
}
