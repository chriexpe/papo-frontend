//! Mention counts against the actual pinned Go backend on disposable PostgreSQL. The SDK's
//! classification of cached history must agree with the backend's own notification dispatch;
//! run only through the proof harness.
//!
//! Rows that arrive live in a history no REST page has proven keep every count a lower bound,
//! because nothing proves the rows before them complete; those are checked by value. A history
//! loaded from a REST page proves its coverage, so its counts must be exact, and live rows that
//! extend it keep them exact.
#![cfg(feature = "test-support")]
use papo_api::{
    RestClient,
    wire::{LoginRequest, OpenDirectMessageRequest, RegisterRequest},
};
use papo_sdk::{
    Client, ClientConfig, DataRoot, HistoryQuery, NotificationSettings, Session, UnixMicros,
    UnreadCount, Uuid, VisibilityContinuity, secret::test_store::ScriptedSecretStore,
};
#[path = "session13_proof/badge_support.rs"]
mod support;
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use support::{mentions_reach, online, rest_as, send, settled, text_channel};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Keeps history handles and subscriptions open: dropping them may release the snapshots.
type Held = Vec<Box<dyn std::any::Any>>;

/// REST identities and the SDK account under test.
struct World {
    session: Session,
    me: Uuid,
    owner: RestClient,
    me_rest: RestClient,
    peer: RestClient,
    live_channel: Uuid,
    paged_channel: Uuid,
}

/// The newest page, which a REST response proves complete when it holds fewer rows than asked.
fn newest_page() -> HistoryQuery<'static> {
    HistoryQuery {
        limit: Some(100),
        ..HistoryQuery::default()
    }
}

/// One message of the scenario and whether the backend notifies the SDK account about it.
struct Step<'a> {
    name: &'a str,
    author: &'a RestClient,
    content: String,
    replies_to_account: bool,
    triggers: bool,
}

#[test]
#[ignore = "requires isolated pinned backend and PostgreSQL; run scripts/papo-backend-proof.py"]
fn sdk_mention_counts_match_the_actual_backend_notification_dispatch() -> Result {
    let address = std::env::var("PAPO_BACKEND_PROOF_ADDRESS")?;
    let password = std::env::var("PAPO_PROOF_PASSWORD")?;
    let directory = tempfile::tempdir()?;
    let mut client = Client::open(
        DataRoot::new(directory.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig {
            insecure_loopback: true,
            ..ClientConfig::default()
        },
    )?;
    client.block_on(async {
        tokio::time::timeout(Duration::from_secs(40), async {
            let world = prepare(&client, &address, &password).await?;
            let mut held = Held::new();
            let mut counted = prove_live_rows(&world, &mut held).await?;
            counted.extend(prove_paged_rows(&world, &mut held).await?);
            counted.extend(prove_direct_conversation(&world, &mut held).await?);
            prove_backend_agrees(&world, counted).await?;
            prove_mute_and_read(&world).await
        })
        .await?
    })??;
    let report = client.shutdown()?;
    assert!(report.state.finished && report.shared.finished);
    Ok(())
}

async fn prepare(client: &Client, address: &str, password: &str) -> Result<World> {
    let owner = rest_as(address, "cursorproof", password, false).await?;
    let peer = rest_as(address, "badgepeer", password, true).await?;
    let live_channel = text_channel(&owner, "badge-live-proof").await?;
    let paged_channel = text_channel(&owner, "badge-paged-proof").await?;
    let handle = client.handle();
    let server = handle.add_server(address, UnixMicros(0)).await?;
    let auth = handle.auth(server)?;
    auth.register(RegisterRequest {
        username: "badgeproof".into(),
        password: password.to_owned(),
    })
    .await?;
    let session = auth
        .login(
            LoginRequest {
                username: "badgeproof".into(),
                password: password.to_owned(),
            },
            UnixMicros(0),
        )
        .await?;
    online(&session).await?;
    let me_rest = rest_as(address, "badgeproof", password, false).await?;
    Ok(World {
        me: session.account().user(),
        session,
        owner,
        me_rest,
        peer,
        live_channel,
        paged_channel,
    })
}

fn scenario(world: &World) -> Vec<Step<'_>> {
    let mention = papo_sdk::mentions::encode_user_mention(world.me);
    let step = |name, author, content: String, replies_to_account, triggers| Step {
        name,
        author,
        content,
        replies_to_account,
        triggers,
    };
    vec![
        step(
            "direct mention, duplicated and inside code",
            &world.owner,
            format!("hi {mention} and `{mention}`"),
            false,
            true,
        ),
        step(
            "own message",
            &world.me_rest,
            "parent written by the SDK account".into(),
            false,
            false,
        ),
        step(
            "reply to the account",
            &world.owner,
            "re: your message".into(),
            true,
            true,
        ),
        step(
            "@everyone from the owner",
            &world.owner,
            "@everyone heads up".into(),
            false,
            true,
        ),
        step(
            "@everyone without the permission",
            &world.peer,
            "@everyone from a plain member".into(),
            false,
            false,
        ),
        step(
            "unsupported role token",
            &world.owner,
            "@role(admins) hello".into(),
            false,
            false,
        ),
        step(
            "plain text",
            &world.owner,
            "nothing to see".into(),
            false,
            false,
        ),
        step(
            "own @everyone",
            &world.me_rest,
            "@everyone from myself".into(),
            false,
            false,
        ),
    ]
}

/// Sends the scenario and returns the messages the backend notifies the SDK account about.
/// `observe` runs after each send with the step name, the rows so far and the triggers so far.
async fn play(
    world: &World,
    channel: Uuid,
    observe: impl AsyncFn(&str, u64, u64) -> Result,
) -> Result<BTreeSet<Uuid>> {
    let (mut counted, mut parent, mut rows) = (BTreeSet::new(), None::<String>, 0);
    for step in scenario(world) {
        let reply_to = step.replies_to_account.then(|| parent.clone()).flatten();
        let id = send(step.author, channel, &step.content, reply_to.as_deref()).await?;
        if step.name == "own message" {
            parent = Some(id.to_string());
        }
        rows += 1;
        if step.triggers {
            counted.insert(id);
        }
        observe(step.name, rows, counted.len() as u64).await?;
    }
    Ok(counted)
}

/// Rows arriving in a history no page has proven are live: the count is right but only a lower
/// bound.
async fn prove_live_rows(world: &World, held: &mut Held) -> Result<BTreeSet<Uuid>> {
    let history = world.session.history(world.live_channel).await?;
    held.push(Box::new(history.subscribe().await?));
    held.push(Box::new(history));
    play(world, world.live_channel, async |name, rows, triggers| {
        let state = settled(&world.session, world.live_channel, rows, name).await?;
        let counted = match state.mention_count {
            UnreadCount::AtLeast(counted) => counted,
            other => panic!("a live row must leave a lower bound, got {other:?} after: {name}"),
        };
        assert_eq!(counted, triggers, "live row after: {name}");
        Ok(())
    })
    .await
}

/// A history opened after the rows exist is proven by a REST page, so its count is exact.
async fn prove_paged_rows(world: &World, held: &mut Held) -> Result<BTreeSet<Uuid>> {
    let counted = play(world, world.paged_channel, async |_, _, _| Ok(())).await?;
    let history = world.session.history(world.paged_channel).await?;
    held.push(Box::new(history.subscribe().await?));
    history.page(newest_page()).await?;
    held.push(Box::new(history));
    mentions_reach(
        &world.session,
        world.paged_channel,
        UnreadCount::Exact(counted.len() as u64),
        "paged history to count exactly",
    )
    .await?;
    Ok(counted)
}

/// In a direct conversation every message from the peer is a trigger.
async fn prove_direct_conversation(world: &World, held: &mut Held) -> Result<BTreeSet<Uuid>> {
    let dm: Uuid = world
        .peer
        .open_direct_conversation(&OpenDirectMessageRequest {
            user_id: world.me.to_string(),
        })
        .await?
        .id
        .parse()?;
    let mut counted = BTreeSet::new();
    for content in ["first from the peer", "second from the peer"] {
        counted.insert(send(&world.peer, dm, content, None).await?);
    }
    let mut resources = world.session.subscribe_resources();
    while !resources
        .borrow_and_update()
        .dms
        .iter()
        .any(|known| known.id == dm.to_string())
    {
        resources.changed().await?;
    }
    let history = world.session.history(dm).await?;
    held.push(Box::new(history.subscribe().await?));
    history.page(newest_page()).await?;
    held.push(Box::new(history));
    mentions_reach(
        &world.session,
        dm,
        UnreadCount::Exact(2),
        "direct conversation count",
    )
    .await?;
    Ok(counted)
}

/// The backend's inbox is its own decision on who was notified: the SDK's classification of
/// cached history must select exactly the same messages.
async fn prove_backend_agrees(world: &World, expected: BTreeSet<Uuid>) -> Result {
    let mut previous = None;
    for _ in 0..100 {
        let page = world.session.notifications().inbox(None).await?;
        let held: BTreeSet<Uuid> = page
            .records
            .iter()
            .filter_map(|row| row.record.message_id.as_deref()?.parse().ok())
            .collect();
        if held == expected && previous.as_ref() == Some(&held) {
            return Ok(());
        }
        previous = Some(held);
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    Err(format!("inbox {previous:?} differs from the SDK's counted messages {expected:?}").into())
}

async fn prove_mute_and_read(world: &World) -> Result {
    let channel = world.paged_channel;
    let mut settings = NotificationSettings::default();
    settings.muted.insert(channel);
    world.session.notifications().set_settings(settings).await?;
    mentions_reach(
        &world.session,
        channel,
        UnreadCount::Exact(0),
        "muted channel to read zero",
    )
    .await?;
    world
        .session
        .notifications()
        .set_settings(NotificationSettings::default())
        .await?;
    mentions_reach(
        &world.session,
        channel,
        UnreadCount::Exact(3),
        "unmuted channel to recount",
    )
    .await?;

    let snapshot = world
        .session
        .history(channel)
        .await?
        .subscribe()
        .await?
        .snapshot;
    let newest: Uuid = snapshot
        .messages
        .last()
        .and_then(|message| message.id.as_deref()?.parse().ok())
        .ok_or("newest row")?;
    world
        .session
        .reading_observer()
        .report_visible(
            channel,
            snapshot.generation,
            snapshot.version,
            &[newest],
            VisibilityContinuity::Discontinuous,
        )
        .await?;
    mentions_reach(
        &world.session,
        channel,
        UnreadCount::Exact(0),
        "read channel to clear",
    )
    .await?;

    // A live row extends coverage a REST page already proved, so the count stays exact.
    let mention = papo_sdk::mentions::encode_user_mention(world.me);
    send(
        &world.owner,
        channel,
        &format!("{mention} live after reading"),
        None,
    )
    .await?;
    let state = settled(&world.session, channel, 1, "a live row after the read").await?;
    assert_eq!(
        state.mention_count,
        UnreadCount::Exact(1),
        "live after read"
    );
    Ok(())
}
