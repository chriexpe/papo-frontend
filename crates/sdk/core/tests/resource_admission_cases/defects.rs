//! One table of malformed backend replies, each expressed for every producer that fetches
//! that input. Only reconnect pages the roster, so `has_more` and partial-roster defects
//! have no gateway or operation form.

use super::{Account, CHANNEL};
use papo_api::{
    Uuid,
    gateway::{ChannelUpdateEvent, DMUpdateEvent, KnownEvent, RoleAddEvent},
    wire::{DirectConversation, UserSummary},
};
use papo_sdk::{CacheResource, operations::MetadataScope};
use serde_json::{Value, json};

/// One scripted reply, consumed by the next matching request.
pub(super) struct Reply {
    pub method: &'static str,
    pub path: &'static str,
    pub body: Value,
}

pub(super) struct Defect {
    pub name: &'static str,
    pub group: CacheResource,
    pub reconnect: Vec<Reply>,
    /// The event that makes the gateway refetch, and the replies it then receives.
    pub gateway: Option<(KnownEvent, Vec<Reply>)>,
    pub operation: Option<(MetadataScope, Vec<Reply>)>,
}

fn get(path: &'static str, body: Value) -> Reply {
    Reply {
        method: "GET",
        path,
        body,
    }
}

fn summaries(body: Value) -> Reply {
    Reply {
        method: "POST",
        path: "/users/user_summary_batch",
        body,
    }
}

fn channel(id: &str, overwrites: &Value) -> Value {
    json!({"id": id, "name": "history", "type": "text", "position": 0, "permissions": overwrites})
}

fn channel_event() -> KnownEvent {
    KnownEvent::ChannelUpdate(Box::new(ChannelUpdateEvent {
        channel_id: CHANNEL.into(),
        name: "history".into(),
        ..ChannelUpdateEvent::default()
    }))
}

fn role_event(own: Uuid) -> KnownEvent {
    KnownEvent::RoleAdd(Box::new(RoleAddEvent {
        user_id: own.to_string(),
        role_id: Uuid::from_u128(9).to_string(),
    }))
}

fn dm_event() -> KnownEvent {
    KnownEvent::DMUpdate(Box::new(DMUpdateEvent {
        dm: DirectConversation {
            id: Uuid::from_u128(31).to_string(),
            user: UserSummary {
                id: Some(Uuid::from_u128(32).to_string()),
                ..UserSummary::default()
            },
            ..DirectConversation::default()
        },
    }))
}

/// A channel-list defect, which all three producers fetch the same way.
fn channels(name: &'static str, body: Value) -> Defect {
    Defect {
        name,
        group: CacheResource::Channels,
        reconnect: vec![get("/channels", body.clone())],
        gateway: Some((channel_event(), vec![get("/channels", body.clone())])),
        operation: Some((MetadataScope::Channels, vec![get("/channels", body)])),
    }
}

/// The gateway form first admits a changed own summary, then rejects the roles; nothing
/// of the changed summary may be published.
fn roles(name: &'static str, own: Uuid, renamed: &Value, body: Value) -> Defect {
    let gateway = vec![summaries(json!([renamed])), get("/roles", body.clone())];
    Defect {
        name,
        group: CacheResource::Roles,
        reconnect: vec![get("/roles", body.clone())],
        gateway: Some((role_event(own), gateway)),
        operation: Some((MetadataScope::Roles, vec![get("/roles", body)])),
    }
}

/// A server defect; no gateway event refetches the server.
fn server(name: &'static str, body: Value) -> Defect {
    Defect {
        name,
        group: CacheResource::Server,
        reconnect: vec![get("/server", body.clone())],
        gateway: None,
        operation: Some((MetadataScope::Server, vec![get("/server", body)])),
    }
}

/// A DM or block defect: reconnect, `dm_update` and the social refresh fetch both lists.
fn social(name: &'static str, group: CacheResource, path: &'static str, body: Value) -> Defect {
    Defect {
        name,
        group,
        reconnect: vec![get(path, body.clone())],
        gateway: Some((dm_event(), vec![get(path, body.clone())])),
        operation: Some((MetadataScope::Social, vec![get(path, body)])),
    }
}

/// An own-row defect: the roster page on reconnect, the summary batch elsewhere.
fn own_rows(name: &'static str, own: Uuid, rows: Value) -> Defect {
    let page = json!({"users": rows.clone(), "has_more": false});
    Defect {
        name,
        group: CacheResource::Members,
        reconnect: vec![get("/users", page)],
        gateway: Some((role_event(own), vec![summaries(rows.clone())])),
        operation: Some((MetadataScope::Member(own), vec![summaries(rows)])),
    }
}

fn roster(name: &'static str, page: Value) -> Defect {
    Defect {
        name,
        group: CacheResource::Members,
        reconnect: vec![get("/users", page)],
        gateway: None,
        operation: None,
    }
}

pub(super) fn all(account: &Account<'_>) -> Vec<Defect> {
    let mut defects = channel_defects();
    defects.extend(people_defects(account));
    defects.extend(social_defects());
    defects
}

fn channel_defects() -> Vec<Defect> {
    let restrictive_bad_role = json!([{"role_id": "", "permissions": {"read_channel": false}}]);
    let unhyphenated_channel = CHANNEL.replace('-', "");
    // The backend refuses to create a 501st channel, so a longer list cannot be genuine.
    let over: Vec<_> = (0..501_u128)
        .map(|index| channel(&Uuid::from_u128(0x5000 + index).to_string(), &json!([])))
        .collect();
    let typed = |kind: &str| {
        let mut row = channel(CHANNEL, &json!([]));
        row["type"] = json!(kind);
        json!({"channels": [row]})
    };
    vec![
        channels("channels absent", json!({})),
        channels("channels null", json!({"channels": null})),
        channels(
            "nested overwrite with an empty role_id",
            json!({"channels": [channel(CHANNEL, &restrictive_bad_role)]}),
        ),
        channels(
            "duplicate channel IDs",
            json!({"channels": [channel(CHANNEL, &json!([])), channel(CHANNEL, &json!([]))]}),
        ),
        channels(
            "empty-string channel ID",
            json!({"channels": [channel("", &json!([]))]}),
        ),
        channels(
            "non-canonical channel ID",
            json!({"channels": [channel(&unhyphenated_channel, &json!([]))]}),
        ),
        channels(
            "more channels than the backend's 500-channel maximum",
            json!({"channels": over}),
        ),
        channels("unsupported channel type", typed("unexpected_type")),
        channels("DM type in the channel list", typed("dm")),
        server(
            "server ID empty",
            json!({"id": "", "name": "fixture", "owner_id": null}),
        ),
        server(
            "owner_id not a UUID",
            json!({"id": Uuid::from_u128(1).to_string(), "owner_id": "owner"}),
        ),
    ]
}

/// Roles, own-row and roster defects, built from the account's current own row.
fn people_defects(account: &Account<'_>) -> Vec<Defect> {
    let own = account.own();
    let snapshot = account.snapshot();
    let own_row = snapshot
        .members
        .iter()
        .find(|member| member.id.as_deref() == Some(own.to_string().as_str()))
        .map(serde_json::to_value)
        .expect("signed-in roster holds the account")
        .expect("summary serializes");
    let with_roles = |roles: Value| {
        let mut row = own_row.clone();
        row["roles"] = roles;
        row
    };
    let mut renamed = with_roles(json!([]));
    renamed["nickname"] = json!("renamed-before-rejection");
    vec![
        roles("roles absent", own, &renamed, json!({})),
        roles("roles null", own, &renamed, json!({"roles": null})),
        own_rows(
            "own role assignments null",
            own,
            json!([with_roles(Value::Null)]),
        ),
        own_rows(
            "own role with an empty-string ID",
            own,
            json!([with_roles(json!([{"id": ""}]))]),
        ),
        own_rows(
            "duplicate own rows",
            own,
            json!([with_roles(json!([])), with_roles(json!([]))]),
        ),
        roster("has_more absent", json!({"users": [with_roles(json!([]))]})),
        roster("users null", json!({"users": null, "has_more": false})),
        roster(
            "partial roster: more pages without a boundary",
            json!({"users": [], "has_more": true}),
        ),
        roster(
            "partial roster: complete page without the account",
            json!({"users": [], "has_more": false}),
        ),
        Defect {
            name: "roster cursor moving backwards",
            group: CacheResource::Members,
            reconnect: vec![
                get(
                    "/users",
                    json!({"users": [with_roles(json!([]))], "has_more": true}),
                ),
                get(
                    "/users",
                    json!({"users": [{"id": Uuid::from_u128(34).to_string(), "roles": [],
                        "created_at": "2025-01-01T00:00:00Z"}], "has_more": true}),
                ),
            ],
            gateway: None,
            operation: None,
        },
    ]
}

fn social_defects() -> Vec<Defect> {
    let dm = json!({"id": Uuid::from_u128(31).to_string(), "created_at": "2026-01-01T00:00:00Z",
        "unread_count": 0, "user": {"id": Uuid::from_u128(32).to_string()}});
    let mut peerless = dm.clone();
    peerless["user"] = json!({});
    let blocked = json!({"id": Uuid::from_u128(33).to_string()});
    vec![
        social(
            "duplicate DM IDs",
            CacheResource::Dms,
            "/dms",
            json!({"dms": [dm.clone(), dm]}),
        ),
        social(
            "DM without a peer",
            CacheResource::Dms,
            "/dms",
            json!({"dms": [peerless]}),
        ),
        social(
            "duplicate block IDs",
            CacheResource::Blocks,
            "/users/blocks",
            json!({"users": [blocked.clone(), blocked]}),
        ),
        social(
            "empty-string block ID",
            CacheResource::Blocks,
            "/users/blocks",
            json!({"users": [{"id": ""}]}),
        ),
    ]
}

/// Queues every reply of one producer's form of a defect.
pub(super) async fn script(account: &Account<'_>, replies: Vec<Reply>) -> super::Result {
    for reply in replies {
        account.script(reply.method, reply.path, reply.body).await?;
    }
    Ok(())
}
