//! Validates acknowledged message identities and exact timestamp shape; never derives acknowledgements from history.

use super::persistence::Outcome;
use crate::{Message, Uuid};

pub(super) fn message(
    message: Message,
    channel: Uuid,
    author: Uuid,
    edited_target: Option<Uuid>,
) -> Result<Outcome, papo_api::Error> {
    let parse = |value: Option<&str>| {
        value
            .and_then(|id| id.parse::<Uuid>().ok())
            .ok_or(papo_api::Error::Decode)
    };
    let id = parse(message.id.as_deref())?;
    if parse(message.channel_id.as_deref())? != channel
        || parse(message.author_id.as_deref())? != author
        || edited_target.is_some_and(|target| target != id)
    {
        return Err(papo_api::Error::Decode);
    }
    for value in std::iter::once(
        message
            .created_at
            .as_deref()
            .ok_or(papo_api::Error::Decode)?,
    )
    .chain(message.edited_at.as_deref())
    {
        let time =
            chrono::DateTime::parse_from_rfc3339(value).map_err(|_| papo_api::Error::Decode)?;
        if time.timestamp_subsec_nanos() % 1000 != 0 {
            return Err(papo_api::Error::Decode);
        }
    }
    Ok(Outcome {
        message_id: Some(id),
        message: Some(message),
    })
}

#[cfg(test)]
mod tests {
    //! All values are synthetic; validation does not access a clock or socket.
    use super::*;
    fn value() -> Message {
        Message {
            id: Some(Uuid::from_u128(1).to_string()),
            channel_id: Some(Uuid::from_u128(2).to_string()),
            author_id: Some(Uuid::from_u128(3).to_string()),
            created_at: Some("2026-01-01T00:00:00.123456Z".into()),
            ..Default::default()
        }
    }
    #[test]
    fn only_same_account_channel_target_and_microsecond_receipts_confirm() {
        let channel = Uuid::from_u128(2);
        let author = Uuid::from_u128(3);
        let confirmed = message(value(), channel, author, None).unwrap();
        assert_eq!(confirmed.message_id, Some(Uuid::from_u128(1)));
        assert!(message(value(), Uuid::from_u128(4), author, None).is_err());
        assert!(message(value(), channel, Uuid::from_u128(4), None).is_err());
        assert!(message(value(), channel, author, Some(Uuid::from_u128(4))).is_err());
        for timestamp in [
            None,
            Some("invalid"),
            Some("2026-01-01T00:00:00.123456789Z"),
        ] {
            let mut invalid = value();
            invalid.created_at = timestamp.map(str::to_owned);
            assert!(message(invalid, channel, author, None).is_err());
        }
    }
}
