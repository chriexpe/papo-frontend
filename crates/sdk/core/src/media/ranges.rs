//! Validated partial representations; HTTP range transport and account authorization live elsewhere.

use super::{MediaError, MediaSource};
use serde::{Deserialize, Serialize};

/// Actual encoded byte interval, never a full-object cache grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRange {
    /// First represented byte, inclusive.
    pub start_bytes: u64,
    /// Last represented byte, inclusive.
    pub end_bytes: u64,
    /// Complete object size when supplied by the server.
    pub total_bytes: Option<u64>,
}

pub(super) fn normalize(
    source: &MediaSource,
    response: &mut papo_api::media::MediaResponse,
) -> Result<Option<MediaRange>, MediaError> {
    let MediaSource::Range {
        source: inner,
        start_bytes,
        end_bytes,
    } = source
    else {
        if response.status != 200
            || response
                .headers
                .contains_key(reqwest::header::CONTENT_RANGE)
        {
            return Err(MediaError::InvalidData);
        }
        return Ok(None);
    };
    if start_bytes > end_bytes
        || !matches!(
            inner.as_ref(),
            MediaSource::Blob(_) | MediaSource::Attachment(_) | MediaSource::EmbedVideo(_)
        )
    {
        return Err(MediaError::InvalidData);
    }
    if response.status == 200 {
        if response
            .headers
            .contains_key(reqwest::header::CONTENT_RANGE)
        {
            return Err(MediaError::InvalidData);
        }
        let total_bytes = response.bytes.len() as u64;
        if *start_bytes >= total_bytes {
            return Err(MediaError::InvalidData);
        }
        let last = (*end_bytes).min(total_bytes - 1);
        response
            .bytes
            .truncate(usize::try_from(last + 1).map_err(|_| MediaError::InvalidData)?);
        response
            .bytes
            .drain(..usize::try_from(*start_bytes).map_err(|_| MediaError::InvalidData)?);
        return Ok(Some(MediaRange {
            start_bytes: *start_bytes,
            end_bytes: last,
            total_bytes: Some(total_bytes),
        }));
    }
    if response.status != 206 {
        return Err(MediaError::InvalidData);
    }
    let header = response
        .headers
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|h| h.to_str().ok())
        .ok_or(MediaError::InvalidData)?;
    let (span, total) = header
        .strip_prefix("bytes ")
        .and_then(|h| h.split_once('/'))
        .ok_or(MediaError::InvalidData)?;
    let (start, end) = span.split_once('-').ok_or(MediaError::InvalidData)?;
    let parse = |text: &str| text.parse::<u64>().map_err(|_| MediaError::InvalidData);
    let start = parse(start)?;
    let end = parse(end)?;
    let total = if total == "*" {
        None
    } else {
        Some(parse(total)?)
    };
    if start != *start_bytes
        || end < start
        || end > *end_bytes
        || end.checked_sub(start).and_then(|n| n.checked_add(1))
            != Some(response.bytes.len() as u64)
        || total.is_some_and(|total| end >= total)
        || (end < *end_bytes && total != end.checked_add(1))
    {
        return Err(MediaError::InvalidData);
    }
    Ok(Some(MediaRange {
        start_bytes: start,
        end_bytes: end,
        total_bytes: total,
    }))
}

#[cfg(test)]
mod tests {
    //! Pure response validation proves partial bytes never become full-object refs.
    use super::*;
    fn response(status: u16, header: Option<&str>) -> papo_api::media::MediaResponse {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(value) = header {
            headers.insert(reqwest::header::CONTENT_RANGE, value.parse().unwrap());
        }
        papo_api::media::MediaResponse {
            status,
            headers,
            bytes: vec![0; 3],
        }
    }
    #[test]
    fn malformed_and_unrequested_partial_representations_are_rejected() {
        let source = MediaSource::Range {
            source: Box::new(MediaSource::Attachment(crate::Uuid::from_u128(1))),
            start_bytes: 2,
            end_bytes: 4,
        };
        assert_eq!(
            normalize(&source, &mut response(206, Some("bytes 2-4/10")))
                .unwrap()
                .unwrap()
                .start_bytes,
            2
        );
        for header in [
            None,
            Some("bytes 0-2/10"),
            Some("bytes 2-5/10"),
            Some("bytes 2-4/4"),
        ] {
            assert!(normalize(&source, &mut response(206, header)).is_err());
        }
        assert!(
            normalize(
                &MediaSource::Attachment(crate::Uuid::from_u128(1)),
                &mut response(206, Some("bytes 2-4/10"))
            )
            .is_err()
        );
    }
    #[test]
    fn ignored_ranges_are_sliced_and_never_stored_as_a_full_response() {
        let source = MediaSource::Range {
            source: Box::new(MediaSource::Attachment(crate::Uuid::from_u128(1))),
            start_bytes: 1,
            end_bytes: 2,
        };
        let mut full = response(200, None);
        let range = normalize(&source, &mut full).unwrap().unwrap();
        assert_eq!(full.bytes.len(), 2);
        assert_eq!(range.total_bytes, Some(3));
    }
}
