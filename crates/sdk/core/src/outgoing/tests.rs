//! Deterministic payload bounds and forward recovery; socket ordering and process gates are integration tests.

use super::*;

#[test]
fn payload_bytes_and_stable_local_dependencies_round_trip_without_borrowed_files() {
    let id = Uuid::from_u128(1);
    let kind = OperationKind::Message(OutgoingMessage {
        reply_to: Some(MessageTarget::Local(id)),
        attachments: vec![OutgoingAttachment {
            filename: "SPOILER_pixel.png".into(),
            bytes: vec![0, 255, 1],
        }],
        ..Default::default()
    });
    let json = serde_json::to_vec(&kind).unwrap();
    let restored: OperationKind = serde_json::from_slice(&json).unwrap();
    assert_eq!(restored.dependency(), Some(id));
    let OperationKind::Message(message) = restored else {
        panic!("creation payload changed");
    };
    assert_eq!(message.attachments[0].bytes, [0, 255, 1]);
    assert_eq!(message.attachments[0].filename, "SPOILER_pixel.png");
}

#[test]
fn invalid_filenames_text_bounds_and_ambiguous_reaction_ids_are_rejected() {
    for filename in ["", "../x", "a\\b", "a\r\nb", "a\0b"] {
        let kind = OperationKind::Message(OutgoingMessage {
            attachments: vec![OutgoingAttachment {
                filename: filename.into(),
                bytes: vec![],
            }],
            ..Default::default()
        });
        assert!(matches!(kind.validate(), Err(OutgoingError::InvalidInput)));
    }
    let text = OperationKind::Message(OutgoingMessage {
        content: "x".repeat(8193),
        ..Default::default()
    });
    assert!(text.validate().is_err());
    let target = MessageTarget::Remote(Uuid::from_u128(1));
    for (emoji_id, unicode) in [(None, None), (Some(Uuid::from_u128(2)), Some("x".into()))] {
        assert!(
            OperationKind::Reaction {
                target,
                emoji_id,
                unicode,
                add: true
            }
            .validate()
            .is_err()
        );
    }
}

#[test]
fn v1_upgrade_recovers_only_sending_and_keeps_intents_and_full_durability() {
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(include_str!("../state_v1.sql")).unwrap();
    db.pragma_update(None, "user_version", 1).unwrap();
    db.execute("INSERT INTO servers(id,canonical_address,position,added_at_us) VALUES(zeroblob(16),'https://example.invalid',0,0)",[]).unwrap();
    db.execute("INSERT INTO accounts(server_id,user_id,position,created_at_us) VALUES(zeroblob(16),zeroblob(16),0,0)",[]).unwrap();
    for (index, state) in ["Queued", "Sending", "Sent", "Unknown", "Failed"]
        .into_iter()
        .enumerate()
    {
        db.execute("INSERT INTO operations(local_id,server_id,user_id,channel_id,kind,payload,state,created_at_us) VALUES(?1,zeroblob(16),zeroblob(16),zeroblob(16),'sdk-v1',x'0102',?2,0)",rusqlite::params![Uuid::from_u128(index as u128).as_bytes(),state]).unwrap();
    }
    crate::state_schema::migrate(&mut db).unwrap();
    crate::state_schema::migrate(&mut db).unwrap();
    let states: Vec<String> = db
        .prepare("SELECT state FROM operations ORDER BY rowid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(states, ["Queued", "Unknown", "Sent", "Unknown", "Failed"]);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM operations WHERE payload=x'0102'",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        5
    );
    assert_eq!(
        db.pragma_query_value(None, "synchronous", |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn unicode_reaction_limit_uses_backend_runes_not_utf8_bytes() {
    let target = MessageTarget::Remote(Uuid::from_u128(1));
    assert!(
        OperationKind::Reaction {
            target,
            emoji_id: None,
            unicode: Some("👍".repeat(16)),
            add: true
        }
        .validate()
        .is_ok()
    );
    assert!(
        OperationKind::Reaction {
            target,
            emoji_id: None,
            unicode: Some("👍".repeat(17)),
            add: true
        }
        .validate()
        .is_err()
    );
}
