//! The server-list projection without an SDK client: registry and resources
//! in, rail entries out.

use papo_sdk::secret::CredentialGeneration;
use papo_sdk::{SavedServer, ServerAddress, UnixMicros, Uuid};

use super::*;

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
    let list = project(&registry, &HashMap::new());
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
    let (text_channels, voice_channels) = channels(&[
        channel(1, Some("general"), ChannelKind::Text),
        channel(2, Some("hangout"), ChannelKind::Voice),
    ]);
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
    let list = project(&registry, &resources);
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
    let list = project(&registry, &HashMap::new());
    assert_eq!(list.servers[0].account.as_ref().unwrap().id, first.id);
}

#[test]
fn channels_keep_the_server_s_order_and_drop_what_cannot_be_a_row() {
    let (text, voice) = channels(&[
        channel(3, Some("random"), ChannelKind::Text),
        channel(1, Some("general"), ChannelKind::Text),
        channel(9, Some("Topics"), ChannelKind::Category),
        channel(5, None, ChannelKind::Text),
        channel(2, Some("hangout"), ChannelKind::Voice),
    ]);
    assert_eq!(text, [entry(3, "random"), entry(1, "general")]);
    assert_eq!(voice, [entry(2, "hangout")]);
}
