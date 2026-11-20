//! Streaming boundary chunking tests; no sockets or clocks are used.

use super::*;
use sha2::{Digest, Sha256};

#[test]
fn streaming_parser_matches_across_arbitrary_frame_splits() {
    let body = b"--b123\r\nContent-Disposition: form-data; name=\"channel_id\"\r\n\r\nch1\r\n--b123\r\nContent-Disposition: form-data; name=\"attachments\"; filename=\"one.bin\"\r\n\r\n\0\xff\r\n--fake\r\n--b123\r\nContent-Disposition: form-data; name=\"attachments\"; filename=\"empty.bin\"\r\n\r\n\r\n--b123--\r\n";
    for chunk_size in [1, 2, 3, 5, 7, 13, 64, body.len()] {
        let mut parser = MultipartParser::new("multipart/form-data; boundary=b123").unwrap();
        for chunk in body.chunks(chunk_size) {
            parser.push(chunk).unwrap();
        }
        let upload = parser.finish().unwrap();
        assert_eq!(
            upload.fields.get("channel_id").map(String::as_str),
            Some("ch1")
        );
        assert_eq!(upload.files.len(), 2);
        assert_eq!(upload.files[0].filename, "one.bin");
        assert_eq!(upload.files[0].size_bytes, 10);
        let expected_hash: [u8; 32] = Sha256::digest(b"\0\xff\r\n--fake").into();
        assert_eq!(upload.files[0].sha256, expected_hash);
        assert_eq!(upload.files[1].filename, "empty.bin");
        assert_eq!(upload.files[1].size_bytes, 0);
        let empty_hash: [u8; 32] = Sha256::digest(b"").into();
        assert_eq!(upload.files[1].sha256, empty_hash);
    }
}
