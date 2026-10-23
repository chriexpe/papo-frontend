//! Typed presentation boundary for the shared authoring contract.
use crate::{
    account,
    bindings::{Backend, CoreError, NativeClient},
    media::{EmbedFieldView, EmbedView},
};
use papo_core::{
    api::{models::*, net::Update},
    embed_authoring as core,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize, uniffi::Record)]
pub struct EmbedMediaDraft {
    pub url: String,
    pub mime: String,
    pub attachment: String,
}
impl From<EmbedMediaDraft> for CustomEmbedMediaInput {
    fn from(m: EmbedMediaDraft) -> Self {
        Self {
            url: m.url,
            mime_type: m.mime,
            attachment_name: m.attachment,
        }
    }
}
impl From<CustomEmbedMediaInput> for EmbedMediaDraft {
    fn from(m: CustomEmbedMediaInput) -> Self {
        Self {
            url: m.url,
            mime: m.mime_type,
            attachment: m.attachment_name,
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, uniffi::Record)]
#[serde(from = "CustomEmbedInput", into = "CustomEmbedInput")]
pub struct EmbedDraft {
    pub title: String,
    pub description: String,
    pub url: String,
    pub color: String,
    pub site: String,
    pub author: String,
    pub author_url: String,
    pub author_icon: String,
    pub footer: String,
    pub thumbnail: Option<EmbedMediaDraft>,
    pub image: Option<EmbedMediaDraft>,
    pub video: Option<EmbedMediaDraft>,
    pub fields: Vec<EmbedFieldView>,
}
impl From<EmbedDraft> for CustomEmbedInput {
    fn from(d: EmbedDraft) -> Self {
        Self {
            title: d.title,
            description: d.description,
            url: d.url,
            color: d.color,
            site_name: d.site,
            author: (!d.author.is_empty() || !d.author_url.is_empty() || !d.author_icon.is_empty())
                .then_some(CustomEmbedAuthorInput {
                    name: d.author,
                    url: d.author_url,
                    icon_url: d.author_icon,
                }),
            footer: (!d.footer.is_empty()).then_some(CustomEmbedFooterInput { text: d.footer }),
            thumbnail: d.thumbnail.map(Into::into),
            image: d.image.map(Into::into),
            video: d.video.map(Into::into),
            fields: d
                .fields
                .into_iter()
                .map(|f| CustomEmbedFieldInput {
                    name: f.name,
                    value: f.value,
                    inline: f.inline,
                })
                .collect(),
        }
    }
}
impl From<CustomEmbedInput> for EmbedDraft {
    fn from(d: CustomEmbedInput) -> Self {
        let author = d.author.unwrap_or_default();
        Self {
            title: d.title,
            description: d.description,
            url: d.url,
            color: d.color,
            site: d.site_name,
            author: author.name,
            author_url: author.url,
            author_icon: author.icon_url,
            footer: d.footer.unwrap_or_default().text,
            thumbnail: d.thumbnail.map(Into::into),
            image: d.image.map(Into::into),
            video: d.video.map(Into::into),
            fields: d
                .fields
                .into_iter()
                .map(|f| EmbedFieldView {
                    name: f.name,
                    value: f.value,
                    inline: f.inline,
                })
                .collect(),
        }
    }
}
#[derive(uniffi::Record)]
pub struct EmbedIssue {
    pub index: u32,
    pub field: String,
    pub code: String,
    pub limit: u32,
}
#[derive(uniffi::Record)]
pub struct EditableEmbeds {
    pub editable: bool,
    pub has_custom: bool,
    pub drafts: Vec<EmbedDraft>,
}
pub(crate) fn inputs(
    drafts: Vec<EmbedDraft>,
    files: &[String],
) -> Result<Vec<CustomEmbedInput>, String> {
    let inputs: Vec<_> = drafts.into_iter().map(Into::into).collect();
    if core::validate(&inputs, files).is_empty() {
        Ok(inputs)
    } else {
        Err("Invalid custom embeds".into())
    }
}

#[uniffi::export]
pub fn new_embed_draft() -> EmbedDraft {
    EmbedDraft::default()
}
#[uniffi::export]
pub fn new_embed_media() -> EmbedMediaDraft {
    EmbedMediaDraft::default()
}
#[uniffi::export]
pub fn encode_embed_drafts(drafts: Vec<EmbedDraft>) -> String {
    serde_json::to_string(&drafts).unwrap_or_else(|_| "[]".into())
}
#[uniffi::export]
pub fn decode_embed_drafts(json: String) -> Result<Vec<EmbedDraft>, CoreError> {
    if json.len() > 1024 * 1024 {
        return Err("Embed draft is too large".to_owned().into());
    }
    serde_json::from_str(&json).map_err(|e| e.to_string().into())
}
#[uniffi::export]
pub fn validate_embed_drafts(drafts: Vec<EmbedDraft>, attachments: Vec<String>) -> Vec<EmbedIssue> {
    core::validate(
        &drafts.into_iter().map(Into::into).collect::<Vec<_>>(),
        &attachments,
    )
    .into_iter()
    .map(|i| EmbedIssue {
        index: i.index as u32,
        field: i.field,
        code: i.code.into(),
        limit: i.limit as u32,
    })
    .collect()
}
#[uniffi::export]
pub fn embed_draft_preview(draft: EmbedDraft) -> EmbedView {
    EmbedView::from(&core::preview(&draft.into(), "draft"))
}

#[uniffi::export]
impl NativeClient {
    pub fn editable_embeds(&self, owner: String, id: String) -> Result<EditableEmbeds, CoreError> {
        let backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        let store = match backend.as_ref() {
            Some(Backend::Live(c)) => &c.runtime.store,
            Some(Backend::Demo(c)) => &c.store,
            None => return Err("Client is closed".to_owned().into()),
        };
        if store.me != owner {
            return Err("Account changed".to_owned().into());
        }
        let message = store
            .preview_message(&id)
            .filter(|m| store.can_edit_message(m))
            .ok_or("Only the message author can edit it".to_owned())?;
        let drafts = core::editable(&message.previews);
        Ok(EditableEmbeds {
            editable: drafts.is_some(),
            has_custom: message
                .previews
                .iter()
                .any(|p| p.source_type.as_deref() == Some("custom")),
            drafts: drafts
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }

    pub fn edit_message(
        &self,
        owner: String,
        id: String,
        content: String,
        embeds: Option<Vec<EmbedDraft>>,
    ) -> Result<(), CoreError> {
        {
            let mut backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            let store = match backend.as_mut() {
                Some(Backend::Live(c)) => &c.runtime.store,
                Some(Backend::Demo(c)) => &c.store,
                None => return Err("Client is closed".to_owned().into()),
            };
            if store.me != owner {
                return Err("Account changed".to_owned().into());
            }
            let message = store
                .preview_message(&id)
                .filter(|m| store.can_edit_message(m))
                .ok_or("Only the message author can edit it".to_owned())?;
            if embeds.is_some() && core::editable(&message.previews).is_none() {
                return Err("Protected embed media must be preserved".to_owned().into());
            }
            if content.trim().is_empty()
                && embeds.as_ref().map_or(
                    !message
                        .previews
                        .iter()
                        .any(|p| p.source_type.as_deref() == Some("custom")),
                    |e| e.is_empty(),
                )
                && message.attachments.is_empty()
            {
                return Err("A message needs text, an attachment or an embed"
                    .to_owned()
                    .into());
            }
            if let Some(Backend::Demo(c)) = backend.as_mut() {
                return c
                    .dispatch(crate::Action::Edit {
                        message_id: id,
                        content,
                        embeds,
                    })
                    .map_err(Into::into);
            }
        }
        let embeds = embeds.map(|e| inputs(e, &[])).transpose()?;
        let request = self.scoped_api(&owner)?;
        let message = account::run(request.api.edit_message(&id, &content, embeds))?;
        self.finish_request(&request, vec![Update::Edited(Box::new(message))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, UploadFile};

    #[test]
    fn typed_drafts_roundtrip_all_fields_and_reject_invalid_references() {
        let input: CustomEmbedInput = serde_json::from_value(serde_json::json!({
            "title":"Title", "description":"Description", "url":"https://example.org", "color":"#112233", "site_name":"site",
            "author":{"name":"Author", "url":"https://example.org/author", "icon_url":"https://example.org/avatar.png"},
            "footer":{"text":"Footer"}, "image":{"attachment_name":"photo.png","mime_type":"image/png"},
            "thumbnail":{"url":"https://example.org/thumb.gif"}, "video":{"url":"https://example.org/clip.webm","mime_type":"video/webm"},
            "fields":[{"name":"key","value":"value","inline":true}]
        })).unwrap();
        let restored =
            decode_embed_drafts(encode_embed_drafts(vec![input.clone().into()])).unwrap();
        assert_eq!(CustomEmbedInput::from(restored[0].clone()), input);
        assert!(validate_embed_drafts(restored.clone(), vec!["photo.png".into()]).is_empty());
        assert!(!validate_embed_drafts(restored, vec!["renamed.png".into()]).is_empty());
        assert!(decode_embed_drafts("x".repeat(1024 * 1024 + 1)).is_err());
    }

    #[test]
    fn acknowledged_edits_are_author_only_and_preserve_protected_embeds_on_text_edits() {
        let client = NativeClient::demo();
        client
            .dispatch(Action::Send {
                local_id: "custom-draft".into(),
                channel_id: "general".into(),
                content: "original".into(),
                reply_to: None,
                notify_reply: false,
                embeds: vec![EmbedDraft {
                    title: "original card".into(),
                    fields: vec![EmbedFieldView {
                        name: "field".into(),
                        value: "kept".into(),
                        inline: true,
                    }],
                    ..Default::default()
                }],
            })
            .unwrap();
        assert!(
            client
                .editable_embeds("other".into(), "custom-draft".into())
                .is_err()
        );
        let editable = client
            .editable_embeds("me".into(), "custom-draft".into())
            .unwrap();
        assert!(editable.editable && editable.has_custom);
        assert_eq!(editable.drafts[0].fields[0].value, "kept");
        client
            .edit_message("me".into(), "custom-draft".into(), "changed".into(), None)
            .unwrap();
        assert_eq!(
            client
                .editable_embeds("me".into(), "custom-draft".into())
                .unwrap()
                .drafts[0]
                .title,
            "original card"
        );
        client
            .edit_message(
                "me".into(),
                "custom-draft".into(),
                "changed".into(),
                Some(vec![]),
            )
            .unwrap();
        assert!(
            !client
                .editable_embeds("me".into(), "custom-draft".into())
                .unwrap()
                .has_custom
        );
        assert!(
            client
                .edit_message("me".into(), "custom-draft".into(), "".into(), Some(vec![]))
                .is_err()
        );

        let path =
            std::env::temp_dir().join(format!("embed-image-{}", papo_core::cache::new_local_id()));
        let bytes = include_bytes!("../../../android/compose/src/main/assets/demo/portrait.png");
        std::fs::write(&path, bytes).unwrap();
        client
            .dispatch(Action::SendAttachments {
                local_id: "protected-card".into(),
                channel_id: "general".into(),
                content: "caption".into(),
                reply_to: None,
                notify_reply: false,
                files: vec![UploadFile {
                    path: path.to_string_lossy().into(),
                    name: "photo.png".into(),
                    mime: "image/png".into(),
                    size: bytes.len() as u64,
                    spoiler: false,
                }],
                embeds: vec![EmbedDraft {
                    image: Some(EmbedMediaDraft {
                        attachment: "photo.png".into(),
                        mime: "image/png".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
            })
            .unwrap();
        let before = client
            .poll()
            .unwrap()
            .messages
            .into_iter()
            .find(|m| m.id == "protected-card")
            .unwrap();
        assert!(
            before.attachments.is_empty(),
            "an embed attachment must not appear twice"
        );
        assert_eq!(
            client
                .embed_image(before.embeds[0].id.clone(), false)
                .unwrap(),
            bytes
        );
        assert!(
            !client
                .editable_embeds("me".into(), "protected-card".into())
                .unwrap()
                .editable
        );
        assert!(
            client
                .edit_message(
                    "me".into(),
                    "protected-card".into(),
                    "new caption".into(),
                    Some(vec![])
                )
                .is_err()
        );
        client
            .edit_message(
                "me".into(),
                "protected-card".into(),
                "new caption".into(),
                None,
            )
            .unwrap();
        let after = client
            .poll()
            .unwrap()
            .messages
            .into_iter()
            .find(|m| m.id == "protected-card")
            .unwrap();
        assert_eq!(after.embeds[0].id, before.embeds[0].id);
        assert_eq!(after.content, "new caption");
        std::fs::remove_file(path).unwrap();
    }
}
