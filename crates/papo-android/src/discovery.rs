//! Search and community settings projections; all requests use the core actor.
use super::*;
use crate::bindings::NativeClient;
use papo_core::{api::models, state::Store};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub manage_server: bool,
    pub manage_channels: bool,
    pub manage_roles: bool,
    pub ban_members: bool,
    pub pin_message: bool,
    pub everyone_message: bool,
    pub send_attachment: bool,
}
impl Capabilities {
    pub fn from_store(store: &Store) -> Self {
        if store.is_server_owner() {
            return Self {manage_server:true,manage_channels:true,manage_roles:true,ban_members:true,pin_message:true,everyone_message:true,send_attachment:true};
        }
        let mut value = Self::from(store.my_role_permissions());
        value.manage_server = store.can_manage_server();
        value.manage_channels = store.can_manage_channels();
        value.manage_roles = store.can_manage_roles();
        value
    }
}
impl From<models::RolePermissions> for Capabilities {
    fn from(p: models::RolePermissions) -> Self {
        Self {
            manage_server: p.manage_server,
            manage_channels: p.manage_channels,
            manage_roles: p.manage_roles,
            ban_members: p.ban_members,
            pin_message: p.pin_message,
            everyone_message: p.everyone_message,
            send_attachment: p.send_attachment,
        }
    }
}
impl From<Capabilities> for models::RolePermissions {
    fn from(p: Capabilities) -> Self {
        Self {
            manage_server: p.manage_server,
            manage_channels: p.manage_channels,
            manage_roles: p.manage_roles,
            ban_members: p.ban_members,
            pin_message: p.pin_message,
            everyone_message: p.everyone_message,
            send_attachment: p.send_attachment,
        }
    }
}
#[derive(Clone, Debug, Deserialize, uniffi::Record)]
pub struct RoleView {
    pub id: String,
    pub name: String,
    pub color: String,
    pub permissions: Capabilities,
}
#[derive(Clone, Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub message: Option<bindings::MessageView>,
    pub id: String,
    pub kind: String,
    pub content: String,
    pub channel_id: String,
    pub channel_name: String,
    pub author_id: String,
    pub author: String,
    pub at: i64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SearchSuggestion {
    pub label: String,
    pub replacement: String,
    pub start: u32,
    pub end: u32,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SearchAnalysis {
    pub suggestions: Vec<SearchSuggestion>,
    pub valid: bool,
    pub text: String,
}

pub(crate) fn check_notifications(store: &Store, id: &str, setting: &str) -> Result<(), String> {
    if !matches!(setting, "all" | "only_mentions" | "off")
        || store
            .channel(id)
            .is_none_or(|c| c.kind == ChannelKind::Category)
    {
        return Err("Choose a valid channel notification setting".into());
    }
    Ok(())
}
pub(crate) fn check_server(
    store: &Store,
    name: &str,
    public: bool,
    password: Option<&str>,
) -> Result<(), String> {
    if name.trim().is_empty() || name.chars().count() > 32 {
        return Err("Choose a server name between 1 and 32 characters".into());
    }
    let making_private = store.server.as_ref().is_some_and(|s| s.public) && !public;
    if (making_private || password.is_some()) && password.is_none_or(|p| p.chars().count() < 8) {
        return Err("A new server password needs at least 8 characters".into());
    }
    Ok(())
}
impl Client {
    pub(crate) fn search(&mut self, query: String, more: bool) -> Result<(), String> {
        let store = &mut self.runtime.store;
        if !more && store.searching && query == self.search_query {
            return Ok(());
        }
        if more && (store.searching || !store.search_has_more || query != self.search_query) {
            return Ok(());
        }
        let request = papo_core::search::search_request(&query, store, false, false, None)
            .ok_or("Complete the search filters before searching")?;
        let cursor = if more {
            store
                .search_results
                .last()
                .and_then(|r| r.created_at.map(|at| (at, r.id.clone())))
        } else {
            None
        };
        if more && cursor.is_none() {
            store.search_has_more = false;
            return Ok(());
        }
        if !more {
            store.reset_search();
            store.error = None;
            self.search_query = query;
        }
        store.searching = true;
        self.runtime.net.send(Command::Search {
            generation: store.search_generation,
            request,
            cursor,
            append: more,
        });
        Ok(())
    }
}

#[uniffi::export]
impl NativeClient {
    pub fn analyze_search(
        &self,
        query: String,
        cursor: u32,
    ) -> Result<SearchAnalysis, bindings::CoreError> {
        let backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        let store = match backend.as_ref().ok_or("Client is closed".to_owned())? {
            bindings::Backend::Live(c) => &c.runtime.store,
            bindings::Backend::Demo(c) => &c.store,
        };
        // Compose offsets are UTF-16; Rust slices are byte boundaries.
        let mut offset = 0;
        let caret = query
            .char_indices()
            .find_map(|(i, ch)| {
                if offset + ch.len_utf16() > cursor as usize {
                    Some(i)
                } else {
                    offset += ch.len_utf16();
                    None
                }
            })
            .unwrap_or(query.len());
        let (start, end) = papo_core::search::search_token_range(&query, caret);
        let suggestions = papo_core::search::search_shortcuts(store, &query[..caret], false)
            .into_iter()
            .map(|hint| SearchSuggestion {
                label: hint.label,
                replacement: hint.replacement,
                start: query[..start].encode_utf16().count() as u32,
                end: query[..end].encode_utf16().count() as u32,
            })
            .collect();
        let request = papo_core::search::search_request(&query, store, false, false, None);
        Ok(SearchAnalysis {
            suggestions,
            valid: request.is_some(),
            text: request.and_then(|r| r.text).unwrap_or_default(),
        })
    }
    pub fn server_icon(&self) -> Result<Vec<u8>, bindings::CoreError> {
        use base64::Engine;
        let blob = {
            let backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            let store = match backend.as_ref().ok_or("Client is closed".to_owned())? {
                bindings::Backend::Live(c) => &c.runtime.store,
                bindings::Backend::Demo(c) => &c.store,
            };
            store
                .server
                .as_ref()
                .and_then(|s| s.icon.clone())
                .ok_or("Server icon is unavailable".to_owned())?
        };
        base64::engine::general_purpose::STANDARD
            .decode(blob.split_once("base64,").map(|(_, b)| b).unwrap_or(&blob))
            .map_err(|_| bindings::CoreError::from("Invalid server icon".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permissions_follow_owner_and_roles_instead_of_presence_or_names() {
        let mut demo = demo::Demo::new();
        let owner: bindings::ClientSnapshot = serde_json::from_value(demo.poll()).unwrap();
        assert!(
            owner.capabilities.manage_server
                && owner.capabilities.manage_channels
                && owner.capabilities.manage_roles
        );
        demo.store.me = "ana".into();
        let member: bindings::ClientSnapshot = serde_json::from_value(demo.poll()).unwrap();
        assert!(
            !member.capabilities.manage_server
                && !member.capabilities.manage_channels
                && !member.capabilities.manage_roles
        );
        demo.store.roles[0].permissions.manage_channels = true;
        demo.store
            .members
            .iter_mut()
            .find(|m| m.id == "ana")
            .unwrap()
            .roles = vec!["designers".into()];
        let delegated: bindings::ClientSnapshot = serde_json::from_value(demo.poll()).unwrap();
        assert!(delegated.capabilities.manage_channels);
        assert!(!delegated.capabilities.manage_server);
    }
    #[test]
    fn demo_search_filters_clear_and_open_the_exact_matching_message() {
        let mut demo = demo::Demo::new();
        demo.dispatch(Action::Search {
            query: "de:@ana em:#geral".into(),
            more: false,
        })
        .unwrap();
        let snapshot: bindings::ClientSnapshot = serde_json::from_value(demo.poll()).unwrap();
        assert!(!snapshot.search_results.is_empty());
        assert!(
            snapshot
                .search_results
                .iter()
                .all(|r| r.author_id == "ana" && r.channel_id == "general")
        );
        let hit = &snapshot.search_results[0];
        demo.dispatch(Action::SelectChannel {
            channel_id: hit.channel_id.clone(),
        })
        .unwrap();
        demo.dispatch(Action::JumpToMessage {
            channel_id: hit.channel_id.clone(),
            message_id: hit.id.clone(),
        })
        .unwrap();
        assert_eq!(demo.poll()["navigationTarget"], hit.id);
        demo.dispatch(Action::ClearSearch).unwrap();
        assert!(demo.store.search_results.is_empty());
    }
    #[test]
    fn settings_validate_private_passwords_and_notification_values() {
        let demo = demo::Demo::new();
        assert!(check_server(&demo.store, "Renamed", true, None).is_ok());
        assert!(check_server(&demo.store, "Renamed", false, None).is_err());
        assert!(check_server(&demo.store, "Renamed", false, Some("long-enough")).is_ok());
        assert!(check_notifications(&demo.store, "general", "only_mentions").is_ok());
        assert!(check_notifications(&demo.store, "projects", "all").is_err());
        assert!(check_notifications(&demo.store, "general", "unknown").is_err());
    }
}
