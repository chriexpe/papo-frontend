//! Generated gateway payloads; leaves transport and policy outside.
/// Backend `voice_joined` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct VoiceJoinedEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `members` field.
    #[serde(default, deserialize_with = "crate::gateway::nullable_list")]
    pub members: Vec<VoiceState>,
    /// Backend `active_speakers` field.
    #[serde(default, deserialize_with = "crate::gateway::nullable_list")]
    pub active_speakers: Vec<String>,
}
/// Backend `voice_answer` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct VoiceAnswerEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `sdp` field.
    pub sdp: String,
}
/// Backend `voice_offer` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct VoiceOfferEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `sdp` field.
    pub sdp: String,
}
/// Backend `voice_ice_candidate` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct VoiceICECandidateEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `candidate` field.
    pub candidate: String,
    /// Backend `sdp_mid` field.
    #[serde(default)]
    pub sdp_mid: Option<String>,
    /// Backend `sdp_mline_index` field.
    #[serde(default)]
    pub sdp_mline_index: Option<i64>,
}
/// Backend `voice_state_update` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct VoiceStateUpdateEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `user_id` field.
    pub user_id: String,
    /// Backend `muted` field.
    pub muted: bool,
    /// Backend `camera_on` field.
    pub camera_on: bool,
    /// Backend `screen_sharing` field.
    pub screen_sharing: bool,
}
/// Backend `voice_leave` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct VoiceLeaveEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `user_id` field.
    pub user_id: String,
}
/// Backend `active_speaker_update` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ActiveSpeakerUpdateEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `user_ids` field.
    #[serde(default, deserialize_with = "crate::gateway::nullable_list")]
    pub user_ids: Vec<String>,
}
/// Backend `voice_audio_routes` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct VoiceAudioRoutesEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `routes` field.
    #[serde(default, deserialize_with = "crate::gateway::nullable_list")]
    pub routes: Vec<VoiceAudioRoute>,
}
/// Backend `_VoiceState` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct VoiceState {
    /// Backend `user_id` field.
    pub user_id: String,
    /// Backend `muted` field.
    pub muted: bool,
    /// Backend `camera_on` field.
    pub camera_on: bool,
    /// Backend `screen_sharing` field.
    pub screen_sharing: bool,
}
/// Backend `_VoiceAudioRoute` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct VoiceAudioRoute {
    /// Backend `track_id` field.
    pub track_id: String,
    /// Backend `user_id` field.
    pub user_id: String,
}
