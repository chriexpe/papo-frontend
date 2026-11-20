//! Owns one SDP transaction at a time and defers trickle ICE until its remote SDP is applied.

use crate::pipeline::{MediaResult, RequestedPad};
use gst::{glib, prelude::*};
use gstreamer as gst;
use gstreamer_sdp as sdp;
use gstreamer_webrtc as rtc;
use papo_sdk::{MediaFatalError, MediaReportKind, VideoCodec};
use std::time::Duration;

const MAX_PENDING_CANDIDATES: usize = 128;
const SDP_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct Candidate {
    pub text: String,
    pub mid: Option<String>,
    pub index: Option<i64>,
}

pub(crate) struct Negotiation {
    pub webrtc: gst::Element,
    remote: Option<rtc::WebRTCSessionDescription>,
    pending_ice: Vec<Candidate>,
    awaiting_answer: bool,
}

impl Negotiation {
    pub fn new(webrtc: gst::Element) -> Self {
        Self {
            webrtc,
            remote: None,
            pending_ice: Vec::new(),
            awaiting_answer: false,
        }
    }

    pub fn waiting(&self) -> bool {
        self.awaiting_answer
    }

    pub fn established(&self) -> bool {
        self.remote.is_some()
    }

    pub fn receive_line(&self, caps: &gst::Caps) -> MediaResult<()> {
        self.webrtc
            .emit_by_name::<Option<rtc::WebRTCRTPTransceiver>>(
                "add-transceiver",
                &[&rtc::WebRTCRTPTransceiverDirection::Recvonly, caps],
            )
            .ok_or(MediaFatalError::NegotiationFailed)?;
        Ok(())
    }

    pub async fn offer(&mut self) -> MediaResult<MediaReportKind> {
        let report = self.local("create-offer", "offer").await?;
        self.awaiting_answer = true;
        Ok(report)
    }

    pub async fn apply_remote(
        &mut self,
        text: &str,
        offer: bool,
        codec: Codec,
    ) -> MediaResult<Option<MediaReportKind>> {
        if offer == self.awaiting_answer {
            return Err(MediaFatalError::NegotiationFailed);
        }
        let message = sdp::SDPMessage::parse_buffer(text.as_bytes())
            .map_err(|_| MediaFatalError::NegotiationFailed)?;
        codec.validate_remote(&message)?;
        let kind = if offer {
            rtc::WebRTCSDPType::Offer
        } else {
            rtc::WebRTCSDPType::Answer
        };
        let remote = rtc::WebRTCSessionDescription::new(kind, message);
        promise(|promise| {
            self.webrtc
                .emit_by_name::<()>("set-remote-description", &[&remote, promise]);
        })
        .await?;
        self.remote = Some(remote);
        self.awaiting_answer = false;
        for candidate in std::mem::take(&mut self.pending_ice) {
            self.candidate(candidate)?;
        }
        if offer {
            return self.local("create-answer", "answer").await.map(Some);
        }
        Ok(None)
    }

    async fn local(&self, action: &str, field: &str) -> MediaResult<MediaReportKind> {
        let desc = promise(|promise| {
            self.webrtc
                .emit_by_name::<()>(action, &[&None::<gst::Structure>, promise]);
        })
        .await?
        .and_then(|result| result.get::<rtc::WebRTCSessionDescription>(field).ok())
        .ok_or(MediaFatalError::NegotiationFailed)?;
        let sdp = desc
            .sdp()
            .as_text()
            .map_err(|_| MediaFatalError::NegotiationFailed)?;
        promise(|promise| {
            self.webrtc
                .emit_by_name::<()>("set-local-description", &[&desc, promise]);
        })
        .await?;
        Ok(if field == "offer" {
            MediaReportKind::LocalOffer { sdp }
        } else {
            MediaReportKind::LocalAnswer { sdp }
        })
    }

    pub fn candidate(&mut self, candidate: Candidate) -> MediaResult<()> {
        if candidate.text.trim().is_empty() {
            return Ok(());
        }
        if let Some(remote) = &self.remote
            && !self.awaiting_answer
        {
            if let Some(index) = candidate_index(remote.sdp(), &candidate) {
                self.webrtc
                    .emit_by_name::<()>("add-ice-candidate", &[&index, &candidate.text]);
            }
        } else {
            if self.pending_ice.len() == MAX_PENDING_CANDIDATES {
                return Err(MediaFatalError::NegotiationFailed);
            }
            self.pending_ice.push(candidate);
        }
        Ok(())
    }

    pub fn track(&self, pad: &gst::Pad) -> Option<(bool, String)> {
        let line = pad.property::<Option<rtc::WebRTCRTPTransceiver>>("transceiver")?;
        let media = self.remote.as_ref()?.sdp().media(line.mlineindex())?;
        let video = match media.media()? {
            "video" => true,
            "audio" => false,
            _ => return None,
        };
        let track = media.attribute_val("msid").and_then(track_id).or_else(|| {
            media
                .attributes()
                .filter(|attr| attr.key() == "ssrc")
                .find_map(|attr| {
                    let (_, msid) = attr.value()?.split_once(" msid:")?;
                    track_id(msid)
                })
        })?;
        Some((video, track.to_owned()))
    }
}

fn track_id(msid: &str) -> Option<&str> {
    msid.split_ascii_whitespace().nth(1)
}

fn candidate_index(sdp: &sdp::SDPMessageRef, candidate: &Candidate) -> Option<u32> {
    let supplied_index = candidate.index.map(u32::try_from).transpose().ok()?;
    let index = match (&candidate.mid, supplied_index) {
        (_, Some(index)) => index,
        (Some(mid), None) => sdp
            .medias()
            .position(|media| media.attribute_val("mid") == Some(mid))
            .and_then(|index| u32::try_from(index).ok())?,
        (None, None) => return None,
    };
    let media = sdp.media(index)?;
    if candidate
        .mid
        .as_deref()
        .is_some_and(|mid| media.attribute_val("mid") != Some(mid))
    {
        return None;
    }
    Some(index)
}

async fn promise(emit: impl FnOnce(&gst::Promise)) -> MediaResult<Option<gst::Structure>> {
    let (promise, reply) = gst::Promise::new_future();
    emit(&promise);
    let result = tokio::time::timeout(SDP_TIMEOUT, reply).await;
    let Ok(result) = result else {
        promise.interrupt();
        return Err(MediaFatalError::NegotiationFailed);
    };
    let value = result.map_err(|_| MediaFatalError::NegotiationFailed)?;
    if value.as_ref().is_some_and(|reply| reply.has_field("error")) {
        return Err(MediaFatalError::NegotiationFailed);
    }
    Ok(value.map(|reply| reply.to_owned()))
}

pub(crate) async fn wait_for_rtp(pad: &gst::Pad) -> MediaResult<()> {
    if pad.current_caps().is_some() {
        return Ok(());
    }
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let ready_tx = std::sync::Mutex::new(Some(ready_tx));
    let id = pad.connect_notify(Some("caps"), move |pad, _| {
        if pad.current_caps().is_some() {
            let sender = ready_tx
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(sender) = sender {
                let _ = sender.send(());
            }
        }
    });
    struct Signal<'a>(&'a gst::Pad, Option<glib::SignalHandlerId>);
    impl Drop for Signal<'_> {
        fn drop(&mut self) {
            if let Some(id) = self.1.take() {
                self.0.disconnect(id);
            }
        }
    }
    let _disconnect = Signal(pad, Some(id));
    if pad.current_caps().is_none() {
        tokio::time::timeout(SDP_TIMEOUT, ready_rx)
            .await
            .map_err(|_| MediaFatalError::NegotiationFailed)?
            .map_err(|_| MediaFatalError::Internal)?;
    }
    Ok(())
}

pub(crate) fn sender(
    peer: &gst::Element,
    caps: &gst::Caps,
) -> MediaResult<(RequestedPad, rtc::WebRTCRTPTransceiver)> {
    let request = RequestedPad::new(peer, "sink_%u")?;
    let line = request
        .pad
        .property::<rtc::WebRTCRTPTransceiver>("transceiver");
    line.set_codec_preferences(Some(caps));
    line.set_direction(rtc::WebRTCRTPTransceiverDirection::Sendonly);
    Ok((request, line))
}

pub(crate) fn opus_caps() -> gst::Caps {
    gst::Caps::builder("application/x-rtp")
        .field("media", "audio")
        .field("encoding-name", "OPUS")
        .field("clock-rate", 48_000i32)
        .field("payload", 111i32)
        .field("encoding-params", "2")
        .field("stereo", "1")
        .field("useinbandfec", "1")
        .build()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Codec {
    Vp8,
    H264,
}

impl TryFrom<VideoCodec> for Codec {
    type Error = MediaFatalError;

    fn try_from(value: VideoCodec) -> Result<Self, Self::Error> {
        match value {
            VideoCodec::Vp8 => Ok(Self::Vp8),
            VideoCodec::H264 => Ok(Self::H264),
            _ => Err(MediaFatalError::UnsupportedCodec),
        }
    }
}

impl Codec {
    fn encoding(self) -> &'static str {
        match self {
            Self::Vp8 => "VP8",
            Self::H264 => "H264",
        }
    }

    pub fn caps(self) -> gst::Caps {
        let builder = gst::Caps::builder("application/x-rtp")
            .field("media", "video")
            .field("encoding-name", self.encoding())
            .field("clock-rate", 90_000i32)
            .field("payload", 96i32);
        match self {
            Self::Vp8 => builder.build(),
            Self::H264 => builder
                .field("packetization-mode", "1")
                .field("profile-level-id", "4d0032")
                .field("level-asymmetry-allowed", "1")
                .build(),
        }
    }

    fn validate_remote(self, sdp: &sdp::SDPMessageRef) -> MediaResult<()> {
        for media in sdp.medias() {
            if media.media() != Some("video") || media.port() == 0 {
                continue;
            }
            let supported = media.formats().filter_map(|pt| pt.parse().ok()).any(|pt| {
                media.caps_from_media(pt).and_then(|caps| {
                    let fields = caps.structure(0)?;
                    let encoding = fields.get::<&str>("encoding-name").ok()?;
                    Some(
                        encoding.eq_ignore_ascii_case(self.encoding())
                            && (self == Self::Vp8
                                || (fields.get::<&str>("packetization-mode").ok() == Some("1")
                                    && fields
                                        .get::<&str>("profile-level-id")
                                        .ok()
                                        .is_some_and(|id| id.eq_ignore_ascii_case("4d0032")))),
                    )
                }) == Some(true)
            });
            if !supported {
                return Err(MediaFatalError::UnsupportedCodec);
            }
        }
        Ok(())
    }
}

pub(crate) fn connect_ice(
    peer: &gst::Element,
    enqueue: impl Fn(MediaReportKind) + Send + Sync + 'static,
) {
    peer.connect_closure(
        "on-ice-candidate",
        false,
        glib::closure!(move |_peer: &gst::Element, index: u32, candidate: String| {
            if !candidate.trim().is_empty() && !loopback(&candidate) {
                enqueue(MediaReportKind::LocalIceCandidate {
                    candidate,
                    sdp_mid: None,
                    sdp_mline_index: Some(i64::from(index)),
                });
            }
        }),
    );
}

fn loopback(candidate: &str) -> bool {
    candidate
        .split_ascii_whitespace()
        .nth(4)
        .and_then(|address| address.parse::<std::net::IpAddr>().ok())
        .is_some_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trickle_ice_checks_mid_and_index_together() {
        gst::init().unwrap();
        let sdp = sdp::SDPMessage::parse_buffer(b"v=0\r\nm=audio 9 RTP/AVP 111\r\na=mid:audio\r\nm=video 9 RTP/AVP 96\r\na=mid:video\r\n").unwrap();
        let mut candidate = Candidate {
            text: String::new(),
            mid: Some("video".into()),
            index: None,
        };
        assert_eq!(candidate_index(&sdp, &candidate), Some(1));
        for index in [0, 2, -1] {
            candidate.index = Some(index);
            assert_eq!(candidate_index(&sdp, &candidate), None);
        }
        candidate.index = Some(1);
        assert_eq!(candidate_index(&sdp, &candidate), Some(1));
    }

    #[test]
    fn local_ice_omits_loopback_candidates() {
        assert!(loopback("candidate:x 1 UDP 1 127.0.0.1 9 typ host"));
        assert!(loopback("candidate:x 1 UDP 1 ::1 9 typ host"));
        assert!(!loopback("candidate:x 1 UDP 1 192.0.2.1 9 typ host"));
    }
}
