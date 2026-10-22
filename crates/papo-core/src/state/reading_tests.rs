use super::*;

fn navigation_message(id: &str, second: i64) -> Message {
    Message {
        id: id.to_owned(),
        channel_id: "geral".to_owned(),
        author_id: "outro".to_owned(),
        external_author: None,
        content: id.to_owned(),
        at: ("2026-10-21T12:00:00Z".parse::<DateTime<Utc>>().unwrap()
            + chrono::Duration::seconds(second))
        .with_timezone(&Local),
        edited: false,
        reply_to: None,
        attachments: Vec::new(),
        previews: Vec::new(),
        reactions: Vec::new(),
        pinned: false,
        pending: false,
    }
}

fn navigation_store(count: usize) -> Store {
    let mut store = Store {
        me: "eu".to_owned(),
        selected_channel: "geral".to_owned(),
        channels: vec![Channel {
            id: "geral".to_owned(),
            name: "Geral".to_owned(),
            kind: ChannelKind::Text,
            topic: None,
            position: 0,
            permissions: Vec::new(),
            notification_settings: "only_mentions".to_owned(),
            parent_id: None,
            unread: true,
            mentions: 0,
        }],
        ..Store::default()
    };
    store.messages = (1..=count)
        .map(|index| navigation_message(&format!("m{index}"), index as i64))
        .collect();
    let first = store.messages[0].clone();
    store.read_states.insert(
        "geral".to_owned(),
        ChannelReadState {
            read_at: Some(first.at.with_timezone(&Utc)),
            read_message_id: Some(first.id),
            ..ChannelReadState::default()
        },
    );
    store
}

fn ids(range: std::ops::RangeInclusive<usize>) -> Vec<String> {
    range.map(|index| format!("m{index}")).collect()
}

fn frontier_id(store: &Store) -> Option<String> {
    store
        .read_states
        .get("geral")
        .and_then(|state| state.read_message_id.clone())
}

#[test]
fn reading_diagnostics_capture_cursor_evidence_without_message_content() {
    let mut store = navigation_store(10);
    store.messages[1].content = "private message body excluded from diagnostics".into();
    store.observe_chat_viewport("geral", &ids(9..=10), true);
    let snapshot = store.reading_diagnostics("geral");
    assert_eq!(snapshot["frontier"][1], "m1");
    assert_eq!(snapshot["acknowledged"][1], "m10");
    assert_eq!(snapshot["gapRange"][0], "m2");
    assert_eq!(snapshot["span"][0][1], "m9");
    assert!(!snapshot.to_string().contains("private message body"));
}

#[test]
fn android_latest_acknowledges_channel_without_erasing_catch_up() {
    let mut store = navigation_store(10);
    store.observe_chat_viewport("geral", &ids(9..=10), true);
    assert!(!store.channel("geral").unwrap().unread);
    assert_eq!(frontier_id(&store).as_deref(), Some("m1"));
    assert_eq!(store.hourglass_target("geral").as_deref(), Some("m2"));
    assert_eq!(
        store.pending_read_receipts(),
        vec![("geral".into(), "m10".into())]
    );
    store.observe_chat_viewport("geral", &ids(7..=8), false);
    assert_eq!(
        store.hourglass_range("geral"),
        Some(("m2".into(), "m8".into()))
    );
    assert!(!store.channel("geral").unwrap().unread);
    store.reset_read_span("geral");
    store.observe_chat_viewport("geral", &ids(9..=10), true);
    assert_eq!(
        store.hourglass_range("geral"),
        Some(("m2".into(), "m8".into()))
    );
    store.mark_read("geral");
    assert_eq!(store.hourglass_range("geral"), None);
}

#[test]
fn catch_up_can_be_consumed_from_both_ends_after_channel_acknowledgement() {
    let mut store = navigation_store(10);
    store.observe_chat_viewport("geral", &ids(9..=10), true);
    store.observe_chat_viewport("geral", &ids(7..=8), false);
    store.reset_read_span("geral");
    store.observe_chat_viewport("geral", &ids(2..=4), false);
    assert_eq!(
        store.hourglass_range("geral"),
        Some(("m5".into(), "m8".into()))
    );
    store.observe_chat_viewport("geral", &ids(5..=8), false);
    assert_eq!(store.hourglass_range("geral"), None);
    assert!(!store.channel("geral").unwrap().unread);
}

#[test]
fn a_new_arrival_behind_the_viewport_is_unread_even_with_an_old_hourglass() {
    let mut store = navigation_store(10);
    store.observe_chat_viewport("geral", &ids(9..=10), true);
    store.reset_read_span("geral");
    store.observe_chat_viewport("geral", &ids(2..=3), false);
    let message = navigation_message("m11", 11);
    store.apply_live_message_effects(&message, true);
    store.messages.push(message);
    assert!(store.channel("geral").unwrap().unread);
    assert_eq!(store.first_badge_unread("geral").as_deref(), Some("m11"));
    store.observe_chat_viewport("geral", &ids(3..=4), false);
    assert!(store.channel("geral").unwrap().unread);
    store.reset_read_span("geral");
    store.observe_chat_viewport("geral", &ids(10..=11), true);
    assert!(!store.channel("geral").unwrap().unread);
    assert!(store.hourglass_range("geral").is_some());
}

#[test]
fn reaching_a_history_window_end_is_not_reaching_the_live_head() {
    let mut store = navigation_store(10);
    let ticket = store.begin_message_context("geral");
    let messages = (2..=4)
        .map(|n| {
            serde_json::from_value(serde_json::json!({
                "id":format!("m{n}"),"channel_id":"geral","author_id":"outro","content":"x",
                "created_at":store.message(&format!("m{n}")).unwrap().at.to_rfc3339()
            }))
            .unwrap()
        })
        .collect();
    store.apply(Update::MessageContext {
        ticket,
        message_id: "m3".into(),
        context: models::MessageContext {
            channel_id: "geral".into(),
            messages,
            has_more_before: true,
            has_more_after: true,
        },
    });
    store.observe_chat_viewport("geral", &ids(3..=4), true);
    assert_ne!(
        store.read_states["geral"].acknowledged_id.as_deref(),
        Some("m10")
    );
    assert!(store.channel("geral").unwrap().unread);
}

#[test]
fn skipping_a_second_new_block_extends_the_single_preserved_hourglass() {
    let mut store = navigation_store(10);
    store.observe_chat_viewport("geral", &ids(9..=10), true);
    store.observe_chat_viewport("geral", &ids(7..=8), false);
    assert_eq!(
        store.hourglass_range("geral"),
        Some(("m2".into(), "m8".into()))
    );
    for i in 11..=30 {
        store.messages.push(navigation_message(&format!("m{i}"), i));
    }
    // First down-arrow goes to the oldest arrival, not to the live head.
    store.reset_read_span("geral");
    store.observe_chat_viewport("geral", &ids(11..=13), false);
    assert!(store.channel("geral").unwrap().unread);
    // Second down-arrow skips the rest. Keep one conservative interval,
    // retaining the older block rather than silently throwing it away.
    store.reset_read_span("geral");
    store.observe_chat_viewport("geral", &ids(28..=30), true);
    assert!(!store.channel("geral").unwrap().unread);
    assert_eq!(
        store.hourglass_range("geral"),
        Some(("m2".into(), "m30".into()))
    );
}

#[test]
fn skipping_new_arrivals_after_first_new_jump_creates_a_new_hourglass() {
    let mut store = navigation_store(10);
    store.mark_read("geral");
    for i in 11..=30 {
        store.messages.push(navigation_message(&format!("m{i}"), i));
    }
    store.reset_read_span("geral");
    store.observe_chat_viewport("geral", &ids(11..=13), false);
    assert_eq!(frontier_id(&store).as_deref(), Some("m13"));
    store.reset_read_span("geral");
    store.observe_chat_viewport("geral", &ids(28..=30), true);
    assert_eq!(store.hourglass_target("geral").as_deref(), Some("m14"));
    assert!(!store.channel("geral").unwrap().unread);
}

#[test]
fn reading_a_context_page_does_not_consume_the_unloaded_middle_of_catch_up() {
    let mut store = navigation_store(10);
    store.observe_chat_viewport("geral", &ids(9..=10), true);
    store.reset_read_span("geral");
    let ticket = store.begin_message_context("geral");
    let messages = (1..=4)
        .map(|n| {
            serde_json::from_value(serde_json::json!({
                "id":format!("m{n}"),"channel_id":"geral","author_id":"outro","content":"x",
                "created_at":store.message(&format!("m{n}")).unwrap().at.to_rfc3339()
            }))
            .unwrap()
        })
        .collect();
    store.apply(Update::MessageContext {
        ticket,
        message_id: "m2".into(),
        context: models::MessageContext {
            channel_id: "geral".into(),
            messages,
            has_more_before: false,
            has_more_after: true,
        },
    });
    store.observe_chat_viewport("geral", &ids(2..=4), false);
    assert_eq!(frontier_id(&store).as_deref(), Some("m4"));
    assert_eq!(
        store.hourglass_range("geral"),
        Some(("m5".into(), "m10".into()))
    );
    assert!(!store.channel("geral").unwrap().unread);
}

#[test]
fn notification_read_queue_survives_failure_and_only_clears_the_confirmed_owner_ids() {
    let mut store = navigation_store(10);
    store.queue_notification_reads("geral", vec!["n1".into(), "n2".into(), "n1".into()]);
    assert_eq!(store.pending_notification_reads(), ["n1", "n2"]);
    store.apply(Update::NotificationReadsSynced {
        owner_user_id: "wrong-owner".into(),
        ids: vec!["n1".into()],
        success: true,
    });
    assert_eq!(store.pending_notification_reads().len(), 2);
    store.apply(Update::NotificationReadsSynced {
        owner_user_id: store.me.clone(),
        ids: vec!["n1".into()],
        success: true,
    });
    assert_eq!(store.pending_notification_reads(), ["n2"]);
    let state = store.read_states["geral"].to_cached(&store.me, "geral");
    assert_eq!(
        ChannelReadState::from_cached(state).pending_notification_reads,
        ["n2"]
    );
}

#[test]
fn read_frontiers_retain_submillisecond_order_across_cache_round_trip() {
    let at: DateTime<Utc> = "2026-10-21T12:00:00.123456789Z".parse().unwrap();
    let state = ChannelReadState {
        read_at: Some(at),
        read_message_id: Some("m1".into()),
        seen_to_at: Some(at),
        seen_to_id: Some("m2".into()),
        ..Default::default()
    };
    assert_eq!(
        ChannelReadState::from_cached(state.to_cached("me", "geral")),
        state
    );
}

#[test]
fn channel_receipts_require_owner_and_exact_target_acknowledgement() {
    let mut store = navigation_store(10);
    store.observe_chat_viewport("geral", &ids(9..=10), true);
    store.apply(Update::ChannelReadSynced {
        owner_user_id: "other".into(),
        channel_id: "geral".into(),
        message_id: "m10".into(),
        success: true,
    });
    assert_eq!(store.pending_read_receipts().len(), 1);
    store.apply(Update::ChannelReadSynced {
        owner_user_id: store.me.clone(),
        channel_id: "geral".into(),
        message_id: "m9".into(),
        success: true,
    });
    assert_eq!(store.pending_read_receipts().len(), 1);
    store.apply(Update::ChannelReadSynced {
        owner_user_id: store.me.clone(),
        channel_id: "geral".into(),
        message_id: "m10".into(),
        success: false,
    });
    assert_eq!(store.pending_read_receipts().len(), 1);
    store.apply(Update::ChannelReadSynced {
        owner_user_id: store.me.clone(),
        channel_id: "geral".into(),
        message_id: "m10".into(),
        success: true,
    });
    assert!(store.pending_read_receipts().is_empty());
    assert!(store.hourglass_range("geral").is_some());
}

#[test]
fn remote_channel_read_clears_the_badge_but_preserves_local_catch_up() {
    let mut store = navigation_store(10);
    store.observe_chat_viewport("geral", &ids(7..=8), false);
    let key = store.message_key("m10").unwrap();
    store.merge_remote_read("geral", "m10", key);
    assert!(!store.channel("geral").unwrap().unread);
    assert!(store.hourglass_range("geral").is_some());
    assert!(store.pending_read_receipts().is_empty());
}

#[test]
fn mention_bell_walks_newest_to_oldest_and_excludes_plain_notifications() {
    let mut store = navigation_store(10);
    let me = store.me.clone();
    for id in ["m3", "m7"] {
        store
            .messages
            .iter_mut()
            .find(|m| m.id == id)
            .unwrap()
            .content = format!("hello @mention(<@{me}>)");
    }
    store.read_states.get_mut("geral").unwrap().unread_mentions =
        VecDeque::from(["m3".into(), "m5".into(), "m7".into()]);
    assert_eq!(store.unread_mention_ids("geral"), ["m7", "m3"]);
    store.observe_chat_viewport("geral", &["m7".into()], false);
    assert_eq!(store.unread_mention_ids("geral"), ["m3"]);
    store.observe_chat_viewport("geral", &["m10".into()], true);
    assert!(store.unread_mention_ids("geral").is_empty());
}

#[test]
fn catch_up_progress_and_acknowledged_head_restore_independently_after_restart() {
    let mut store = navigation_store(30);
    store.observe_chat_viewport("geral", &ids(28..=30), true);
    store.reset_read_span("geral");
    store.observe_chat_viewport("geral", &ids(2..=5), false);
    let expected = store.hourglass_range("geral");
    let read = store
        .take_cache_ops()
        .into_iter()
        .find_map(|op| match op {
            CacheOp::UpsertReadState(read) => Some(read),
            _ => None,
        })
        .unwrap();
    let mut reopened = Store::default();
    reopened.restore_cached_metadata(CachedServerMetadata {
        owner_user_id: Some(store.me.clone()),
        channels: store.channels.iter().map(CachedChannel::from).collect(),
        read_states: vec![read],
        ..Default::default()
    });
    reopened.me = store.me.clone();
    reopened.messages = store.messages.clone();
    reopened.observe_chat_viewport("geral", &ids(2..=5), false);
    assert_eq!(reopened.hourglass_range("geral"), expected);
    assert!(!reopened.channel("geral").unwrap().unread);
    assert_eq!(
        reopened.read_states["geral"].acknowledged_id.as_deref(),
        Some("m30")
    );
}

#[test]
fn server_read_confirmation_clears_cached_mentions_without_an_open_notification_entry() {
    let mut store = navigation_store(10);
    store
        .read_states
        .get_mut("geral")
        .unwrap()
        .unread_mentions
        .push_back("m3".into());
    store.apply(Update::Notifications(vec![models::Notification {
        id: "n3".into(),
        message_id: Some("m3".into()),
        channel_id: Some("geral".into()),
        author_id: Some("outro".into()),
        message_content: None,
        read: true,
        created_at: None,
    }]));
    assert!(store.read_states["geral"].unread_mentions.is_empty());
    assert_eq!(store.channel("geral").unwrap().mentions, 0);
}

#[test]
fn late_and_pending_notifications_cannot_resurrect_mentions_after_reading() {
    let mut store = navigation_store(10);
    let me = store.me.clone();
    let target = store.messages.iter_mut().find(|m| m.id == "m3").unwrap();
    target.content = format!("hello @mention(<@{me}>)");
    let at = target.at.with_timezone(&Utc);
    store.observe_chat_viewport("geral", &ids(9..=10), true);
    let notification = models::Notification {
        id: "n3".into(),
        message_id: Some("m3".into()),
        channel_id: Some("geral".into()),
        author_id: Some("outro".into()),
        message_content: Some(format!("@mention(<@{me}>)")),
        read: false,
        created_at: Some(at),
    };
    store.apply(Update::Notifications(vec![notification.clone()]));
    assert_eq!(store.pending_notification_reads(), ["n3"]);
    assert!(store.unread_mention_ids("geral").is_empty());
    assert!(!store.channel("geral").unwrap().unread);
    store.counted_notifications.clear(); // equivalent to a cache restore
    store.apply(Update::Notifications(vec![notification]));
    assert!(store.unread_mention_ids("geral").is_empty());
}

#[test]
fn unloaded_mentions_keep_chronological_order_across_cache_restore() {
    let mut store = navigation_store(10);
    let base = store.messages[0].at.with_timezone(&Utc);
    for (id, seconds) in [("z-old", 1), ("a-new", 9)] {
        let state = store.read_states.get_mut("geral").unwrap();
        state.unread_mentions.push_back(id.into());
        state
            .mention_times
            .insert(id.into(), base + chrono::Duration::seconds(seconds));
    }
    assert_eq!(store.unread_mention_ids("geral"), ["a-new", "z-old"]);
    let restored =
        ChannelReadState::from_cached(store.read_states["geral"].to_cached(&store.me, "geral"));
    store.read_states.insert("geral".into(), restored);
    assert_eq!(store.unread_mention_ids("geral"), ["a-new", "z-old"]);
}

#[test]
fn signed_out_read_watermarks_never_leak_into_another_account() {
    let mut store = navigation_store(10);
    store.observe_chat_viewport("geral", &ids(9..=10), true);
    store.apply(Update::Session(None));
    assert!(store.read_marks.is_empty());
    assert!(store.read_states.is_empty());
    assert!(store.pending_read_receipts().is_empty());
}

#[test]
fn hourglass_button_must_target_the_same_boundary_as_its_visible_marker() {
    let mut store = navigation_store(30);
    store.observe_chat_viewport("geral", &ids(28..=30), true);
    store.observe_chat_viewport("geral", &ids(25..=27), false);
    // The retained head no longer contains the persisted reading anchor.
    let retained_start = store.message("m20").unwrap().at;
    store
        .messages
        .retain(|message| message.at >= retained_start);
    for i in 31..=32 {
        store.messages.push(navigation_message(&format!("m{i}"), i));
    }
    // Both new arrivals fit in the destination viewport; no new rows are skipped.
    store.reset_read_span("geral");
    store.observe_chat_viewport("geral", &ids(29..=32), true);
    let marker = store.hourglass_range("geral").unwrap().0;
    assert_eq!(marker, "m20");
    assert_eq!(store.hourglass_navigation_target("geral"), Some(marker));
}

#[test]
fn legacy_millisecond_read_cursor_does_not_reopen_the_already_read_anchor() {
    let mut store = navigation_store(30);
    store.mark_read("geral");
    let at = store.messages.last().unwrap().at.with_timezone(&Utc)
        + chrono::Duration::nanoseconds(123_456);
    store.messages.last_mut().unwrap().at = at.with_timezone(&Local);
    let state = store.read_states.get_mut("geral").unwrap();
    state.read_at = DateTime::from_timestamp_millis(at.timestamp_millis());
    state.seen_to_at = state.read_at;
    for i in 31..=32 {
        store.messages.push(navigation_message(&format!("m{i}"), i));
    }
    store.reset_read_span("geral");
    store.observe_chat_viewport("geral", &ids(31..=32), true);
    assert_eq!(
        store.hourglass_target("geral"),
        None,
        "showing both new messages must not resurrect the old read cursor as unread"
    );
}

#[test]
fn deleting_the_read_anchor_does_not_prevent_reading_a_contiguous_loaded_prefix() {
    let mut store = navigation_store(30);
    store.observe_chat_viewport("geral", &ids(28..=30), true);
    store.messages.retain(|message| message.id != "m1");
    store.messages.insert(0, navigation_message("m0", 0));
    store.history_has_more.insert("geral".into(), true);
    store.reset_read_span("geral");
    store.observe_chat_viewport("geral", &ids(2..=5), false);
    assert_eq!(frontier_id(&store).as_deref(), Some("m5"));
    assert_eq!(store.hourglass_target("geral").as_deref(), Some("m6"));
}
