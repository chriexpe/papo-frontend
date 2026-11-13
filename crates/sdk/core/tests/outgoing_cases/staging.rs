//! Real file-copy isolation from the shared FULL writer; deterministic gates replace timing benchmarks.

use super::*;
use papo_sdk::{OutgoingDispatchPhase, OutgoingDispatchProbe};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::Semaphore;

struct Release(Arc<Pause>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release.add_permits(2);
    }
}

struct Pause {
    entered: Semaphore,
    release: Semaphore,
}
impl OutgoingDispatchProbe for Pause {
    fn reached(
        &self,
        _: Uuid,
        phase: OutgoingDispatchPhase,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if phase == OutgoingDispatchPhase::Staging {
                self.entered.add_permits(1);
                self.release.acquire().await.unwrap().forget();
            }
        })
    }
}
#[test]
fn another_accounts_full_commit_completes_while_all_bulk_copy_slots_are_paused() -> Result {
    let mut f = Fixture::new(Config::default())?;
    let pause = Arc::new(Pause {
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    f.set_outgoing_probe(pause.clone())?;
    f.run(async {
        let _release = Release(pause.clone());
        let (a, channel) = signed_in(&f).await?;
        let auth = f.client().handle().auth(a.account().server())?;
        register(&auth, "staging-other").await?;
        let b = login(&auth, "staging-other").await?;
        fixture::online(&b).await?;
        register(&auth, "staging-third").await?;
        let c = login(&auth, "staging-third").await?;
        fixture::online(&c).await?;
        let queue = a.outgoing();
        let large = queue.save_draft(
            channel,
            DraftBody {
                message: OutgoingMessage {
                    attachments: vec![OutgoingAttachment {
                        filename: "large.bin".into(),
                        bytes: vec![1; 100 * 1024 * 1024],
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
            UnixMicros(0),
            None,
        );
        let other = c.outgoing();
        let large_other = other.save_draft(
            channel,
            DraftBody {
                message: OutgoingMessage {
                    attachments: vec![OutgoingAttachment {
                        filename: "other-large.bin".into(),
                        bytes: vec![2; 100 * 1024 * 1024],
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
            UnixMicros(0),
            None,
        );
        let small = async {
            pause.entered.acquire_many(2).await.unwrap().forget();
            // The first chunk is copied, but 100 MiB staging cannot finish until release.
            let saved = b
                .outgoing()
                .save_draft(channel, DraftBody::default(), UnixMicros(0), None)
                .await;
            pause.release.add_permits(2);
            assert_eq!(saved?, 1);
            Ok::<_, Box<dyn std::error::Error>>(())
        };
        let (large, large_other, small) = tokio::join!(large, large_other, small);
        assert_eq!(large_other?, 1);
        small?;
        assert_eq!(large?, 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

impl Fixture {
    #[cfg(feature = "test-support")]
    pub fn set_outgoing_probe(
        &mut self,
        probe: Arc<dyn papo_sdk::OutgoingDispatchProbe>,
    ) -> Result {
        self.close_client()?;
        self.config.outgoing_probe = Some(probe);
        self.reopen()
    }
}
