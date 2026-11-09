//! Large durable cleanup remains atomic and is not a shared-cache scheduling turn.

use crate::storage_tests::{Fixture, account, address};
use crate::{StorageError, UnixMicros};

#[test]
fn removal_deletes_more_than_one_shared_turn_of_state_without_losing_other_accounts() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let server=address(1); let first=account(&server,1); let other=account(&server,2);
        let registry=fixture.storage().registry();
        registry.add_server(server,UnixMicros(1)).await.unwrap();
        for id in [first,other] {registry.add_account(id,None,None,false,UnixMicros(1)).await.unwrap();}
        registry.writer.write(move |tx|->Result<(),StorageError> {
            for index in 0..601u128 {
                let channel=papo_api::Uuid::from_u128(index);
                tx.execute("INSERT INTO drafts(server_id,user_id,channel_id,text,mentions,updated_at_us) VALUES(?1,?2,?3,'',x'',1)",rusqlite::params![first.server().as_bytes(),first.user().as_bytes(),channel.as_bytes()])?;
            }
            for id in [first,other] {
                tx.execute("INSERT INTO operations(local_id,server_id,user_id,channel_id,kind,payload,state,created_at_us) VALUES(?1,?2,?3,zeroblob(16),'message',x'','Queued',1)",rusqlite::params![id.user().as_bytes(),id.server().as_bytes(),id.user().as_bytes()])?;
            }
            Ok(())
        }).await.unwrap();
        let jobs=fixture.storage().removals();
        jobs.begin_account(first).await.unwrap();
        assert_eq!(jobs.resume().await.unwrap(), [] as [crate::removal::RemovalJob; 0]);
        let (drafts,operations):(i64,i64)=registry.readers.query(registry.path.clone(),|tx| {
            Ok((tx.query_row("SELECT count(*) FROM drafts",[],|r|r.get(0))?,tx.query_row("SELECT count(*) FROM operations",[],|r|r.get(0))?))
        }).await.unwrap();
        assert_eq!((drafts,operations),(0,1));
        assert_eq!(registry.accounts(other.server()).await.unwrap()[0].id,other);
    });
}
