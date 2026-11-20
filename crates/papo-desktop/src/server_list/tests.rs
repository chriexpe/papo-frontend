//! The server-list projection without an SDK client: registry and resources
//! in, rail entries out.

use papo_sdk::secret::CredentialGeneration;
use papo_sdk::{ChannelBadges, SavedServer, ServerAddress, UnixMicros, UnreadCount, Uuid};

use super::*;
use crate::badges::Badge;

fn server(address: &str) -> SavedServer {
    SavedServer {
        address: ServerAddress::parse(address).unwrap(),
        added_at: UnixMicros(0),
    }
}

fn account(server: &SavedServer, user: u128, active: bool) -> SavedAccount {
    SavedAccount {
        id: AccountId::new(server.address.id(), Uuid::from_u128(user)),
        username: format!("user{user}"),
        display_name: (user == 2).then(|| "Ana".to_owned()),
        avatar_ref: None,
        active,
        created_at: UnixMicros(0),
        credential_generation: CredentialGeneration::new(1),
    }
}

fn channel(id: u128, name: Option<&str>, kind: ChannelKind) -> ChannelRecord {
    ChannelRecord {
        id: Uuid::from_u128(id),
        name: name.map(str::to_owned),
        kind,
        position: None,
        topic: None,
        parent_id: None,
        permissions: Vec::new(),
        created_at: None,
        last_message: None,
        last_read_message: None,
        last_read_at: None,
        notification_settings: None,
    }
}

fn entry(id: u128, name: &str) -> ChannelEntry {
    ChannelEntry {
        id: Uuid::from_u128(id).to_string(),
        name: name.to_owned(),
        badge: Badge::Nothing,
    }
}

#[test]
fn servers_keep_their_saved_order_and_fall_back_to_their_address() {
    let (lumen, mirante) = (
        server("https://lumen.example.invalid"),
        server("http://mirante.local"),
    );
    let registry = RegistrySnapshot {
        servers: vec![mirante.clone(), lumen.clone()],
        ..RegistrySnapshot::default()
    };
    let list = project(&registry, &HashMap::new(), &BTreeMap::new());
    let names: Vec<_> = list.servers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["http://mirante.local", "lumen.example.invalid"]);
    assert_eq!(list.order(), [mirante.address.id(), lumen.address.id()]);
    assert!(
        list.servers
            .iter()
            .all(|s| s.account.is_none() && s.icon.is_none())
    );
}

#[test]
fn a_signed_in_server_takes_its_name_owner_icon_and_channels_from_its_resources() {
    let lumen = server("https://lumen.example.invalid");
    let ana = account(&lumen, 2, true);
    let registry = RegistrySnapshot {
        servers: vec![lumen.clone()],
        accounts: vec![account(&lumen, 1, false), ana.clone()],
        ..RegistrySnapshot::default()
    };
    let (text_channels, voice_channels) = channels(
        &[
            channel(1, Some("general"), ChannelKind::Text),
            channel(2, Some("hangout"), ChannelKind::Voice),
        ],
        None,
    );
    let resources = HashMap::from([(
        ana.id,
        ServerResources {
            name: Some("Lumen".to_owned()),
            owner: Some("nadia".to_owned()),
            icon: Some(Arc::from(&b"png"[..])),
            text_channels,
            voice_channels,
        },
    )]);
    let list = project(&registry, &resources, &BTreeMap::new());
    let shown = &list.servers[0];
    assert_eq!(shown.name, "Lumen");
    assert_eq!(shown.owner.as_deref(), Some("nadia"));
    assert_eq!(shown.icon.as_deref(), Some(&b"png"[..]));
    let account = shown.account.as_ref().unwrap();
    assert_eq!(account.id, ana.id, "the active account is shown");
    assert_eq!(
        (account.name.as_str(), account.username.as_str()),
        ("Ana", "user2")
    );
    assert_eq!(shown.text_channels, [entry(1, "general")]);
    assert_eq!(shown.voice_channels, [entry(2, "hangout")]);
}

#[test]
fn without_an_active_account_the_first_saved_one_is_shown() {
    let lumen = server("https://lumen.example.invalid");
    let first = account(&lumen, 1, false);
    let registry = RegistrySnapshot {
        servers: vec![lumen.clone()],
        accounts: vec![first.clone(), account(&lumen, 3, false)],
        ..RegistrySnapshot::default()
    };
    let list = project(&registry, &HashMap::new(), &BTreeMap::new());
    assert_eq!(list.servers[0].account.as_ref().unwrap().id, first.id);
}

#[test]
fn channels_keep_the_server_s_order_and_drop_what_cannot_be_a_row() {
    let (text, voice) = channels(
        &[
            channel(3, Some("random"), ChannelKind::Text),
            channel(1, Some("general"), ChannelKind::Text),
            channel(9, Some("Topics"), ChannelKind::Category),
            channel(5, None, ChannelKind::Text),
            channel(2, Some("hangout"), ChannelKind::Voice),
        ],
        None,
    );
    assert_eq!(text, [entry(3, "random"), entry(1, "general")]);
    assert_eq!(voice, [entry(2, "hangout")]);
}

fn uuid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn channel_badges(has_unread: Option<bool>, mentions: UnreadCount) -> Arc<ChannelBadges> {
    Arc::new(ChannelBadges {
        has_unread,
        mention_count: mentions,
        direct_message: false,
    })
}

#[test]
fn each_channel_takes_its_badge_by_identity_and_unknown_ones_show_nothing() {
    let snapshot = BadgesSnapshot {
        generation: papo_sdk::Generation::for_test(1),
        version: 1,
        channels_complete: false,
        channels: BTreeMap::from([
            (uuid(1), channel_badges(Some(true), UnreadCount::Exact(0))),
            (uuid(2), channel_badges(None, UnreadCount::AtLeast(3))),
            (uuid(3), channel_badges(None, UnreadCount::AtLeast(0))),
        ]),
    };
    let (text, voice) = channels(
        &[
            channel(1, Some("general"), ChannelKind::Text),
            channel(2, Some("media"), ChannelKind::Text),
            channel(3, Some("random"), ChannelKind::Text),
            channel(4, Some("new"), ChannelKind::Text),
            channel(5, Some("hangout"), ChannelKind::Voice),
        ],
        Some(&snapshot),
    );
    let badges: Vec<_> = text.iter().map(|entry| entry.badge).collect();
    assert_eq!(badges[0], Badge::Unread);
    assert_eq!(badges[1].mentions().unwrap().shown(), "3+");
    assert_eq!(badges[2], Badge::Nothing, "unknown unread shows nothing");
    assert_eq!(badges[3], Badge::Nothing, "no badge image for it yet");
    assert_eq!(voice[0].badge, Badge::Nothing);
}

#[test]
fn a_server_takes_the_aggregate_of_the_account_its_rows_show() {
    let lumen = server("https://lumen.example.invalid");
    // Neither account is active: the rows and the tile both follow the first.
    let (first, second) = (account(&lumen, 1, false), account(&lumen, 3, false));
    let registry = RegistrySnapshot {
        servers: vec![lumen.clone(), server("http://mirante.local")],
        accounts: vec![first.clone(), second.clone()],
        ..RegistrySnapshot::default()
    };
    let aggregate = |mentions| {
        Arc::new(AccountBadges {
            all: papo_sdk_ui::BadgeAggregate {
                mention_count: UnreadCount::Exact(mentions),
                has_unread: Some(true),
            },
            dms: papo_sdk_ui::BadgeAggregate::default(),
        })
    };
    let accounts = BTreeMap::from([(first.id, aggregate(120)), (second.id, aggregate(2))]);
    let list = project(&registry, &HashMap::new(), &accounts);
    assert_eq!(list.servers[0].badge.mentions().unwrap().shown(), "99+");
    assert_eq!(
        list.servers[1].badge,
        Badge::Nothing,
        "no aggregate: nothing claimed"
    );
}
