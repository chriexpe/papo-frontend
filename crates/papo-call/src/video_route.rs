//! Negotiates the SFU's assignment extension and admits packets before they enter a decoder.

use gstreamer as gst;
use gstreamer_rtp::RTPBuffer;
use gstreamer_sdp as sdp;
use papo_sdk::{VideoRouteFence, VideoTarget};

const URI: &str = "urn:papo:rtp-hdrext:video-route-v1";
const OFFER_ID: u8 = 14;

pub(crate) fn offer_extension(caps: &mut gst::CapsRef) {
    // Only receive transceivers advertise this. The SFU owns outbound route IDs.
    if let Some(fields) = caps.structure_mut(0) {
        fields.set(format!("extmap-{OFFER_ID}"), URI);
    }
}

pub(crate) fn negotiated_extension(media: &sdp::SDPMediaRef) -> Option<u8> {
    media.attributes().find_map(|attribute| {
        if attribute.key() != "extmap" {
            return None;
        }
        let mut parts = attribute.value()?.split_ascii_whitespace();
        let mut specifier = parts.next()?.split('/');
        let id = specifier.next()?.parse::<u8>().ok()?;
        let permits_sfu_send = matches!(specifier.next(), None | Some("sendonly" | "sendrecv"));
        (id == OFFER_ID && permits_sfu_send && parts.next() == Some(URI)).then_some(id)
    })
}

pub(crate) fn gate_decoder_input(pad: &gst::Pad, target: VideoTarget, fence: VideoRouteFence) {
    use gst::prelude::*;

    #[cfg(not(feature = "benchmark-telemetry"))]
    let _ = target;

    // This probe belongs to the branch's sink pad and survives for exactly that pad's life.
    // It is downstream of internal jitter/FEC/RTX buffering, before our queue. A buffer that
    // waited upstream during a route change must match the *new branch's* ID when it arrives.
    let _ = pad.add_probe(
        gst::PadProbeType::BUFFER | gst::PadProbeType::BUFFER_LIST,
        move |_, info| {
            let accepts = |buffer: &gst::BufferRef| accepts_assignment(buffer, fence);
            let accepted = info.buffer().is_some_and(|buffer| accepts(buffer))
                || info
                    .buffer_list()
                    .is_some_and(|list| list.iter().all(accepts));
            #[cfg(feature = "benchmark-telemetry")]
            {
                if accepted {
                    if let Some(buffer) = info.buffer() {
                        record_accepted_packet(buffer, target, fence);
                    } else if let Some(list) = info.buffer_list() {
                        for buffer in list.iter() {
                            record_accepted_packet(buffer, target, fence);
                        }
                    }
                } else {
                    crate::telemetry::record_fence_rejected();
                }
            }
            if accepted {
                gst::PadProbeReturn::Ok
            } else {
                gst::PadProbeReturn::Drop
            }
        },
    );
}

#[cfg(feature = "benchmark-telemetry")]
fn record_accepted_packet(buffer: &gst::BufferRef, target: VideoTarget, fence: VideoRouteFence) {
    if let Ok(packet) = RTPBuffer::from_buffer_readable(buffer) {
        crate::telemetry::record_fence_accepted(
            target,
            fence.assignment_id,
            packet.timestamp(),
            packet.is_marker(),
            buffer.pts().map(gst::ClockTime::nseconds),
        );
    }
}

fn accepts_assignment(buffer: &gst::BufferRef, fence: VideoRouteFence) -> bool {
    let Ok(packet) = RTPBuffer::from_buffer_readable(buffer) else {
        return false;
    };
    let expected = fence.assignment_id.to_be_bytes();
    packet.extension_onebyte_header(fence.extension_id, 0) == Some(expected.as_slice())
        && packet
            .extension_onebyte_header(fence.extension_id, 1)
            .is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gstreamer_rtp::prelude::RTPBufferExt as _;

    fn packet(seq: u16, extension: Option<(u8, &[u8])>) -> gst::Buffer {
        let mut buffer = gst::Buffer::new_rtp_with_sizes(1, 0, 0).unwrap();
        let mut rtp = RTPBuffer::from_buffer_writable(buffer.get_mut().unwrap()).unwrap();
        rtp.set_seq(seq);
        if let Some((id, data)) = extension {
            rtp.add_extension_onebyte_header(id, data).unwrap();
        }
        drop(rtp);
        buffer
    }

    #[test]
    fn only_the_negotiated_sfu_send_extension_arms_the_decoder() {
        gst::init().unwrap();
        for (extmap, expected) in [
            ("", None),
            ("14/recvonly urn:papo:rtp-hdrext:video-route-v1", None),
            ("14/sendonly urn:papo:rtp-hdrext:video-route-v1", Some(14)),
            ("14 urn:other", None),
            ("14 urn:papo:rtp-hdrext:video-route-v1", Some(14)),
        ] {
            let extension = if extmap.is_empty() {
                String::new()
            } else {
                format!("a=extmap:{extmap}\r\n")
            };
            let text = format!("v=0\r\nm=video 9 RTP/AVP 96\r\n{extension}");
            let message = sdp::SDPMessage::parse_buffer(text.as_bytes()).unwrap();
            assert_eq!(negotiated_extension(message.media(0).unwrap()), expected);
        }
    }

    #[test]
    fn slot_reuse_drops_replacement_rtp_before_the_route_and_old_rtp_after_it() {
        gst::init().unwrap();
        let old = VideoRouteFence {
            extension_id: OFFER_ID,
            assignment_id: 1,
        };
        let replacement = VideoRouteFence {
            assignment_id: 2,
            ..old
        };
        let old_packet = packet(u16::MAX, Some((OFFER_ID, &old.assignment_id.to_be_bytes())));
        let replacement_packet = packet(
            0,
            Some((OFFER_ID, &replacement.assignment_id.to_be_bytes())),
        );
        let unmarked = packet(1, None);
        let malformed = packet(2, Some((OFFER_ID, &[2])));
        let another_extension = packet(3, Some((1, &replacement.assignment_id.to_be_bytes())));

        assert!(accepts_assignment(&old_packet, old));
        assert!(!accepts_assignment(&replacement_packet, old));
        assert!(!accepts_assignment(&unmarked, old));
        assert!(!accepts_assignment(&old_packet, replacement));
        assert!(accepts_assignment(&replacement_packet, replacement));
        for packet in [unmarked, malformed, another_extension] {
            assert!(!accepts_assignment(&packet, replacement));
        }
    }
}
