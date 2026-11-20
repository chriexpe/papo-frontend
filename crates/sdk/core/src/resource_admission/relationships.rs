//! Social relationship and requested-profile admission. These observations do not establish roster membership.

use super::{Admission, AdmissionFault, ResourceRejection, parse_id};
use crate::{
    CacheResource, DirectRecord, ProfileRecord, Uuid,
    resources::{ProfileBatch, UserObservation},
};
use papo_api::wire;
use std::collections::HashSet;

impl Admission<'_> {
    pub fn profile(
        self,
        requested: Uuid,
        raw: wire::UserProfile,
    ) -> Result<(ProfileRecord, UserObservation), ResourceRejection> {
        let group = CacheResource::Profiles;
        if parse_id(raw.id.as_deref()) != Some(requested) {
            return Err(self.reject(group, AdmissionFault::UnexpectedId));
        }
        self.profile_fields(requested, raw)
    }

    fn profile_fields(
        self,
        id: Uuid,
        raw: wire::UserProfile,
    ) -> Result<(ProfileRecord, UserObservation), ResourceRejection> {
        let user = wire::UserSummary {
            username: raw.username,
            nickname: raw.nickname,
            typing: raw.typing,
            created_at: raw.created_at,
            roles: raw.roles,
            ..Default::default()
        };
        Ok((
            ProfileRecord {
                id,
                description: raw.description,
                avatar_format: raw.avatar_format,
                banner_media: raw.banner_media,
            },
            self.user(CacheResource::Profiles, id, user, id == self.own)?,
        ))
    }

    pub fn profiles(
        self,
        requested: &[Uuid],
        profiles: Option<Vec<wire::UserProfile>>,
    ) -> Result<ProfileBatch, ResourceRejection> {
        let group = CacheResource::Profiles;
        let rows = profiles.ok_or_else(|| self.reject(group, AdmissionFault::MissingList))?;
        let mut seen = HashSet::new();
        let mut present = Vec::with_capacity(rows.len());
        for raw in rows {
            let id = self.required_id(group, raw.id.as_deref())?;
            if !requested.contains(&id) {
                return Err(self.reject(group, AdmissionFault::UnexpectedId));
            }
            if !seen.insert(id) {
                return Err(self.reject(group, AdmissionFault::DuplicateId));
            }
            present.push(self.profile_fields(id, raw)?);
        }
        let absent = requested
            .iter()
            .copied()
            .filter(|id| !seen.contains(id))
            .collect();
        Ok(ProfileBatch { present, absent })
    }

    pub fn dms(
        self,
        list: wire::DirectConversationList,
    ) -> Result<Vec<(DirectRecord, UserObservation)>, ResourceRejection> {
        let group = CacheResource::Dms;
        let mut seen = HashSet::with_capacity(list.dms.len());
        list.dms
            .into_iter()
            .map(|dm| {
                let id = self.required_id(group, Some(&dm.id))?;
                if !seen.insert(id) {
                    return Err(self.reject(group, AdmissionFault::DuplicateId));
                }
                let peer_id = self.required_id(group, dm.user.id.as_deref())?;
                let user = self.user(group, peer_id, dm.user, peer_id == self.own)?;
                let record = DirectRecord {
                    id,
                    peer_id,
                    created_at: dm.created_at,
                    unread_count: dm.unread_count,
                    last_message: dm
                        .last_message
                        .map(|head| self.head(group, head))
                        .transpose()?,
                    last_read_message: self.optional_id(group, dm.last_read_message.as_deref())?,
                    last_read_at: dm.last_read_at,
                };
                Ok((record, user))
            })
            .collect()
    }

    pub fn blocks(
        self,
        list: wire::BlockListResponse,
    ) -> Result<Vec<UserObservation>, ResourceRejection> {
        let group = CacheResource::Blocks;
        let mut seen = HashSet::with_capacity(list.users.len());
        list.users
            .into_iter()
            .map(|raw| {
                let id = self.required_id(group, raw.id.as_deref())?;
                if !seen.insert(id) {
                    return Err(self.reject(group, AdmissionFault::DuplicateId));
                }
                self.user(group, id, raw, id == self.own)
            })
            .collect()
    }
}
