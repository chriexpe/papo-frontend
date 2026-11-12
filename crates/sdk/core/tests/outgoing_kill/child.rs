//! Killed subprocess, using the real SDK and an ephemeral in-memory host credential capability.

use super::*;
use papo_sdk::{
    AccountId, Client, ClientConfig, DataRoot, DraftBody, OperationKind, OutgoingAttachment,
    OutgoingDispatchPhase, OutgoingDispatchProbe, OutgoingMessage, ServerAddress,
    secret::{CredentialGeneration, SecretStore, SecretValue, test_store::ScriptedSecretStore},
};
use std::{
    future::{Future, pending},
    io::Write,
    pin::Pin,
    sync::Arc,
};

struct Probe {
    phase: Option<OutgoingDispatchPhase>,
}
impl OutgoingDispatchProbe for Probe {
    fn reached(
        &self,
        id: Uuid,
        phase: OutgoingDispatchPhase,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if id == local(1) && Some(phase) == self.phase {
                println!("PAPO_BOUNDARY");
                std::io::stdout().flush().unwrap();
                pending::<()>().await;
            }
        })
    }
}
#[test]
#[ignore = "SIGKILL child, run only by outgoing_kill parent"]
fn outgoing_child() {
    let mode = std::env::var("PAPO_KILL_MODE").unwrap();
    let user = std::env::var("PAPO_KILL_ACCOUNT").unwrap().parse().unwrap();
    let root = std::path::PathBuf::from(std::env::var_os("PAPO_KILL_ROOT").unwrap());
    let db = rusqlite::Connection::open(root.join("state.sqlite")).unwrap();
    let address: String = db
        .query_row("SELECT canonical_address FROM servers", [], |row| {
            row.get(0)
        })
        .unwrap();
    let server = ServerAddress::parse(&address).unwrap().id();
    drop(db);
    let account = AccountId::new(server, user);
    let generation = CredentialGeneration::new(
        std::env::var("PAPO_KILL_GENERATION")
            .unwrap()
            .parse()
            .unwrap(),
    );
    let secret = STANDARD
        .decode(std::env::var("PAPO_KILL_SECRET").unwrap())
        .unwrap();
    let store = Arc::new(ScriptedSecretStore::default());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(store.write(
        SecretKey::new(account, generation),
        SecretValue::new(secret),
    ))
    .unwrap();
    drop(rt);
    let phase = match mode.as_str() {
        "queued" => Some(OutgoingDispatchPhase::Queued),
        "claimed" => Some(OutgoingDispatchPhase::Claimed),
        "response" => Some(OutgoingDispatchPhase::Response),
        "sent" => Some(OutgoingDispatchPhase::Committed),
        _ => None,
    };
    let client = Client::open(
        DataRoot::new(root),
        store,
        ClientConfig {
            insecure_loopback: true,
            outgoing_probe: Some(Arc::new(Probe { phase })),
            ..Default::default()
        },
    )
    .unwrap();
    client
        .block_on(async {
            let session = client.handle().session(account).unwrap();
            fixture::online(&session).await.unwrap();
            let queue = session.outgoing();
            let a = std::env::var("PAPO_KILL_CHANNEL_A")
                .unwrap()
                .parse()
                .unwrap();
            let b = std::env::var("PAPO_KILL_CHANNEL_B")
                .unwrap()
                .parse()
                .unwrap();
            let revision = queue
                .save_draft(a, draft(), UnixMicros(1), Some(0))
                .await
                .unwrap();
            queue
                .enqueue_draft(a, local(1), revision, UnixMicros(10))
                .await
                .unwrap();
            for (id, channel) in [(local(2), a), (local(3), b)] {
                queue
                    .enqueue(
                        channel,
                        id,
                        OperationKind::Message(OutgoingMessage {
                            content: "synthetic".into(),
                            ..Default::default()
                        }),
                        UnixMicros(0),
                    )
                    .await
                    .unwrap();
            }
            println!("PAPO_ENQUEUED");
            std::io::stdout().flush().unwrap();
            pending::<()>().await;
        })
        .unwrap();
}

fn draft() -> DraftBody {
    DraftBody {
        message: OutgoingMessage {
            content: "synthetic".into(),
            attachments: vec![OutgoingAttachment {
                filename: "kill.bin".into(),
                bytes: vec![0, 255, 1],
            }],
            ..Default::default()
        },
        ..Default::default()
    }
}
