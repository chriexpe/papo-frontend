//! Registry behavior across FULL commits and profile restarts; no live services.

use crate::storage_tests::{Fixture, account, address};
use crate::{StorageError, UnixMicros};

#[test]
fn server_and_account_order_and_active_selection_survive_restart() {
    let mut fixture = Fixture::new();
    let a = address(1);
    let b = address(2);
    let first = account(&a, 1);
    let second = account(&a, 2);
    fixture.runtime().block_on(async {
        let registry = fixture.storage().registry();
        registry.add_server(a.clone(), UnixMicros(1)).await.unwrap();
        registry.add_server(b.clone(), UnixMicros(2)).await.unwrap();
        registry
            .add_account(first, None, None, true, UnixMicros(3))
            .await
            .unwrap();
        registry
            .add_account(second, None, None, true, UnixMicros(4))
            .await
            .unwrap();
        registry
            .reorder_servers(vec![b.id(), a.id()])
            .await
            .unwrap();
        registry
            .reorder_accounts(a.id(), vec![second, first])
            .await
            .unwrap();
        registry
            .set_active_account(a.id(), Some(first))
            .await
            .unwrap();
    });
    fixture.reopen();
    fixture.runtime().block_on(async {
        let registry = fixture.storage().registry();
        assert_eq!(
            registry
                .servers()
                .await
                .unwrap()
                .iter()
                .map(|s| s.address.id())
                .collect::<Vec<_>>(),
            [b.id(), a.id()]
        );
        let accounts = registry.accounts(a.id()).await.unwrap();
        assert_eq!(
            accounts.iter().map(|a| a.id).collect::<Vec<_>>(),
            [second, first]
        );
        assert_eq!(
            accounts.iter().map(|a| a.active).collect::<Vec<_>>(),
            [false, true]
        );
    });
}

#[test]
fn invalid_order_rolls_back_and_does_not_close_the_state_writer() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let registry = fixture.storage().registry();
        let a = address(1);
        let b = address(2);
        registry.add_server(a.clone(), UnixMicros(1)).await.unwrap();
        registry.add_server(b.clone(), UnixMicros(2)).await.unwrap();
        assert!(matches!(
            registry.reorder_servers(vec![b.id(), b.id()]).await,
            Err(StorageError::InvalidOrder)
        ));
        assert!(matches!(
            registry
                .reorder_servers(vec![b.id(), address(3).id()])
                .await,
            Err(StorageError::InvalidOrder)
        ));
        assert!(matches!(
            registry.add_server(a.clone(), UnixMicros(3)).await,
            Err(StorageError::AlreadyExists)
        ));
        assert_eq!(
            registry
                .servers()
                .await
                .unwrap()
                .iter()
                .map(|s| s.address.id())
                .collect::<Vec<_>>(),
            [a.id(), b.id()]
        );
        registry
            .add_server(address(3), UnixMicros(3))
            .await
            .unwrap();
    });
}

#[test]
fn active_selection_is_per_server_and_rejects_cross_account_identity() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let registry = fixture.storage().registry();
        let a = address(1);
        let b = address(2);
        let first = account(&a, 1);
        let second = account(&a, 2);
        let other = account(&b, 1);
        for address in [&a, &b] {
            registry
                .add_server(address.clone(), UnixMicros(1))
                .await
                .unwrap();
        }
        for id in [first, second, other] {
            registry
                .add_account(id, None, None, true, UnixMicros(1))
                .await
                .unwrap();
        }
        assert!(matches!(
            registry.set_active_account(a.id(), Some(other)).await,
            Err(StorageError::NotFound)
        ));
        assert_eq!(
            registry
                .accounts(a.id())
                .await
                .unwrap()
                .iter()
                .map(|a| a.active)
                .collect::<Vec<_>>(),
            [false, true]
        );
        assert!(registry.accounts(b.id()).await.unwrap()[0].active);
        registry.set_active_account(a.id(), None).await.unwrap();
        assert!(
            registry
                .accounts(a.id())
                .await
                .unwrap()
                .iter()
                .all(|a| !a.active)
        );
    });
}
