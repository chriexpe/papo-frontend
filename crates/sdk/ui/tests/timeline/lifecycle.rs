//! Public idle timeline streams retain typed closure after access loss, replacement and removal.

use papo_sdk::{HistoryClosedReason, HistoryError};
use papo_sdk_ui::{Timeline, TimelineDirection, TimelineError};
use papo_test_server::{Failure, RestStep};

use super::support::{CHANNEL, Fixture, Result, channel, closed, config, ids, login, online};

fn access_refusal(expected: HistoryClosedReason) -> Result {
    let path = format!("/channels/{CHANNEL}/messages");
    let rest = if expected == HistoryClosedReason::NotFound {
        vec![
            RestStep {
                method: "GET".into(),
                path: path.clone(),
                delay_ms: 0,
                failure: None,
            },
            RestStep {
                method: "GET".into(),
                path,
                delay_ms: 0,
                failure: Some(Failure {
                    status: 404,
                    slug: "not-found".into(),
                }),
            },
        ]
    } else {
        vec![]
    };
    let fixture = Fixture::new(80, rest)?;
    fixture.run(async {
        let session = fixture.signed_in("timeline-refused").await?;
        let timeline = Timeline::live(session.clone(), channel(), config()).await?;
        let mut idle = timeline.subscribe()?;
        assert_eq!(ids(&idle).len(), 4);
        if expected == HistoryClosedReason::Forbidden {
            fixture.server().set_channel_readable(&session.account().user().to_string(), CHANNEL, false).await?;
        }
        let result = timeline.paginate(TimelineDirection::Older).await;
        assert!(matches!(result,
            Err(TimelineError::History(HistoryError::Closed(reason)) | TimelineError::Closed(reason)) if reason == expected));
        assert_eq!(closed(&mut idle).await?, expected);
        assert!(matches!(timeline.subscribe(), Err(TimelineError::Closed(reason)) if reason == expected));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn public_idle_stream_closes_forbidden_and_evicted_history_cannot_be_resubscribed() -> Result {
    access_refusal(HistoryClosedReason::Forbidden)
}

#[test]
fn public_idle_stream_closes_not_found_and_evicted_history_cannot_be_resubscribed() -> Result {
    access_refusal(HistoryClosedReason::NotFound)
}

#[test]
fn public_idle_streams_close_on_account_replacement_then_removal_without_retargeting() -> Result {
    let fixture = Fixture::new(12, vec![])?;
    fixture.run(async {
        let first = fixture.signed_in("timeline-replace").await?;
        let old = Timeline::live(first.clone(), channel(), config()).await?;
        let mut idle = old.subscribe()?;
        let first_generation = idle.generation;
        let auth = fixture.client().handle().auth(first.account().server())?;
        let current = login(&auth, "timeline-replace").await?;
        online(&current).await?;
        assert_eq!(current.account(), first.account());
        assert_ne!(current.generation(), first_generation);
        assert_eq!(closed(&mut idle).await?, HistoryClosedReason::AccountClosed);
        assert!(matches!(
            old.subscribe(),
            Err(TimelineError::Closed(HistoryClosedReason::AccountClosed))
        ));
        let fresh = Timeline::live(current.clone(), channel(), config()).await?;
        let mut fresh_idle = fresh.subscribe()?;
        assert_eq!(fresh_idle.generation, current.generation());
        assert_eq!(ids(&fresh_idle).len(), 4);
        fixture
            .client()
            .handle()
            .remove_account(current.account())
            .await?;
        assert_eq!(
            closed(&mut fresh_idle).await?,
            HistoryClosedReason::AccountClosed
        );
        assert!(matches!(
            fresh.subscribe(),
            Err(TimelineError::Closed(HistoryClosedReason::AccountClosed))
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
