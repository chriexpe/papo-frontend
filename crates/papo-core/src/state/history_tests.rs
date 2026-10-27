use super::*;

fn wire(index: u32) -> models::Message {
    serde_json::from_value(serde_json::json!({
        "id": format!("m{index:03}"), "channel_id": "chat", "author_id": "other",
        "content": format!("message {index}"),
        "created_at": DateTime::from_timestamp(1_000 + i64::from(index), 123_456_789).unwrap(),
    }))
    .unwrap()
}

fn fresh() -> Store {
    let mut store = Store {
        selected_channel: "chat".into(),
        me: "me".into(),
        ..Store::default()
    };
    store.apply(Update::Connection(Connection::Online));
    let ticket = store.mark_loading("chat");
    store.apply(Update::Messages {
        ticket,
        messages: vec![wire(10), wire(11), wire(12)],
        has_more: true,
        pinned_ids: None,
    });
    store.take_cache_ops();
    store
}

fn open_context(store: &mut Store) {
    let ticket = store.begin_message_context("chat");
    store.apply(Update::MessageContext {
        ticket,
        message_id: "m002".into(),
        context: models::MessageContext {
            channel_id: "chat".into(),
            messages: vec![wire(3), wire(1), wire(2)],
            has_more_before: true,
            has_more_after: true,
        },
    });
}

fn ids(store: &Store) -> Vec<&str> {
    store
        .messages_in("chat")
        .map(|message| message.id.as_str())
        .collect()
}

#[test]
fn distant_context_is_separate_from_head_and_never_persisted_as_continuous_history() {
    let mut store = fresh();
    open_context(&mut store);
    assert_eq!(ids(&store), ["m001", "m002", "m003"]);
    assert_eq!(store.messages.len(), 3);
    assert!(store.take_cache_ops().is_empty());
    store.end_message_context("chat");
    assert_eq!(ids(&store), ["m010", "m011", "m012"]);
    assert_eq!(store.begin_load_older("chat").unwrap().1, "m010");
}

#[test]
fn cancelled_or_superseded_context_cannot_replace_the_current_view() {
    let mut store = fresh();
    let stale = store.begin_message_context("chat");
    let current = store.begin_message_context("chat");
    store.apply(Update::MessageContext {
        ticket: stale,
        message_id: "m002".into(),
        context: models::MessageContext {
            channel_id: "chat".into(),
            messages: vec![wire(2)],
            has_more_before: true,
            has_more_after: true,
        },
    });
    assert!(!store.browsing_history("chat"));
    assert!(store.loading_message_context("chat"));
    store.end_message_context("chat");
    store.apply(Update::MessageContextFailed {
        ticket: current,
        message: "old error".into(),
    });
    assert!(store.error.is_none());
}

#[test]
fn live_head_arrivals_do_not_bridge_a_context_gap() {
    let mut store = fresh();
    open_context(&mut store);
    store.apply(Update::Event(Box::new(Event::Message(Box::new(wire(13))))));
    assert_eq!(ids(&store), ["m001", "m002", "m003"]);
    assert!(store.messages.iter().any(|message| message.id == "m013"));
    store.end_message_context("chat");
    assert_eq!(ids(&store).last(), Some(&"m013"));
}

#[test]
fn context_replays_edits_and_deletions_received_during_fetch() {
    let mut store = fresh();
    let ticket = store.begin_message_context("chat");
    store.apply(Update::Event(Box::new(Event::MessageEdited {
        id: "m002".into(),
        channel_id: "chat".into(),
        content: "edited while loading".into(),
    })));
    store.apply(Update::Event(Box::new(Event::MessageDeleted {
        id: "m001".into(),
        channel_id: "chat".into(),
    })));
    store.apply(Update::MessageContext {
        ticket,
        message_id: "m002".into(),
        context: models::MessageContext {
            channel_id: "chat".into(),
            messages: vec![wire(1), wire(2), wire(3)],
            has_more_before: false,
            has_more_after: true,
        },
    });
    assert_eq!(ids(&store), ["m002", "m003"]);
    assert_eq!(
        store.message("m002").unwrap().content,
        "edited while loading"
    );
}

#[test]
fn deleted_context_target_reports_failure_and_preserves_head() {
    let mut store = fresh();
    let ticket = store.begin_message_context("chat");
    store.apply(Update::Event(Box::new(Event::MessageDeleted {
        id: "m002".into(),
        channel_id: "chat".into(),
    })));
    store.apply(Update::MessageContext {
        ticket,
        message_id: "m002".into(),
        context: models::MessageContext {
            channel_id: "chat".into(),
            messages: vec![wire(2)],
            has_more_before: false,
            has_more_after: true,
        },
    });
    assert!(!store.browsing_history("chat"));
    assert!(store.error.is_some());
}

#[test]
fn context_pages_browse_both_directions_without_changing_head_cursor() {
    let mut store = fresh();
    store.cache_history_has_more.insert("chat".into(), true);
    open_context(&mut store);
    assert!(store.begin_load_cached_older("chat").is_none());
    assert_eq!(store.begin_load_older("chat").unwrap().1, "m001");
    let ticket = store.history_ticket("chat").unwrap();
    store.apply(Update::OlderMessages {
        ticket,
        messages: vec![wire(0)],
        has_more: false,
    });
    assert_eq!(ids(&store), ["m000", "m001", "m002", "m003"]);
    assert!(!store.can_load_older("chat"));
    assert_eq!(store.begin_load_newer("chat").unwrap().1, "m003");
    let ticket = store.history_ticket("chat").unwrap();
    store.apply(Update::NewerMessages {
        ticket,
        messages: vec![wire(4), wire(5)],
        has_more: true,
    });
    assert_eq!(ids(&store).last(), Some(&"m005"));
    assert!(store.can_load_newer("chat"));
    store.end_message_context("chat");
    assert!(store.can_load_older("chat"));
    assert_eq!(store.begin_load_older("chat").unwrap().1, "m010");
}

#[test]
fn stale_history_completion_cannot_merge_after_refresh_or_reconnect() {
    for reconnect in [false, true] {
        let mut store = fresh();
        store.begin_load_older("chat").unwrap();
        let ticket = store.history_ticket("chat").unwrap();
        if reconnect {
            store.apply(Update::Connection(Connection::Offline));
        } else {
            store.mark_loading("chat");
        }
        store.apply(Update::OlderMessages {
            ticket,
            messages: vec![wire(1)],
            has_more: false,
        });
        assert!(store.message("m001").is_none());
    }
}

#[test]
fn late_cache_page_cannot_undo_a_live_edit_or_a_new_authoritative_window() {
    let mut store = fresh();
    store.apply(Update::Event(Box::new(Event::MessageEdited {
        id: "m010".into(),
        channel_id: "chat".into(),
        content: "live edit".into(),
    })));
    store.restore_cached_page(
        CachedMessagePage {
            channel_id: "chat".into(),
            messages: vec![
                CachedMessage::from_api(&wire(9)),
                CachedMessage::from_api(&wire(10)),
            ],
            has_more: true,
            pins: Vec::new(),
        },
        false,
    );
    assert_eq!(store.message("m010").unwrap().content, "live edit");
    assert!(store.message("m009").is_some());
    // The server cursor remains m010: the cached prefix needs revalidation.
    assert_eq!(store.begin_load_older("chat").unwrap().1, "m010");
}

#[test]
fn paginating_online_revalidates_cached_rows_and_removes_missing_messages() {
    let mut store = fresh();
    store.restore_cached_page(
        CachedMessagePage {
            channel_id: "chat".into(),
            messages: vec![
                CachedMessage::from_api(&wire(8)),
                CachedMessage::from_api(&wire(9)),
                CachedMessage::from_api(&wire(10)),
            ],
            has_more: true,
            pins: Vec::new(),
        },
        false,
    );
    assert_eq!(store.begin_load_cached_older("chat").unwrap().2, "m008");
    let cached_ticket = store.history_ticket("chat").unwrap();
    store.restore_cached_history(&cached_ticket, CachedMessagePage {
        channel_id: "chat".into(),
        messages: vec![CachedMessage::from_api(&wire(6)), CachedMessage::from_api(&wire(7))],
        has_more: false,
        pins: Vec::new(),
    });
    assert!(store.message("m006").is_some(), "Cached history is usable before HTTP completes");
    assert!(!store.loading_older("chat"));
    // Local extension must not advance the server cursor and hide edits/deletions behind it.
    assert_eq!(store.begin_load_older("chat").unwrap().1, "m010");
    let ticket = store.history_ticket("chat").unwrap();
    let mut edited = wire(8);
    edited.content = Some("server edit".into());
    store.apply(Update::OlderMessages {
        ticket,
        messages: vec![edited],
        has_more: false,
    });
    assert!(store.message("m009").is_none());
    assert_eq!(store.message("m008").unwrap().content, "server edit");
    assert!(!store.can_load_older("chat"));
}

#[test]
fn history_snapshot_cannot_resurrect_a_concurrent_delete() {
    let mut store = fresh();
    store.begin_load_older("chat").unwrap();
    let ticket = store.history_ticket("chat").unwrap();
    store.apply(Update::Event(Box::new(Event::MessageDeleted {
        id: "m009".into(),
        channel_id: "chat".into(),
    })));
    store.apply(Update::OlderMessages {
        ticket,
        messages: vec![wire(9)],
        has_more: true,
    });
    assert!(store.message("m009").is_none());
    assert!(
        matches!(store.take_cache_ops().last(), Some(CacheOp::DeleteMessage { message_id }) if message_id == "m009")
    );
}

#[test]
fn cached_newer_preview_keeps_the_http_cursor_and_revalidates_missing_rows() {
    let mut store = fresh();
    open_context(&mut store);
    assert_eq!(store.begin_load_newer("chat").unwrap().1, "m003");
    let ticket = store.history_ticket("chat").unwrap();
    store.preview_cached_history(&ticket, CachedMessagePage {
        channel_id: "chat".into(),
        messages: (4..=8).map(|n| CachedMessage::from_api(&wire(n))).collect(),
        has_more: false,
        pins: Vec::new(),
    });
    assert!(store.loading_older("chat"), "The HTTP request remains in flight");
    assert_eq!(ids(&store), ["m001", "m002", "m003", "m004", "m005", "m006", "m007", "m008"]);
    let mut changed = wire(4);
    changed.content = Some("server edit".into());
    store.apply(Update::NewerMessages {
        ticket,
        messages: vec![changed, wire(6)],
        has_more: true,
    });
    assert_eq!(ids(&store), ["m001", "m002", "m003", "m004", "m006", "m007", "m008"]);
    assert_eq!(store.messages_in("chat").find(|m| m.id == "m004").unwrap().content, "server edit");
    assert_eq!(store.begin_load_newer("chat").unwrap().1, "m006", "Cached m008 is not a validated cursor");
    let ticket = store.history_ticket("chat").unwrap();
    store.apply(Update::NewerMessages { ticket, messages: vec![wire(8)], has_more: false });
    assert!(!store.messages_in("chat").any(|m| m.id == "m007"));
    assert!(!store.can_load_newer("chat"));
    assert!(store.take_cache_ops().iter().all(|op| !matches!(op,
        CacheOp::AppendChannelHistory { .. } | CacheOp::MergeChannelHead { .. }
    )), "Detached context must never become continuous cached history");
}

#[test]
fn cached_older_context_does_not_skip_validation_or_keep_deleted_rows() {
    let mut store = fresh();
    let ticket = store.begin_message_context("chat");
    store.apply(Update::MessageContext {
        ticket, message_id: "m008".into(),
        context: models::MessageContext {
            channel_id: "chat".into(), messages: vec![wire(7), wire(8), wire(9)],
            has_more_before: true, has_more_after: true,
        },
    });
    assert_eq!(store.begin_load_older("chat").unwrap().1, "m007");
    let ticket = store.history_ticket("chat").unwrap();
    store.preview_cached_history(&ticket, CachedMessagePage {
        channel_id: "chat".into(),
        messages: (0..=6).map(|n| CachedMessage::from_api(&wire(n))).collect(),
        has_more: false, pins: Vec::new(),
    });
    assert!(store.messages_in("chat").any(|m| m.id == "m000"));
    store.apply(Update::OlderMessages { ticket, messages: vec![wire(5), wire(6)], has_more: true });
    assert_eq!(store.begin_load_older("chat").unwrap().1, "m005");
    let ticket = store.history_ticket("chat").unwrap();
    store.apply(Update::OlderMessages { ticket, messages: Vec::new(), has_more: false });
    assert_eq!(ids(&store), ["m005", "m006", "m007", "m008", "m009"]);
    assert!(!store.can_load_older("chat"));
}

#[test]
fn malformed_or_non_advancing_pages_fail_without_poisoning_history() {
    let mut store = fresh();
    store.begin_load_older("chat").unwrap();
    let ticket = store.history_ticket("chat").unwrap();
    store.apply(Update::OlderMessages {
        ticket,
        messages: vec![wire(11)],
        has_more: true,
    });
    assert_eq!(ids(&store), ["m010", "m011", "m012"]);
    assert!(!store.loading_older("chat"));
    assert!(!store.can_load_older("chat")); // bounded retry delay
    assert!(store.error.is_some());
}

#[test]
fn browsing_context_never_confirms_the_unseen_gap_as_read() {
    let mut store = fresh();
    store.read_states.insert(
        "chat".into(),
        ChannelReadState {
            read_at: Some(wire(0).created_at),
            read_message_id: Some("m000".into()),
            ..ChannelReadState::default()
        },
    );
    open_context(&mut store);
    store.observe_visible_messages_with_policy(
        "chat",
        &["m001".into(), "m002".into(), "m003".into()],
        true,
    );
    assert_eq!(
        store.read_states["chat"].read_message_id.as_deref(),
        Some("m000")
    );
}

#[test]
fn late_pin_snapshot_replays_live_unpin_and_preserves_new_pin() {
    let mut store = fresh();
    let ticket = store.begin_load_pinned("chat", true).unwrap();
    store.apply(Update::PinChanged {
        channel_id: "chat".into(),
        message_id: "m010".into(),
        pinned: false,
    });
    store.apply(Update::PinChanged {
        channel_id: "chat".into(),
        message_id: "m011".into(),
        pinned: true,
    });
    store.apply(Update::PinnedMessages {
        ticket: Some(ticket),
        channel_id: "chat".into(),
        messages: vec![wire(10)],
    });
    assert_eq!(
        store.pinned_messages["chat"]
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        ["m011"]
    );
    assert!(!store.known_pins["chat"].contains("m010"));
    assert!(store.known_pins["chat"].contains("m011"));
}

#[test]
fn stale_pin_snapshot_is_ignored_after_disconnect() {
    let mut store = fresh();
    let ticket = store.begin_load_pinned("chat", true).unwrap();
    store.apply(Update::Connection(Connection::Offline));
    store.apply(Update::PinnedMessages {
        ticket: Some(ticket),
        channel_id: "chat".into(),
        messages: vec![wire(10)],
    });
    assert!(!store.pinned_messages.contains_key("chat"));
}

#[test]
fn late_cache_hydration_does_not_overwrite_fresh_context_content() {
    let mut store = fresh();
    open_context(&mut store);
    let mut stale = CachedMessage::from_api(&wire(2));
    stale.content = "old cached text".into();
    store.restore_cached_page(
        CachedMessagePage {
            channel_id: "chat".into(),
            messages: vec![stale, CachedMessage::from_api(&wire(10))],
            pins: Vec::new(),
            has_more: true,
        },
        false,
    );
    assert_eq!(store.message("m002").unwrap().content, "message 2");
}

#[test]
fn all_own_message_window_is_a_valid_overlap_when_it_was_fetched_as_history() {
    let mut store = fresh();
    let mut cached_old = CachedMessage::from_api(&wire(9));
    cached_old.author_id = "me".into();
    let mut cached_overlap = CachedMessage::from_api(&wire(10));
    cached_overlap.author_id = "me".into();
    store.restore_cached_page(
        CachedMessagePage {
            channel_id: "chat".into(),
            messages: vec![cached_old, cached_overlap],
            has_more: true,
            pins: Vec::new(),
        },
        false,
    );
    let mut own = wire(10);
    own.author_id = "me".into();
    let ticket = store.mark_loading("chat");
    store.apply(Update::Messages {
        ticket,
        messages: vec![own, wire(11), wire(12)],
        has_more: true,
        pinned_ids: None,
    });
    assert!(store.message("m009").is_some());
}

#[test]
fn live_arrivals_during_head_refresh_extend_proven_cache_coverage_after_replay() {
    let mut store = fresh();
    let ticket = store.mark_loading("chat");
    store.apply(Update::Event(Box::new(Event::Message(Box::new(wire(13))))));
    store.apply(Update::Messages {
        ticket,
        messages: vec![wire(10), wire(11), wire(12)],
        has_more: true,
        pinned_ids: None,
    });
    assert!(store.timeline_ids.contains("m013"));
    assert!(store.take_cache_ops().iter().any(|op| matches!(op, CacheOp::AppendChannelHistory { messages, .. } if messages.iter().any(|message| message.id == "m013"))));
}

#[test]
fn editing_a_distant_context_message_does_not_insert_it_into_the_live_head() {
    let mut store = fresh();
    open_context(&mut store);
    let mut edited = wire(2);
    edited.content = Some("edited context".into());
    store.apply(Update::Edited(Box::new(edited)));
    assert_eq!(store.message("m002").unwrap().content, "edited context");
    assert!(!store.messages.iter().any(|message| message.id == "m002"));
    store.end_message_context("chat");
    assert_eq!(ids(&store), ["m010", "m011", "m012"]);
}

#[test]
fn first_live_arrival_after_an_empty_head_establishes_a_persistable_boundary() {
    let mut store = Store {
        selected_channel: "chat".into(),
        ..Store::default()
    };
    store.apply(Update::Connection(Connection::Online));
    let ticket = store.mark_loading("chat");
    store.apply(Update::Messages {
        ticket,
        messages: Vec::new(),
        has_more: false,
        pinned_ids: None,
    });
    store.take_cache_ops();
    store.apply(Update::Event(Box::new(Event::Message(Box::new(wire(1))))));
    assert_eq!(store.history_server_cursor["chat"].1, "m001");
    assert!(store.take_cache_ops().iter().any(|op| matches!(op, CacheOp::ReplaceChannelSnapshot { messages, .. } if messages.len() == 1 && messages[0].id == "m001")));
}
