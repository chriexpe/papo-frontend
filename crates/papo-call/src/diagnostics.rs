//! Media diagnostics use typed stages and factory names. GStreamer error strings and structures
//! can contain configuration or wire payloads and never cross this boundary.

use gst::prelude::*;
use gstreamer as gst;
use papo_sdk::{MediaFatalError, MediaStartError};

pub(crate) fn initialize() -> Result<(), MediaStartError> {
    gst::init().map_err(|_| MediaStartError::DeviceUnavailable)?;
    crate::platform::initialize_platform();
    for plugin in [
        "webrtcbin",
        "nicesrc",
        "audiomixer",
        "opusenc",
        "opusdec",
        "rtpopuspay",
        "rtpopusdepay",
        "level",
        "rtphdrextclientaudiolevel",
    ] {
        if gst::ElementFactory::find(plugin).is_none() {
            log::warn!("Required media element unavailable: {plugin}");
            return Err(MediaStartError::DeviceUnavailable);
        }
    }
    Ok(())
}

pub(crate) fn codec_selected(element: &gst::Element, direction: &str) {
    if let Some(factory) = element.factory() {
        log::info!("Call video {direction}: {}", factory.name());
    }
}

pub(crate) fn fatal(error: MediaFatalError) {
    log::warn!("Call media stopped: {error:?}");
}

pub(crate) fn graph_error(message: &gst::message::Error) {
    let factory = message
        .src()
        .and_then(|source| source.downcast_ref::<gst::Element>())
        .and_then(gst::Element::factory)
        .map(|factory| factory.name());
    let error = message.error();
    let flow = message.debug().and_then(|debug| {
        ["not-negotiated", "not-linked", "flushing"]
            .into_iter()
            .find(|kind| debug.contains(kind))
    });
    log::warn!(
        "Media graph error: factory={factory:?}, domain={:?}, stream={:?}, resource={:?}, flow={flow:?}",
        error.domain(),
        error.kind::<gst::StreamError>(),
        error.kind::<gst::ResourceError>()
    );
}
