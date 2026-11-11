//! Owns pure channel summary ordering/null/refresh tests; leaves TCP serving to integration tests.

use super::*;
use crate::{HistorySeed, history::uuid};
use papo_api::wire::UserProfile;

const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
const AUTHOR: &str = "30000000-0000-4000-8000-000000000001";
const FIRST: &str = "20000000-0000-4000-8000-000000000001";
const SECOND: &str = "20000000-0000-4000-8000-000000000002";

fn fixture() -> (Resources, History) {
    crate::history_seed::prepare(&[HistorySeed {
        channel_id: CHANNEL.into(),
        author_id: AUTHOR.into(),
        first_message_id: FIRST.into(),
        count: 2,
        first_created_at: "2026-01-01T00:00:00.123456Z".into(),
        timestamp_step_us: 0,
        equal_timestamp_count: 2,
    }])
    .unwrap()
}

fn head(resources: &Resources, history: &History) -> Value {
    list(resources, history, "reader").body.unwrap()["channels"][0]["last_message"].clone()
}

#[test]
fn latest_summary_has_only_pinned_fields_and_explicit_nulls() {
    let (resources, history) = fixture();
    assert_eq!(
        head(&resources, &history),
        json!({
            "id": SECOND, "content": null, "author_id": AUTHOR,
            "author_username": null, "created_at": "2026-01-01T00:00:00.123456Z",
        })
    );
}

#[test]
fn later_timestamp_beats_uuid_and_deleted_head_falls_back_then_clears() {
    let (resources, mut history) = fixture();
    let later_id = "00000000-0000-4000-8000-000000000001";
    let mut later = history.message(uuid(FIRST).unwrap()).unwrap().clone();
    later.id = Some(later_id.into());
    later.created_at = Some("2026-01-01T00:00:00.123457Z".into());
    history.insert(later).unwrap();
    assert_eq!(head(&resources, &history)["id"], later_id);
    history.remove(uuid(later_id).unwrap());
    assert_eq!(head(&resources, &history)["id"], SECOND);
    history.remove(uuid(SECOND).unwrap());
    history.remove(uuid(FIRST).unwrap());
    assert_eq!(head(&resources, &history), Value::Null);
}

#[test]
fn summary_uses_current_author_profile_and_updated_payload() {
    let (mut resources, mut history) = fixture();
    resources.profiles.insert(
        AUTHOR.into(),
        UserProfile {
            username: Some("updated-author".into()),
            ..UserProfile::default()
        },
    );
    let message = history.message_mut(uuid(SECOND).unwrap()).unwrap();
    message.content = Some(String::new());
    let value = head(&resources, &history);
    assert_eq!(value["author_username"], "updated-author");
    assert_eq!(value["content"], "");
    history
        .message_mut(uuid(SECOND).unwrap())
        .unwrap()
        .author_id = None;
    let value = head(&resources, &history);
    assert_eq!(value["author_id"], Value::Null);
    assert_eq!(value["author_username"], Value::Null);
}

#[test]
fn last_message_projection_preserves_account_scoped_read_marks() {
    let (mut resources, history) = fixture();
    resources.read_marks.insert(
        ("reader".into(), CHANNEL.into()),
        (FIRST.into(), "2026-02-01T00:00:00Z".into()),
    );
    let owner = list(&resources, &history, "reader").body.unwrap();
    let other = list(&resources, &history, "other").body.unwrap();
    assert_eq!(owner["channels"][0]["last_read_message"], FIRST);
    assert_eq!(owner["channels"][0]["last_read_at"], "2026-02-01T00:00:00Z");
    assert_eq!(other["channels"][0]["last_read_message"], Value::Null);
    assert_eq!(
        owner["channels"][0]["last_message"],
        other["channels"][0]["last_message"]
    );
}
