//! Receive capacity requested in the first offer. The SDK owns subscription demand and the SFU
//! owns slot assignment. Track identity is read from negotiated SDP and the SDK's route snapshots.

/// Number of receive-only transceivers provisioned for each media kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiveSlots {
    /// Maximum simultaneous camera and screen streams requested together.
    pub video: u8,
    /// Maximum simultaneous audio streams requested.
    pub audio: u8,
}

impl Default for ReceiveSlots {
    fn default() -> Self {
        Self { video: 6, audio: 8 }
    }
}

impl ReceiveSlots {
    /// Bounds negotiation and leaves room for microphone and camera publication.
    pub(crate) fn is_supported(self) -> bool {
        u16::from(self.video) + u16::from(self.audio) <= 62
    }
}
