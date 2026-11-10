//! Owns authoritative non-message fixture snapshots; leaves authentication, permission evaluation and event delivery outside.

use std::collections::BTreeMap;

use papo_api::wire::{
    BlockListResponse, Channel, ChannelList, ChannelPermissionsResponse, DirectConversation,
    DirectConversationList, Role, RoleListResponse, Server, UserProfile, UserSummary,
};
use serde_json::json;

use crate::{
    Config,
    response::{Reply, problem},
};

pub(crate) const CREATED_AT: &str = "2026-01-01T00:00:00Z";

#[derive(Default)]
pub(crate) struct Resources {
    pub(crate) server: Option<Server>,
    pub(crate) channels: BTreeMap<String, Channel>,
    pub(crate) roles: Vec<Role>,
    pub(crate) profiles: BTreeMap<String, UserProfile>,
    pub(crate) dms: BTreeMap<String, Vec<DirectConversation>>,
    pub(crate) blocks: BTreeMap<String, Vec<UserSummary>>,
    pub(crate) read_marks: BTreeMap<(String, String), (String, String)>,
    pub(crate) next_channel_id: u64,
}

impl Resources {
    pub(crate) fn register(&mut self, id: &str, username: &str) {
        self.profiles.insert(
            id.to_owned(),
            UserProfile {
                id: Some(id.to_owned()),
                username: Some(username.to_owned()),
                avatar_format: Some(String::new()),
                created_at: Some(CREATED_AT.into()),
                roles: Some(Vec::new()),
                ..UserProfile::default()
            },
        );
    }

    pub(crate) fn handle(
        &mut self,
        config: &Config,
        method: &str,
        path: &str,
        query: &str,
        user_id: Option<&str>,
        body: &[u8],
    ) -> Option<Reply> {
        if method == "GET" && path == "/server" {
            return Some(if config.server_exists {
                self.server_info(config)
            } else {
                problem(404, "not-found", path)
            });
        }
        let known = matches!(
            path,
            "/channels"
                | "/roles"
                | "/users"
                | "/dms"
                | "/users/blocks"
                | "/users/profile_batch"
                | "/users/user_summary_batch"
        ) || path.starts_with("/channels/")
            || path.ends_with("/profile");
        if !known {
            return None;
        }
        let Some(user_id) = user_id else {
            return Some(problem(401, "unauthorized", path));
        };
        let reply = match (method, path) {
            ("GET", "/channels") => self.channel_list(user_id),
            ("GET", "/roles") => Reply::json(
                200,
                json!(RoleListResponse {
                    roles: Some(self.roles.clone())
                }),
            ),
            ("GET", "/users") => self.members(query),
            ("POST", "/users/profile_batch") => self.profile_batch(body, path),
            ("POST", "/users/user_summary_batch") => self.summary_batch(body, path),
            ("GET", "/dms") => Reply::json(
                200,
                json!(DirectConversationList {
                    dms: self.dms.get(user_id).cloned().unwrap_or_default(),
                }),
            ),
            ("GET", "/users/blocks") => Reply::json(
                200,
                json!(BlockListResponse {
                    users: self.blocks.get(user_id).cloned().unwrap_or_default(),
                }),
            ),
            ("GET", _) if path.ends_with("/profile") => {
                let id = path
                    .strip_prefix("/users/")
                    .and_then(|path| path.strip_suffix("/profile"));
                id.and_then(|id| self.profiles.get(id)).map_or_else(
                    || problem(404, "not-found", path),
                    |profile| Reply::json(200, json!(profile)),
                )
            }
            ("GET", _) if path.ends_with("/permissions") => {
                let id = path
                    .strip_prefix("/channels/")
                    .and_then(|path| path.strip_suffix("/permissions"));
                id.and_then(|id| self.channels.get(id)).map_or_else(
                    || problem(404, "not-found", path),
                    |channel| {
                        Reply::json(
                            200,
                            json!(ChannelPermissionsResponse {
                                channel_id: channel.id.clone(),
                                permissions: Some(channel.permissions.clone().unwrap_or_default()),
                            }),
                        )
                    },
                )
            }
            _ => problem(404, "not-found", path),
        };
        Some(reply)
    }

    fn server_info(&self, config: &Config) -> Reply {
        let mut server = self.server.clone().unwrap_or_else(|| Server {
            id: Some("00000000-0000-4000-8000-000000000000".into()),
            name: Some(config.server_name.clone()),
            icon_format: Some(String::new()),
            public: Some(config.public),
            created_at: Some(CREATED_AT.into()),
            ..Server::default()
        });
        server.channel_count = i64::try_from(self.channels.len()).ok();
        server.role_count = i64::try_from(self.roles.len()).ok();
        server.member_count = i64::try_from(self.profiles.len()).ok();
        Reply::json(200, json!(server))
    }

    fn channel_list(&self, user_id: &str) -> Reply {
        let mut channels: Vec<_> = self
            .channels
            .values()
            .cloned()
            .map(|mut channel| {
                channel.last_read_message = None;
                channel.last_read_at = None;
                if let Some(mark) = channel
                    .id
                    .as_ref()
                    .and_then(|id| self.read_marks.get(&(user_id.to_owned(), id.clone())))
                {
                    channel.last_read_message = Some(mark.0.clone());
                    channel.last_read_at = Some(mark.1.clone());
                }
                channel
            })
            .collect();
        channels.sort_by(|left, right| (left.position, &left.id).cmp(&(right.position, &right.id)));
        Reply::json(
            200,
            json!(ChannelList {
                channels: Some(channels)
            }),
        )
    }
}
