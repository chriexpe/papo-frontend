//! Presence of nullable text in partial resource observations. Policy and merging belong to the SDK.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An optional JSON string where omission and explicit null have distinct merge semantics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NullableText {
    /// The response supplied no observation.
    #[default]
    Missing,
    /// The response explicitly cleared the field.
    Null,
    /// The response supplied this text, including the empty string.
    Value(String),
}

impl NullableText {
    /// Whether this field should be omitted from its containing object.
    #[must_use]
    pub const fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }

    /// Returns the observed string if present.
    #[must_use]
    pub fn as_deref(&self) -> Option<&str> {
        match self {
            Self::Value(text) => Some(text),
            Self::Null | Self::Missing => None,
        }
    }
}

impl From<String> for NullableText {
    fn from(text: String) -> Self {
        Self::Value(text)
    }
}

impl From<&str> for NullableText {
    fn from(text: &str) -> Self {
        Self::Value(text.into())
    }
}

impl Serialize for NullableText {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_deref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for NullableText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Option::<String>::deserialize(deserializer)?.map_or(Self::Null, Self::Value))
    }
}
