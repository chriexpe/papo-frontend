//! Public expression persistence follows confirmed outgoing transitions; no independent send path is exercised.

use super::*;
use papo_sdk::{ExpressionError, Gif, Sticker};

fn gif() -> Gif {
    Gif {
        id: "synthetic-gif".into(),
        url: "https://example.invalid/synthetic.gif".into(),
        page_url: "https://example.invalid/synthetic".into(),
        title: String::new(),
    }
}

#[test]
fn favorites_survive_restart_without_catalog_configuration_and_recents_require_sent() -> Result {
    let mut f = Fixture::new(Config::default())?;
    let (session, channel) = f.run(signed_in(&f))??;
    let account = session.account();
    f.run(async {
        let library = session.expressions();
        library.favorite_gif(gif(), true).await?;
        library
            .favorite_sticker(
                Sticker {
                    id: id(77),
                    name: "fixture".into(),
                },
                true,
            )
            .await?;
        assert!(matches!(
            library.trending(0).await,
            Err(ExpressionError::Unavailable)
        ));
        assert_eq!(library.snapshot().await?.recent_gifs, [] as [Gif; 0]);
        let mut hold = f.server().hold_next_rest("POST", "/messages").await?;
        session
            .outgoing()
            .enqueue(
                channel,
                id(81),
                OperationKind::Message(gif().message()?),
                UnixMicros(0),
            )
            .await?;
        hold.wait_started().await?;
        assert_eq!(library.snapshot().await?.recent_gifs, [] as [Gif; 0]);
        hold.release();
        state(&session, id(81), OperationState::Sent).await?;
        assert_eq!(library.snapshot().await?.recent_gifs, [gif()]);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    drop(session);
    f.reopen()?;
    f.run(async {
        let session = f.client().handle().session(account)?;
        fixture::online(&session).await?;
        let snapshot = session.expressions().snapshot().await?;
        assert_eq!(snapshot.favorite_gifs, [gif()]);
        assert_eq!(snapshot.recent_gifs, [gif()]);
        assert_eq!(snapshot.favorite_stickers.len(), 1);
        f.client().handle().remove_account(account).await?;
        assert!(session.expressions().snapshot().await.is_err());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn rejected_and_ambiguous_sends_never_record_a_recent() -> Result {
    for (status, wanted) in [
        (400, OperationState::Failed),
        (503, OperationState::Unknown),
    ] {
        let f = Fixture::new(Config {
            rest: vec![RestStep {
                method: "POST".into(),
                path: "/messages".into(),
                delay_ms: 0,
                failure: Some(Failure {
                    status,
                    slug: "internal".into(),
                }),
            }],
            ..Config::default()
        })?;
        f.run(async {
            let (session, channel) = signed_in(&f).await?;
            session
                .outgoing()
                .enqueue(
                    channel,
                    id(82),
                    OperationKind::Message(gif().message()?),
                    UnixMicros(0),
                )
                .await?;
            state(&session, id(82), wanted).await?;
            assert_eq!(
                session.expressions().snapshot().await?.recent_gifs,
                [] as [Gif; 0]
            );
            assert_eq!(f.server().outgoing_request_counts().await.requests, 1);
            Ok::<_, Box<dyn std::error::Error>>(())
        })??;
        f.finish()?;
    }
    Ok(())
}
