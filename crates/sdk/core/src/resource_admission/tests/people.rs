//! Roster, summary-batch, profile, DM and block admission; shared fixtures live in the parent.

use super::{
    Admission, AdmissionFault, CacheResource, Diagnostics, OWN, Rejected, Roster, RosterPage, Uuid,
    id, member, page, rejected, roster,
};
use papo_api::wire::{
    BlockListResponse, DirectConversation, DirectConversationList, UserProfile, UserSummary,
};

#[test]
fn a_member_page_without_users_or_has_more_proves_nothing() {
    let log = Diagnostics::default();
    let admission = Admission::for_test(Uuid::from_u128(OWN), &log);
    let cases = [
        page(None, Some(false)),
        page(Some(vec![member(OWN, &[])]), None),
    ];
    for case in cases {
        let error = Roster::new(admission).page(case).rejected();
        assert_eq!(
            error,
            rejected(CacheResource::Members, AdmissionFault::MissingList)
        );
    }
}

#[test]
fn a_partial_roster_is_never_admitted() {
    let log = Diagnostics::default();
    let admission = Admission::for_test(Uuid::from_u128(OWN), &log);
    // "More" with no boundary row, a boundary without an ID, and a repeated cursor.
    let mut nameless = member(7, &[]);
    nameless.id = None;
    for case in [
        page(Some(Vec::new()), Some(true)),
        page(Some(vec![nameless]), Some(true)),
    ] {
        let error = Roster::new(admission).page(case).rejected();
        assert_eq!(
            error,
            rejected(CacheResource::Members, AdmissionFault::Incomplete)
        );
    }
    let mut repeating = Roster::new(admission);
    let first = page(Some(vec![member(OWN, &[])]), Some(true));
    assert!(matches!(
        repeating.page(first.clone()),
        Ok(RosterPage::More(_))
    ));
    assert_eq!(
        repeating.page(first).rejected(),
        rejected(CacheResource::Members, AdmissionFault::Incomplete)
    );
    // A complete roster that lost the account's own row was truncated somewhere.
    assert_eq!(
        roster(&log, vec![member(7, &[])]).rejected(),
        rejected(CacheResource::Members, AdmissionFault::MissingOwnRow)
    );
}

#[test]
fn the_own_member_row_is_security_bearing() {
    let log = Diagnostics::default();
    let mut without_roles = member(OWN, &[]);
    without_roles.roles = None;
    let cases = [
        (vec![without_roles], AdmissionFault::MissingField),
        (
            vec![member(OWN, &[Some(String::new())])],
            AdmissionFault::InvalidId,
        ),
        (
            vec![member(OWN, &[id(9), id(9)])],
            AdmissionFault::DuplicateId,
        ),
        (
            vec![member(OWN, &[]), member(OWN, &[])],
            AdmissionFault::DuplicateId,
        ),
    ];
    for (users, fault) in cases {
        assert_eq!(
            roster(&log, users).rejected(),
            rejected(CacheResource::Members, fault)
        );
    }
    assert_eq!(log.snapshot().dropped_rows, 0);
}

#[test]
fn other_members_malformed_rows_and_labels_are_dropped_with_a_diagnostic() {
    let log = Diagnostics::default();
    let mut blank = member(7, &[]);
    blank.id = Some(String::new());
    let users = vec![
        member(OWN, &[id(9)]),
        blank,
        member(8, &[]),
        member(8, &[]),
        member(10, &[id(9), Some("label".into()), id(11), id(11)]),
        member(12, &[]),
    ];
    let admitted = roster(&log, users).unwrap();
    let ids: Vec<_> = admitted.iter().map(|user| user.id.clone()).collect();
    assert_eq!(ids, vec![id(OWN), id(10), id(12)]);
    let labels: Vec<_> = admitted[1]
        .roles
        .iter()
        .flatten()
        .map(|role| role.id.clone())
        .collect();
    assert_eq!(labels, vec![id(9)]);
    let diagnostics = log.snapshot();
    // One blank row, both copies of the repeated row, one blank and two repeated labels.
    assert_eq!(diagnostics.dropped_rows, 6);
    assert_eq!(diagnostics.rejected_replacements, 0);
    assert_eq!(
        diagnostics.last_dropped.map(|row| row.group),
        Some(CacheResource::Members)
    );
}

#[test]
fn summary_batches_reject_own_row_faults_but_drop_other_malformed_rows() {
    let log = Diagnostics::default();
    let admission = Admission::for_test(Uuid::from_u128(OWN), &log);
    let requested = [
        Uuid::from_u128(7),
        Uuid::from_u128(0xab),
        Uuid::from_u128(OWN),
    ];
    // The signed-in account exists, so its requested row may not be omitted.
    let own_cases = [
        (Vec::new(), AdmissionFault::MissingOwnRow),
        (
            vec![member(OWN, &[]), member(OWN, &[])],
            AdmissionFault::DuplicateId,
        ),
        (
            vec![member(OWN, &[Some(String::new())])],
            AdmissionFault::InvalidId,
        ),
    ];
    for (response, fault) in own_cases {
        assert_eq!(
            admission.summaries(&requested, response).rejected(),
            rejected(CacheResource::Members, fault)
        );
    }
    assert_eq!(
        admission
            .summaries(&requested[..2], vec![member(OWN, &[])])
            .rejected(),
        rejected(CacheResource::Members, AdmissionFault::UnexpectedId)
    );
    // An omitted other user does not exist; that is not a fault.
    assert!(
        admission
            .summaries(&requested[..2], Vec::new())
            .unwrap()
            .is_empty()
    );
    assert_eq!(log.snapshot().dropped_rows, 0);
    let mut uppercase = member(0xab, &[]);
    uppercase.id = uppercase.id.map(|value| value.to_uppercase());
    let response = vec![
        member(OWN, &[]),
        member(8, &[]),
        member(7, &[]),
        member(7, &[]),
        uppercase,
    ];
    let admitted = admission.summaries(&requested, response).unwrap();
    let ids: Vec<_> = admitted.iter().map(|user| user.id.clone()).collect();
    assert_eq!(ids, vec![id(OWN)]);
    // The unrequested row, both repeated rows and the non-canonical row.
    assert_eq!(log.snapshot().dropped_rows, 4);
    assert_eq!(
        admission.event_user("not-a-user").rejected(),
        rejected(CacheResource::Members, AdmissionFault::InvalidId)
    );
}

#[test]
fn a_requested_profile_must_name_the_requested_user_canonically() {
    let log = Diagnostics::default();
    let admission = Admission::for_test(Uuid::from_u128(OWN), &log);
    let profile = |value: Option<String>| UserProfile {
        id: value,
        ..UserProfile::default()
    };
    let requested = Uuid::from_u128(0xab);
    assert!(admission.profile(requested, profile(id(0xab))).is_ok());
    for wrong in [id(7), id(0xab).map(|value| value.to_uppercase()), None] {
        assert_eq!(
            admission.profile(requested, profile(wrong)).rejected(),
            rejected(CacheResource::Profiles, AdmissionFault::UnexpectedId)
        );
    }
}

#[test]
fn dm_and_block_identities_are_security_bearing() {
    let log = Diagnostics::default();
    let admission = Admission::for_test(Uuid::from_u128(OWN), &log);
    let dm = |value, peer: Option<String>| DirectConversation {
        id: Uuid::from_u128(value).to_string(),
        user: UserSummary {
            id: peer,
            ..UserSummary::default()
        },
        ..DirectConversation::default()
    };
    let cases = [
        (
            vec![DirectConversation {
                id: String::new(),
                ..dm(1, id(7))
            }],
            AdmissionFault::InvalidId,
        ),
        (vec![dm(1, None)], AdmissionFault::InvalidId),
        (
            vec![dm(1, id(7)), dm(1, id(8))],
            AdmissionFault::DuplicateId,
        ),
    ];
    for (dms, fault) in cases {
        assert_eq!(
            admission.dms(DirectConversationList { dms }).rejected(),
            rejected(CacheResource::Dms, fault)
        );
    }
    let blocked = |value| UserSummary {
        id: value,
        ..UserSummary::default()
    };
    let cases = [
        (
            vec![blocked(Some(String::new()))],
            AdmissionFault::InvalidId,
        ),
        (
            vec![blocked(id(7)), blocked(id(7))],
            AdmissionFault::DuplicateId,
        ),
    ];
    for (users, fault) in cases {
        assert_eq!(
            admission.blocks(BlockListResponse { users }).rejected(),
            rejected(CacheResource::Blocks, fault)
        );
    }
    assert!(
        admission
            .dms(DirectConversationList::default())
            .unwrap()
            .is_empty()
    );
    assert!(
        admission
            .blocks(BlockListResponse::default())
            .unwrap()
            .is_empty()
    );
}
