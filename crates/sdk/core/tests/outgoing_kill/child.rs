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
    any_id: bool,
}
impl OutgoingDispatchProbe for Probe {
    fn reached(
        &self,
        id: Uuid,
        phase: OutgoingDispatchPhase,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if (self.any_id || id == local(1)) && Some(phase) == self.phase {
                println!("PAPO_BOUNDARY");
                std::io::stdout().flush().unwrap();
                pending::<()>().await;
            }
        })
    }
}

pub(super) struct PatternReader {
    remaining: u64,
    offset: u64,
    seed: u8,
}

impl PatternReader {
    pub(super) fn new(len: u64, seed: u8) -> Self {
        Self {
            remaining: len,
            offset: 0,
            seed,
        }
    }
}

impl std::io::Read for PatternReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let want = usize::try_from(self.remaining.min(buf.len() as u64)).unwrap_or(buf.len());
        for (i, slot) in buf[..want].iter_mut().enumerate() {
            let pos = self.offset + i as u64;
            *slot = ((pos.wrapping_mul(31) ^ u64::from(self.seed)) & 0xff) as u8;
        }
        self.offset += want as u64;
        self.remaining -= want as u64;
        Ok(want)
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
    let (phase, any_id) = match mode.as_str() {
        "staging" => (Some(OutgoingDispatchPhase::Staging), true),
        "queued" => (Some(OutgoingDispatchPhase::Queued), false),
        "claimed" => (Some(OutgoingDispatchPhase::Claimed), false),
        "response" => (Some(OutgoingDispatchPhase::Response), false),
        "sent" => (Some(OutgoingDispatchPhase::Committed), false),
        _ => (None, false),
    };
    let client = Client::open(
        DataRoot::new(root),
        store,
        ClientConfig {
            insecure_loopback: true,
            outgoing_probe: Some(Arc::new(Probe { phase, any_id })),
            ..Default::default()
        },
    )
    .unwrap();
    client
        .block_on(run_child_scenario(&client, account, &mode))
        .unwrap();
}

async fn run_child_scenario(client: &Client, account: AccountId, mode: &str) {
    let session = client.handle().session(account).unwrap();
    fixture::online(&session).await.unwrap();
    let queue = session.outgoing();
    if mode == "staging" {
        let _ = queue
            .stage_attachment(PatternReader::new(4 * 1024 * 1024, 0x42))
            .await;
        pending::<()>().await;
    }
    let a = std::env::var("PAPO_KILL_CHANNEL_A")
        .unwrap()
        .parse()
        .unwrap();
    let b = std::env::var("PAPO_KILL_CHANNEL_B")
        .unwrap()
        .parse()
        .unwrap();
    let draft_body = if mode == "upload_mid" {
        let staged = queue
            .stage_attachment(PatternReader::new(super::UPLOAD_MID_BYTES, 0x6b))
            .await
            .unwrap();
        DraftBody {
            message: OutgoingMessage {
                content: "upload-mid-kill".into(),
                attachments: vec![OutgoingAttachment::from_staged("mid.bin", staged)],
                ..Default::default()
            },
            ..Default::default()
        }
    } else {
        draft()
    };
    let revision = queue
        .save_draft(a, draft_body, UnixMicros(1), Some(0))
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
}

fn draft() -> DraftBody {
    DraftBody {
        message: OutgoingMessage {
            content: "https://example.invalid/synthetic.gif".into(),
            gif: Some(papo_sdk::Gif {
                id: "kill-gif".into(),
                url: "https://example.invalid/synthetic.gif".into(),
                page_url: "https://example.invalid/synthetic".into(),
                title: String::new(),
            }),
            attachments: vec![OutgoingAttachment::from_bytes("kill.bin", vec![0, 255, 1])],
            ..Default::default()
        },
        ..Default::default()
    }
}
