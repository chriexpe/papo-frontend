//! Owns graph construction and rollback. A branch owns its elements and any requested sink pad.

use gst::prelude::*;
use gstreamer as gst;
use papo_sdk::MediaFatalError;

pub(crate) type MediaResult<T> = Result<T, MediaFatalError>;

pub(crate) fn element(name: &str) -> MediaResult<gst::Element> {
    gst::ElementFactory::make(name)
        .build()
        .map_err(|_| MediaFatalError::DeviceError)
}

pub(crate) fn filter(caps: &gst::Caps) -> MediaResult<gst::Element> {
    let filter = element("capsfilter")?;
    filter.set_property("caps", caps);
    Ok(filter)
}

pub(crate) fn queue(max_buffers: u32, max_time_ns: u64) -> MediaResult<gst::Element> {
    let queue = element("queue")?;
    queue.set_property("max-size-buffers", max_buffers);
    queue.set_property("max-size-time", max_time_ns);
    queue.set_property("max-size-bytes", 0u32);
    queue.set_property_from_str("leaky", "downstream");
    Ok(queue)
}

pub(crate) fn pad(element: &gst::Element, name: &str) -> MediaResult<gst::Pad> {
    element.static_pad(name).ok_or(MediaFatalError::Internal)
}

pub(crate) struct RequestedPad {
    owner: gst::Element,
    pub pad: gst::Pad,
}

impl RequestedPad {
    pub fn new(owner: &gst::Element, template: &str) -> MediaResult<Self> {
        Ok(Self {
            pad: owner
                .request_pad_simple(template)
                .ok_or(MediaFatalError::DeviceError)?,
            owner: owner.clone(),
        })
    }
}

impl Drop for RequestedPad {
    fn drop(&mut self) {
        self.owner.release_request_pad(&self.pad);
    }
}

pub(crate) struct Branch {
    graph: gst::Pipeline,
    elements: Vec<gst::Element>,
    inlet: Option<(gst::Pad, gst::Pad)>,
    outlet: Option<(gst::Pad, gst::Pad)>,
    _requested: Option<RequestedPad>,
}

impl Branch {
    pub fn owns(&self, source: &gst::Object) -> bool {
        self.elements.iter().any(|element| {
            source == element.upcast_ref::<gst::Object>() || source.has_as_ancestor(element)
        })
    }

    pub fn attach(
        graph: &gst::Pipeline,
        elements: Vec<gst::Element>,
        source: Option<&gst::Pad>,
        target: Option<RequestedPad>,
    ) -> MediaResult<Self> {
        let mut branch = Self {
            graph: graph.clone(),
            elements: Vec::with_capacity(elements.len()),
            inlet: None,
            outlet: None,
            _requested: target,
        };
        for element in elements {
            graph
                .add(&element)
                .map_err(|_| MediaFatalError::DeviceError)?;
            branch.elements.push(element);
        }
        gst::Element::link_many(&branch.elements).map_err(|_| MediaFatalError::DeviceError)?;
        if let Some((target, last)) = branch._requested.as_ref().zip(branch.elements.last()) {
            let src = pad(last, "src")?;
            src.link(&target.pad)
                .map_err(|_| MediaFatalError::DeviceError)?;
            branch.outlet = Some((src, target.pad.clone()));
        }
        if let Some((source, first)) = source.zip(branch.elements.first()) {
            let sink = pad(first, "sink")?;
            source
                .link(&sink)
                .map_err(|_| MediaFatalError::DeviceError)?;
            branch.inlet = Some((source.clone(), sink));
        }
        for element in branch.elements.iter().rev() {
            element
                .sync_state_with_parent()
                .map_err(|_| MediaFatalError::DeviceError)?;
        }
        Ok(branch)
    }
}

impl Drop for Branch {
    fn drop(&mut self) {
        if let Some((src, sink)) = self.inlet.take() {
            let _ = src.unlink(&sink);
        }
        for element in &self.elements {
            let _ = element.set_state(gst::State::Null);
        }
        if let Some((src, sink)) = self.outlet.take() {
            let _ = src.unlink(&sink);
        }
        for element in self.elements.iter().rev() {
            let _ = self.graph.remove(element);
        }
    }
}

pub(crate) fn ready_codec(name: &str, direction: &str, caps: &gst::Caps) -> Option<gst::Element> {
    let codec = element(name).ok()?;
    let ready = codec.set_state(gst::State::Ready).is_ok();
    let usable = ready
        && codec
            .static_pad(direction)
            .is_some_and(|pad| pad.query_caps(Some(caps)).can_intersect(caps));
    let _ = codec.set_state(gst::State::Null);
    usable.then_some(codec)
}

pub(crate) struct PadProbe {
    pad: gst::Pad,
    id: Option<gst::PadProbeId>,
}

impl PadProbe {
    pub fn block(pad: &gst::Pad) -> Self {
        let id = pad.add_probe(
            gst::PadProbeType::BLOCK | gst::PadProbeType::BUFFER | gst::PadProbeType::BUFFER_LIST,
            |_, _| gst::PadProbeReturn::Ok,
        );
        Self {
            pad: pad.clone(),
            id,
        }
    }

    pub async fn idle(pad: &gst::Pad) -> MediaResult<Self> {
        let (send, ready) = tokio::sync::oneshot::channel();
        let send = std::sync::Mutex::new(Some(send));
        let id = pad.add_probe(gst::PadProbeType::IDLE, move |_, _| {
            let sender = send
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(sender) = sender {
                let _ = sender.send(());
            }
            gst::PadProbeReturn::Ok
        });
        let probe = Self {
            pad: pad.clone(),
            id,
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), ready)
            .await
            .map_err(|_| MediaFatalError::DeviceError)?
            .map_err(|_| MediaFatalError::DeviceError)?;
        Ok(probe)
    }
}

impl Drop for PadProbe {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.pad.remove_probe(id);
        }
    }
}
