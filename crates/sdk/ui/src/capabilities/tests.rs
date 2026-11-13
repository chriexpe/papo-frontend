//! Operation-specific independent gates and external identity separation, with no backend simulation.
use super::*;
#[test]
fn typing_and_reaction_removal_use_read_while_add_uses_send() {
    let read = ChannelCapabilities::from_grants(
        "text",
        ChannelGrants {
            read: true,
            ..ChannelGrants::default()
        },
        GlobalPermissions::default(),
    );
    assert!(read.type_here && read.remove_reaction);
    assert!(!read.send && !read.add_reaction);
    let send = ChannelCapabilities::from_grants(
        "voice",
        ChannelGrants {
            send: true,
            voice: true,
            ..ChannelGrants::default()
        },
        GlobalPermissions::default(),
    );
    assert!(send.send && send.add_reaction && send.connect_voice);
    assert!(!send.type_here && !send.remove_reaction && !send.read && !send.attach);
}
#[test]
fn pin_and_unpin_are_asymmetric_and_direct_attachments_do_not_require_a_role() {
    let pin = ChannelCapabilities::from_grants(
        "text",
        ChannelGrants::default(),
        GlobalPermissions {
            pin_message: true,
            ..GlobalPermissions::default()
        },
    );
    assert!(pin.pin && !pin.unpin);
    let direct = ChannelCapabilities::from_grants(
        "dm",
        direct_grants(true, false),
        GlobalPermissions::default(),
    );
    assert!(direct.attach && direct.pin && direct.unpin);
    assert!(!direct.delete_others && !direct.connect_voice);
    let blocked = ChannelCapabilities::from_grants(
        "dm",
        direct_grants(true, true),
        GlobalPermissions {
            owner: true,
            ..GlobalPermissions::default()
        },
    );
    assert!(!blocked.read && !blocked.send && !blocked.pin && !blocked.can_edit(true));
    assert!(blocked.can_delete(true));
}
#[test]
fn external_author_never_becomes_the_bridge_person() {
    let message = Message {
        author_id: Some(Uuid::from_u128(1).to_string()),
        external_author: Some(papo_sdk::ExternalAuthor::default()),
        ..Message::default()
    };
    assert!(matches!(
        AuthorIdentity::for_message(&message),
        AuthorIdentity::External(_)
    ));
    let message = Message {
        author_id: message.author_id,
        webhook_id: Some(Uuid::from_u128(2).to_string()),
        ..Message::default()
    };
    assert!(matches!(
        AuthorIdentity::for_message(&message),
        AuthorIdentity::App
    ));
}

#[test]
fn administrative_affordances_follow_route_gates_instead_of_similarly_named_bits() {
    let stored_ban_flag = ServerCapabilities::from_grants(GlobalPermissions {
        ban_members: true,
        ..GlobalPermissions::default()
    });
    assert!(!stored_ban_flag.can_ban(false));
    let manage_server = ServerCapabilities::from_grants(GlobalPermissions {
        manage_server: true,
        ..GlobalPermissions::default()
    });
    assert!(manage_server.can_ban(false));
    assert!(!manage_server.can_ban(true));
    assert!(!manage_server.reset_other_password);
    assert!(
        ServerCapabilities::from_grants(GlobalPermissions {
            owner: true,
            ..GlobalPermissions::default()
        })
        .reset_other_password
    );
}
