//! Search surface replacement, lifecycle closure and detached navigation alongside canonical timelines.
use super::support::{Fixture, Result, channel};
use papo_sdk::{SearchQuery, Uuid};
use papo_sdk_ui::{SearchService, SearchUpdate};
use std::sync::Arc;

#[test]
fn search_surface_cancels_old_query_and_invalidates_idle_pages_on_reconnect_and_logout() -> Result {
    let fixture = Fixture::new(0, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("search-ui").await?;
        let payload = serde_json::json!({
            "type": "message",
            "id": Uuid::from_u128(1).to_string(),
            "channel_id": channel().to_string(),
            "created_at": "2026-01-01T00:00:00Z"
        });
        let row = serde_json::from_value(payload)?;
        fixture
            .server()
            .set_search_results(session.account().user(), vec![row])
            .await?;
        let service = Arc::new(SearchService::new(session.clone()));
        let mut subscription = service.subscribe();
        let mut hold = fixture.server().hold_next_rest("POST", "/search").await?;
        let copy = Arc::clone(&service);
        let pending = tokio::spawn(async move {
            copy.search(SearchQuery {
                channel: Some(channel()),
                ..SearchQuery::default()
            })
            .await
        });
        hold.wait_started().await?;
        let page = service
            .search(SearchQuery {
                contains_attachment: Some(false),
                ..SearchQuery::default()
            })
            .await?;
        assert_eq!(page.hits.len(), 1);
        assert!(pending.await?.is_err());
        hold.release();
        assert!(matches!(subscription.current(), SearchUpdate::Ready(_)));
        let target = service.target(&page.hits[0])?;
        assert_eq!(target.channel, channel());
        session.reconnect()?;
        assert!(matches!(subscription.current(), SearchUpdate::Idle));
        assert!(service.target(&page.hits[0]).is_err());
        session.auth().logout().await?;
        assert!(matches!(subscription.next().await, SearchUpdate::Closed));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
