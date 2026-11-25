//! Where the server's moderation of an attachment stands, read from its `moderation_status`,
//! so every client hides the same attachments. Owns only the reading; how a hidden attachment
//! looks and whether the reader revealed it are the client's.

use papo_sdk::Attachment;

/// The server's moderation of an attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Moderation {
    /// Not checked yet (waiting or being checked).
    Pending,
    /// Checked: nothing found.
    Clean,
    /// Checked: shown only once the reader asks (as a spoiler).
    Sensitive,
    /// Checked: the server no longer serves it.
    Blocked,
    /// The check gave up; the server still serves it.
    Failed,
    /// No status, or one this SDK does not know.
    Unknown,
}

impl Moderation {
    /// The moderation of a received attachment.
    #[must_use]
    pub fn of(attachment: &Attachment) -> Moderation {
        Moderation::from_wire(attachment.moderation_status.as_deref())
    }

    /// The moderation a `moderation_status` value names.
    #[must_use]
    pub fn from_wire(status: Option<&str>) -> Moderation {
        match status {
            Some("pending" | "processing") => Moderation::Pending,
            Some("clean") => Moderation::Clean,
            Some("sensitive") => Moderation::Sensitive,
            Some("blocked") => Moderation::Blocked,
            Some("failed") => Moderation::Failed,
            _ => Moderation::Unknown,
        }
    }

    /// Whether the attachment stays hidden until the reader reveals it.
    #[must_use]
    pub fn hides(self) -> bool {
        self == Moderation::Sensitive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_server_status_reads_as_its_moderation() {
        let cases = [
            (Some("pending"), Moderation::Pending),
            (Some("processing"), Moderation::Pending),
            (Some("clean"), Moderation::Clean),
            (Some("sensitive"), Moderation::Sensitive),
            (Some("blocked"), Moderation::Blocked),
            (Some("failed"), Moderation::Failed),
            (Some("quarantined"), Moderation::Unknown),
            (None, Moderation::Unknown),
        ];
        for (status, moderation) in cases {
            assert_eq!(Moderation::from_wire(status), moderation, "{status:?}");
        }
    }

    #[test]
    fn only_a_sensitive_attachment_hides() {
        assert!(Moderation::Sensitive.hides());
        for moderation in [
            Moderation::Pending,
            Moderation::Clean,
            Moderation::Blocked,
            Moderation::Failed,
            Moderation::Unknown,
        ] {
            assert!(!moderation.hides(), "{moderation:?}");
        }
    }

    #[test]
    fn it_reads_a_received_attachment() {
        let attachment: Attachment =
            serde_json::from_value(serde_json::json!({ "moderation_status": "sensitive" }))
                .unwrap();
        assert_eq!(Moderation::of(&attachment), Moderation::Sensitive);
    }
}
