//! Held transport responses cannot cross a reconnect to retire newly authorized media.

use super::*;

#[test]
fn held_old_denial_cannot_revoke_media_fetched_after_reconnect() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2700);
        let source = MediaSource::Attachment(id);
        let path = format!("/attachments/{id}");
        let users = BTreeSet::from([session.account().user().to_string()]);
        f.server()
            .set_encoded_media(encoded(path.clone(), users.clone()))
            .await?;
        session.media().fetch(source.clone(), false).await?;
        let mut rejected = encoded(path.clone(), users.clone());
        rejected.status = 403;
        f.server().set_encoded_media(rejected).await?;
        let mut held = f.server().hold_next_rest("GET", &path).await?;
        let media = session.media();
        let request_source = source.clone();
        let pending = tokio::spawn(async move { media.fetch(request_source, true).await });
        held.wait_started().await?;
        session.reconnect()?;
        fixture::online(&session).await?;
        let mut authorized = encoded(path, users);
        authorized.bytes = vec![9; 8193];
        f.server().set_encoded_media(authorized).await?;
        assert_eq!(
            session
                .media()
                .fetch(source.clone(), true)
                .await?
                .value
                .bytes[0],
            9
        );
        held.release();
        assert!(matches!(
            pending.await?,
            Err(MediaError::Session(papo_sdk::ClientError::Superseded))
        ));
        assert_eq!(session.media().cached(source).await?.value.bytes[0], 9);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
