//! Compares SDK affordances with actual pinned Go services/middleware on disposable PostgreSQL.
//! Expected decisions come from backend execution, never from the SDK's own permission evaluator.
use papo_sdk::{Generation, ResourceSnapshot, Uuid, permissions::global_permissions};
use papo_sdk_ui::capabilities;
use serde_json::Value;
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires isolated pinned backend and disposable PostgreSQL; run scripts/papo-backend-proof.py"]
async fn capabilities_match_pinned_go_for_role_combinations_overrides_owner_and_dm_restrictions() {
    let address = std::env::var("PAPO_BACKEND_PROOF_ADDRESS").unwrap();
    let response = reqwest::Client::new()
        .post(format!("{address}/__proof/permissions"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let report: Value = response.json().await.unwrap();
    let cases = report["ordinary"].as_array().unwrap();
    assert_eq!(cases.len(), 2060);
    let mut operations = 0;
    for (index, case) in cases.iter().enumerate() {
        let member: papo_sdk::UserSummary = serde_json::from_value(case["member"].clone()).unwrap();
        let user: Uuid = member.id.as_deref().unwrap().parse().unwrap();
        let channel: papo_sdk::Channel = serde_json::from_value(case["channel"].clone()).unwrap();
        let channel_id = channel.id.as_deref().unwrap().parse().unwrap();
        let snapshot = ResourceSnapshot {
            version: 1,
            generation: Generation::for_test(1),
            server: Some(Arc::new(
                serde_json::from_value(case["server"].clone()).unwrap(),
            )),
            channels: Arc::new(vec![channel]),
            roles: Arc::new(serde_json::from_value(case["roles"].clone()).unwrap()),
            members: Arc::new(vec![member]),
            profiles: Arc::default(),
            dms: Arc::default(),
            blocks: Arc::default(),
            presence: Arc::default(),
        };
        let global = global_permissions(&snapshot, user).unwrap();
        let actual = [
            global.manage_server,
            global.manage_channels,
            global.manage_roles,
            global.manage_webhooks,
            global.ban_members,
            global.pin_message,
            global.everyone_message,
            global.send_attachment,
        ];
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            case["global"],
            "global case {index}"
        );
        if case["operations"].is_array() {
            operations += 1;
            let caps = capabilities(&snapshot, user, channel_id).unwrap();
            let actual = [
                caps.read,
                caps.send,
                caps.connect_voice,
                caps.pin,
                caps.unpin,
                caps.can_delete(false),
                caps.can_edit(true),
                caps.can_delete(true),
            ];
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                case["operations"],
                "operation case {index}"
            );
        }
    }
    assert_eq!(operations, 1036);
    let direct = report["direct"].as_array().unwrap();
    assert_eq!(direct.len(), 12);
    for (index, case) in direct.iter().enumerate() {
        let grants = papo_sdk::permissions::direct_grants(
            case["member"].as_bool().unwrap(),
            case["blocked"].as_bool().unwrap(),
        );
        let caps = papo_sdk_ui::ChannelCapabilities::from_grants(
            "dm",
            grants,
            papo_sdk::permissions::GlobalPermissions::default(),
        );
        let actual = [
            caps.read,
            caps.send,
            caps.connect_voice,
            caps.pin,
            caps.unpin,
            caps.can_delete(false),
            caps.can_edit(true),
            caps.can_delete(true),
        ];
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            case["operations"],
            "direct case {index}"
        );
    }
}

#[test]
#[ignore = "requires isolated pinned backend and disposable PostgreSQL; run scripts/papo-backend-proof.py"]
fn sdk_typing_command_uses_session_gateway_and_live_generation() {
    use papo_sdk::{
        Client, ClientConfig, ConnectionState, DataRoot, LoginRequest, UnixMicros,
        secret::test_store::ScriptedSecretStore,
    };
    let profile = tempfile::tempdir().unwrap();
    let mut client = Client::open(
        DataRoot::new(profile.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig {
            insecure_loopback: true,
            ..ClientConfig::default()
        },
    )
    .unwrap();
    client
        .block_on(async {
            let address = std::env::var("PAPO_BACKEND_PROOF_ADDRESS").unwrap();
            let handle = client.handle();
            let server = handle.add_server(&address, UnixMicros(0)).await.unwrap();
            let session = handle
                .auth(server)
                .unwrap()
                .login(
                    LoginRequest {
                        username: "cursorproof".into(),
                        password: std::env::var("PAPO_PROOF_PASSWORD").unwrap(),
                    },
                    UnixMicros(0),
                )
                .await
                .unwrap();
            let mut connection = session.subscribe_connection();
            while connection.borrow_and_update().state != ConnectionState::Online {
                connection.changed().await.unwrap();
            }
            let resources = session.subscribe_resources().borrow().clone();
            let channel: Uuid = resources
                .channels
                .iter()
                .find(|channel| channel.name.as_deref() == Some("upload-proof"))
                .unwrap()
                .id
                .as_deref()
                .unwrap()
                .parse()
                .unwrap();
            let mut activity = session.subscribe_activity();
            session.report_typing(channel).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    let image = activity.borrow_and_update().clone();
                    if image
                        .typing
                        .contains_key(&(channel, session.account().user()))
                    {
                        assert_eq!(image.generation, session.generation());
                        break;
                    }
                    activity.changed().await.unwrap();
                }
            })
            .await
            .unwrap();
        })
        .unwrap();
    assert!(client.shutdown().unwrap().reads_drained);
}
