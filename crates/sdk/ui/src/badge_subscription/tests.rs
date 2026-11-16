//! Pull-driven aggregate watch checks using synthetic sources and a dormant local client; no network.

use super::*;
use futures_util::FutureExt;
use papo_sdk::{
    Client, ClientConfig, DataRoot, Generation, ReadState, SavedAccount, SavedServer,
    ServerAddress, UnixMicros, UnreadCount, Uuid,
    secret::{CredentialGeneration, test_store::ScriptedSecretStore},
};

struct Fixture {
    _client: Client,
    _directory: tempfile::TempDir,
    account: AccountId,
    control: watch::Sender<u64>,
    resources: watch::Sender<Arc<ResourceSnapshot>>,
    reading: watch::Sender<Arc<ReadingSnapshot>>,
    subscription: BadgeSubscription,
}

fn reading(version: u64, mentions: u64) -> Arc<ReadingSnapshot> {
    Arc::new(ReadingSnapshot {
        generation: Generation::for_test(1),
        version,
        channels: BTreeMap::from([(
            Uuid::from_u128(10),
            ReadState {
                unread_count: UnreadCount::Exact(mentions),
                mention_count: UnreadCount::Exact(mentions),
                ..ReadState::default()
            },
        )]),
    })
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let client = Client::open(
            DataRoot::new(directory.path()),
            Arc::new(ScriptedSecretStore::default()),
            ClientConfig::default(),
        )
        .unwrap();
        let server = SavedServer {
            address: ServerAddress::parse("https://example.invalid").unwrap(),
            added_at: UnixMicros(0),
        };
        let account = AccountId::new(server.address.id(), Uuid::from_u128(1));
        let registry = Arc::new(RegistrySnapshot {
            version: 0,
            servers: vec![server],
            accounts: vec![SavedAccount {
                id: account,
                display_name: None,
                avatar_ref: None,
                active: true,
                created_at: UnixMicros(0),
                credential_generation: CredentialGeneration::new(1),
            }],
            removals: vec![],
        });
        let resource = Arc::new(ResourceSnapshot {
            version: 1,
            generation: Generation::for_test(1),
            server: Some(Arc::default()),
            channels: Arc::new(vec![
                serde_json::from_value(serde_json::json!({
                    "id": Uuid::from_u128(10),
                }))
                .unwrap(),
            ]),
            roles: Arc::default(),
            members: Arc::default(),
            profiles: Arc::default(),
            dms: Arc::default(),
            blocks: Arc::default(),
        });
        let initial_reading = reading(1, 0);
        let images = BTreeMap::from([(
            account,
            AccountImage {
                resources: Arc::clone(&resource),
                reading: Arc::clone(&initial_reading),
            },
        )]);
        let (control, control_rx) = watch::channel(0);
        let (resources, resource_rx) = watch::channel(resource);
        let (reading, reading_rx) = watch::channel(initial_reading);
        let mut controls = SelectAll::<Source>::new();
        controls.push(watch_source(control_rx, |_| Event::Sessions));
        let mut accounts = SelectAll::<Source>::new();
        accounts.push(account_source(resource_rx, move |value| {
            Event::Resources(account, value)
        }));
        accounts.push(account_source(reading_rx, move |value| {
            Event::Reading(account, value)
        }));
        let subscription = from_sources(Sources {
            handle: client.handle(),
            registry,
            images,
            controls,
            accounts,
        });
        Self {
            _client: client,
            _directory: directory,
            account,
            control,
            resources,
            reading,
            subscription,
        }
    }
}

#[test]
fn unchanged_source_publications_do_not_emit_or_advance_badge_version() {
    let mut fixture = Fixture::new();
    assert_eq!(fixture.subscription.version, 0);
    fixture
        .resources
        .send_modify(|image| Arc::make_mut(image).version += 1);
    fixture.reading.send_replace(reading(2, 0));
    assert!(fixture.subscription.stream.next().now_or_never().is_none());

    fixture.reading.send_replace(reading(3, 2));
    let update = fixture
        .subscription
        .stream
        .next()
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(update.version, 1);
    assert_eq!(
        update.snapshot.accounts[&fixture.account].unwrap().mentions,
        UnreadCount::Exact(2)
    );
    assert!(fixture.subscription.stream.next().now_or_never().is_none());
}

#[test]
fn source_update_before_first_poll_is_not_lost_and_coalesces_without_false_precision() {
    let mut fixture = Fixture::new();
    fixture.reading.send_replace(reading(2, 1));
    let mut latest = reading(99, 3);
    Arc::make_mut(&mut latest)
        .channels
        .get_mut(&Uuid::from_u128(10))
        .unwrap()
        .mention_count = UnreadCount::AtLeast(3);
    fixture.reading.send_replace(latest);
    let update = fixture
        .subscription
        .stream
        .next()
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(update.version, 1);
    assert_eq!(
        update.snapshot.accounts[&fixture.account].unwrap().mentions,
        UnreadCount::AtLeast(3)
    );
    assert!(fixture.subscription.stream.next().now_or_never().is_none());
}

#[test]
fn session_lifecycle_event_invalidates_image_when_session_is_not_open() {
    let mut fixture = Fixture::new();
    assert!(fixture.subscription.snapshot.accounts[&fixture.account].is_some());
    fixture.control.send_replace(1);
    let update = fixture
        .subscription
        .stream
        .next()
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(update.version, 1);
    assert_eq!(update.snapshot.accounts[&fixture.account], None);
    assert_eq!(update.snapshot.direct_messages, None);
    assert!(fixture.subscription.stream.next().now_or_never().is_none());
}

#[test]
fn account_watch_close_invalidates_totals_instead_of_preserving_stale_zero() {
    let mut fixture = Fixture::new();
    drop(fixture.reading);
    let update = fixture
        .subscription
        .stream
        .next()
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(update.snapshot.accounts[&fixture.account], None);
    assert_eq!(update.snapshot.direct_messages, None);
    assert!(fixture.subscription.stream.next().now_or_never().is_none());
}

#[test]
fn control_watch_close_ends_subscription() {
    let mut fixture = Fixture::new();
    drop(fixture.control);
    assert!(
        fixture
            .subscription
            .stream
            .next()
            .now_or_never()
            .unwrap()
            .is_none()
    );
}

#[test]
fn account_source_drains_final_unseen_value_before_closure() {
    let (sender, receiver) = watch::channel(Arc::new(1_u64));
    let account = AccountId::new(
        ServerAddress::parse("https://example.invalid")
            .unwrap()
            .id(),
        Uuid::from_u128(1),
    );
    let mut source = account_source(receiver, move |value| {
        let version = value.map(|value| *value);
        Event::Reading(account, version.map(|version| reading(version, 0)))
    });
    sender.send_replace(Arc::new(2));
    drop(sender);
    match source.next().now_or_never().unwrap().unwrap() {
        Event::Reading(id, Some(value)) => {
            assert_eq!(id, account);
            assert_eq!(value.version, 2);
        }
        _ => panic!("expected final account publication"),
    }
    assert!(matches!(
        source.next().now_or_never().unwrap(),
        Some(Event::Reading(_, None))
    ));
    assert!(source.next().now_or_never().unwrap().is_none());
}
