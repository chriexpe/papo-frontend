//! Incremental multipart form-data parser with bounded rolling buffers and SHA-256 file hashing.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::{ReadUploadError, ReceivedAttachment, ReceivedUpload};

const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_FIELD_BYTES: usize = 128 * 1024;
const MAX_PARTS: usize = 16;
const MAX_FILES: usize = 10;

enum ParseState {
    Preamble,
    Headers,
    FieldBody {
        name: String,
        value: Vec<u8>,
    },
    FileBody {
        filename: String,
        hasher: Sha256,
        size_bytes: u64,
    },
    AfterBoundary,
    Finished,
}

pub(super) struct MultipartParser {
    prefix: Vec<u8>,
    delimiter: Vec<u8>,
    buf: Vec<u8>,
    state: ParseState,
    parts_seen: usize,
    total_field_bytes: usize,
    fields: BTreeMap<String, String>,
    files: Vec<ReceivedAttachment>,
}

impl MultipartParser {
    pub(super) fn new(content_type: &str) -> Option<Self> {
        let boundary = content_type
            .split(';')
            .map(str::trim)
            .find_map(|part| part.strip_prefix("boundary="))?
            .trim_matches('"');
        if boundary.is_empty() || boundary.len() > 200 {
            return None;
        }
        Some(Self {
            prefix: format!("--{boundary}\r\n").into_bytes(),
            delimiter: format!("\r\n--{boundary}").into_bytes(),
            buf: Vec::new(),
            state: ParseState::Preamble,
            parts_seen: 0,
            total_field_bytes: 0,
            fields: BTreeMap::new(),
            files: Vec::new(),
        })
    }

    pub(super) fn push(&mut self, chunk: &[u8]) -> Result<(), ReadUploadError> {
        self.buf.extend_from_slice(chunk);
        loop {
            let progressed = match self.state {
                ParseState::Preamble => self.step_preamble()?,
                ParseState::Headers => self.step_headers()?,
                ParseState::FieldBody { .. } => self.step_field_body()?,
                ParseState::FileBody { .. } => self.step_file_body()?,
                ParseState::AfterBoundary => self.step_after_boundary()?,
                ParseState::Finished => {
                    if self.buf.len() > 64 {
                        return Err(ReadUploadError::Malformed);
                    }
                    false
                }
            };
            if !progressed {
                return Ok(());
            }
        }
    }

    fn step_preamble(&mut self) -> Result<bool, ReadUploadError> {
        if self.buf.len() < self.prefix.len() {
            return Ok(false);
        }
        if !self.buf.starts_with(&self.prefix) {
            return Err(ReadUploadError::Malformed);
        }
        self.buf.drain(..self.prefix.len());
        self.state = ParseState::Headers;
        Ok(true)
    }

    fn step_headers(&mut self) -> Result<bool, ReadUploadError> {
        let Some(end) = find_subslice(&self.buf, b"\r\n\r\n") else {
            if self.buf.len() > MAX_HEADER_BYTES {
                return Err(ReadUploadError::Malformed);
            }
            return Ok(false);
        };
        if self.parts_seen >= MAX_PARTS {
            return Err(ReadUploadError::Malformed);
        }
        self.parts_seen += 1;
        let headers =
            std::str::from_utf8(&self.buf[..end]).map_err(|_| ReadUploadError::Malformed)?;
        let (name, filename) = parse_disposition(headers).ok_or(ReadUploadError::Malformed)?;
        self.buf.drain(..end + 4);
        if let Some(filename) = filename {
            if name != "attachments" || self.files.len() >= MAX_FILES {
                return Err(ReadUploadError::Malformed);
            }
            self.state = ParseState::FileBody {
                filename,
                hasher: Sha256::new(),
                size_bytes: 0,
            };
        } else {
            self.state = ParseState::FieldBody {
                name,
                value: Vec::new(),
            };
        }
        Ok(true)
    }

    fn step_field_body(&mut self) -> Result<bool, ReadUploadError> {
        if let Some(pos) = find_subslice(&self.buf, &self.delimiter) {
            let tail = self.buf[..pos].to_vec();
            self.buf.drain(..pos + self.delimiter.len());
            let ParseState::FieldBody { name, mut value } =
                std::mem::replace(&mut self.state, ParseState::AfterBoundary)
            else {
                return Err(ReadUploadError::Malformed);
            };
            self.add_field_bytes(tail.len())?;
            value.extend_from_slice(&tail);
            let text = String::from_utf8(value).map_err(|_| ReadUploadError::Malformed)?;
            self.fields.insert(name, text);
            return Ok(true);
        }
        let keep = self.delimiter.len().saturating_sub(1);
        if self.buf.len() > keep {
            let flush = self.buf.len() - keep;
            self.add_field_bytes(flush)?;
            if let ParseState::FieldBody { value, .. } = &mut self.state {
                value.extend_from_slice(&self.buf[..flush]);
            }
            self.buf.drain(..flush);
        }
        Ok(false)
    }

    fn add_field_bytes(&mut self, len: usize) -> Result<(), ReadUploadError> {
        self.total_field_bytes = self
            .total_field_bytes
            .checked_add(len)
            .ok_or(ReadUploadError::TooLarge)?;
        if self.total_field_bytes > MAX_FIELD_BYTES {
            return Err(ReadUploadError::TooLarge);
        }
        Ok(())
    }

    fn step_file_body(&mut self) -> Result<bool, ReadUploadError> {
        if let Some(pos) = find_subslice(&self.buf, &self.delimiter) {
            let ParseState::FileBody {
                filename,
                mut hasher,
                mut size_bytes,
            } = std::mem::replace(&mut self.state, ParseState::AfterBoundary)
            else {
                return Err(ReadUploadError::Malformed);
            };
            hasher.update(&self.buf[..pos]);
            size_bytes = size_bytes
                .checked_add(pos as u64)
                .ok_or(ReadUploadError::TooLarge)?;
            self.buf.drain(..pos + self.delimiter.len());
            self.files.push(ReceivedAttachment {
                filename,
                size_bytes,
                sha256: hasher.finalize().into(),
            });
            return Ok(true);
        }
        let keep = self.delimiter.len().saturating_sub(1);
        if self.buf.len() > keep {
            let flush = self.buf.len() - keep;
            if let ParseState::FileBody {
                hasher, size_bytes, ..
            } = &mut self.state
            {
                hasher.update(&self.buf[..flush]);
                *size_bytes = size_bytes
                    .checked_add(flush as u64)
                    .ok_or(ReadUploadError::TooLarge)?;
            }
            self.buf.drain(..flush);
        }
        Ok(false)
    }

    fn step_after_boundary(&mut self) -> Result<bool, ReadUploadError> {
        if self.buf.len() < 2 {
            return Ok(false);
        }
        if self.buf.starts_with(b"--") {
            self.buf.drain(..2);
            self.state = ParseState::Finished;
            Ok(true)
        } else if self.buf.starts_with(b"\r\n") {
            self.buf.drain(..2);
            self.state = ParseState::Headers;
            Ok(true)
        } else {
            Err(ReadUploadError::Malformed)
        }
    }

    pub(super) fn finish(self) -> Result<ReceivedUpload, ReadUploadError> {
        if !matches!(self.state, ParseState::Finished) {
            return Err(ReadUploadError::Malformed);
        }
        Ok(ReceivedUpload {
            fields: self.fields,
            files: self.files,
        })
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let first = needle[0];
    let mut offset = 0;
    while let Some(pos) = haystack[offset..].iter().position(|&b| b == first) {
        let index = offset + pos;
        if index + needle.len() > haystack.len() {
            return None;
        }
        if &haystack[index..index + needle.len()] == needle {
            return Some(index);
        }
        offset = index + 1;
    }
    None
}

fn parse_disposition(headers: &str) -> Option<(String, Option<String>)> {
    let disposition = headers.lines().find(|line| {
        line.to_ascii_lowercase()
            .starts_with("content-disposition:")
    })?;
    let attr = |key: &str| {
        disposition
            .split(';')
            .map(str::trim)
            .find_map(|part| part.strip_prefix(key))
            .and_then(|part| part.strip_prefix('"'))
            .and_then(|part| part.strip_suffix('"'))
            .map(str::to_owned)
    };
    let name = attr("name=")?;
    let filename = attr("filename=");
    Some((name, filename))
}
