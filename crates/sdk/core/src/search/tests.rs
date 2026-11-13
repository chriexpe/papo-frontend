//! Grammar and exact tuple checks without HTTP or time-dependent fixtures.
use super::*;
#[test]
fn quoted_names_resolve_filters_and_ambiguous_labels_fail() {
    let mut resources = crate::ResourceSnapshot::empty(crate::Generation(1));
    resources.members = vec![crate::UserSummary {
        id: Some(Uuid::from_u128(1).to_string()),
        nickname: Some("Quoted Person".into()),
        ..Default::default()
    }]
    .into();
    resources.channels = vec![crate::Channel {
        id: Some(Uuid::from_u128(2).to_string()),
        name: Some("Quoted Channel".into()),
        ..Default::default()
    }]
    .into();
    let names = SearchNames {
        resources: &resources,
    };
    let input = concat!(
        "from:\"@Quoted Person\" in:\"#Quoted Channel\" tem:anexo has:link ",
        "mentions:00000000-0000-0000-0000-000000000001 words"
    );
    let query = SearchQuery::parse(input, &names)
        .unwrap()
        .into_request()
        .unwrap();
    assert_eq!(query.author, Some(Uuid::from_u128(1).to_string()));
    assert_eq!(query.channel_id, Some(Uuid::from_u128(2).to_string()));
    assert_eq!(query.mention, query.author);
    assert_eq!(query.contains_attachment, Some(true));
    assert_eq!(query.has.as_deref(), Some("link"));
    assert_eq!(query.text.as_deref(), Some("words"));
    Arc::make_mut(&mut resources.members).push(crate::UserSummary {
        id: Some(Uuid::from_u128(3).to_string()),
        nickname: Some("Quoted Person".into()),
        ..Default::default()
    });
    assert!(
        SearchQuery::parse(
            "de:\"Quoted Person\"",
            &SearchNames {
                resources: &resources
            }
        )
        .is_err()
    );
}
#[test]
fn global_search_has_no_implicit_dm_and_false_attachment_filter_is_retained() {
    let query = SearchQuery {
        contains_attachment: Some(false),
        ..Default::default()
    }
    .into_request()
    .unwrap();
    assert!(query.channel_id.is_none());
    assert_eq!(query.contains_attachment, Some(false));
    assert!(
        SearchQuery {
            date_start: Some("2026-02-30".into()),
            ..Default::default()
        }
        .into_request()
        .is_err()
    );
    assert!(
        SearchQuery {
            date_start: Some("2026-02-02".into()),
            date_end: Some("2026-02-01".into()),
            ..Default::default()
        }
        .into_request()
        .is_err()
    );
}
#[test]
fn exact_tuple_orders_equivalent_offsets_and_preserves_wire_strings() {
    let a = SearchCursor {
        created_at: "2026-01-01T01:00:00.123456+01:00".into(),
        id: Uuid::from_u128(1).to_string(),
    };
    let b = SearchCursor {
        created_at: "2026-01-01T00:00:00.123456Z".into(),
        id: Uuid::from_u128(2).to_string(),
    };
    assert!(cursor_key(&a).unwrap() < cursor_key(&b).unwrap());
    assert_eq!(a.created_at, "2026-01-01T01:00:00.123456+01:00");
}
