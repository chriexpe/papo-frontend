//! Linux peak-memory comparison proving streamed staging, recovery and upload stay bounded independently of file size.

#![cfg(all(target_os = "linux", feature = "test-support"))]

mod fixture;

use fixture::{Fixture, Result, login, register};
use papo_sdk::{
    DraftBody, OperationKind, OperationState, OutgoingAttachment, OutgoingMessage, RetryDecision,
    UnixMicros, Uuid,
};
use papo_test_server::{Config, Failure, RestStep};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    process::Command,
};

struct PatternReader {
    remaining: u64,
    offset: u64,
    seed: u8,
}

impl PatternReader {
    fn new(len: u64, seed: u8) -> Self {
        Self {
            remaining: len,
            offset: 0,
            seed,
        }
    }
}

impl Read for PatternReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let want = usize::try_from(self.remaining.min(buf.len() as u64)).unwrap_or(buf.len());
        for (i, slot) in buf[..want].iter_mut().enumerate() {
            let pos = self.offset + i as u64;
            *slot = ((pos.wrapping_mul(31) ^ u64::from(self.seed)) & 0xff) as u8;
        }
        self.offset += want as u64;
        self.remaining -= want as u64;
        Ok(want)
    }
}

fn read_status_kib(key: &str) -> u64 {
    let text = std::fs::read_to_string("/proc/self/status").expect("procfs status");
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            return rest
                .split_whitespace()
                .next()
                .expect("kib value")
                .parse()
                .expect("u64 kib");
        }
    }
    panic!("missing {key} in /proc/self/status");
}

#[derive(Debug)]
struct SampleKib {
    stage_hwm: u64,
    recover_rss: u64,
    recover_hwm: u64,
    upload_hwm: u64,
}

fn run_child(size_mib: u64) -> Result<SampleKib> {
    let output = Command::new(std::env::current_exe()?)
        .args([
            "--ignored",
            "--exact",
            "memory_probe_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("PAPO_MEMORY_MIB", size_mib.to_string())
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "memory child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let stdout = String::from_utf8(output.stdout)?;
    let line = stdout
        .lines()
        .find_map(|line| line.find("PAPO_MEM ").map(|index| &line[index..]))
        .ok_or_else(|| {
            format!(
                "missing PAPO_MEM summary line; stdout={stdout:?} stderr={:?}",
                String::from_utf8_lossy(&output.stderr)
            )
        })?;
    let mut parts = line.split_whitespace().skip(1);
    let parse_next = |iter: &mut std::iter::Skip<std::str::SplitWhitespace<'_>>| -> Result<u64> {
        let token = iter.next().ok_or("missing metric token")?;
        let (_, value) = token.split_once('=').ok_or("malformed metric token")?;
        Ok(value.parse()?)
    };
    Ok(SampleKib {
        stage_hwm: parse_next(&mut parts)?,
        recover_rss: parse_next(&mut parts)?,
        recover_hwm: parse_next(&mut parts)?,
        upload_hwm: parse_next(&mut parts)?,
    })
}

#[test]
fn peak_memory_for_100_mib_staging_recovery_and_upload_stays_bounded_versus_1_mib() -> Result {
    let small = run_child(1)?;
    let large = run_child(100)?;
    eprintln!("1 MiB sample:   {small:?}");
    eprintln!("100 MiB sample: {large:?}");

    // File size grew by +99 MiB (101,376 KiB). Incremental working memory across staging,
    // queue recovery and full multipart upload must stay bounded independently of file size.
    let stage_growth_kib = large.stage_hwm.saturating_sub(small.stage_hwm);
    let recover_growth_kib = large.recover_rss.saturating_sub(small.recover_rss);
    let recover_hwm_growth_kib = large.recover_hwm.saturating_sub(small.recover_hwm);
    let upload_growth_kib = large.upload_hwm.saturating_sub(small.upload_hwm);

    assert!(
        stage_growth_kib < 16 * 1024,
        "staging peak memory grew by {stage_growth_kib} KiB between 1 MiB and 100 MiB"
    );
    assert!(
        recover_growth_kib < 16 * 1024,
        "recovery RSS grew by {recover_growth_kib} KiB between 1 MiB and 100 MiB"
    );
    assert!(
        recover_hwm_growth_kib < 16 * 1024,
        "recovery HWM grew by {recover_hwm_growth_kib} KiB between 1 MiB and 100 MiB"
    );
    assert!(
        upload_growth_kib < 16 * 1024,
        "upload peak memory grew by {upload_growth_kib} KiB between 1 MiB and 100 MiB"
    );
    Ok(())
}

fn expected_pattern_sha(size_bytes: u64, seed: u8) -> [u8; 32] {
    let mut hasher = Sha256::new();
    let mut reader = PatternReader::new(size_bytes, seed);
    let mut buf = vec![0_u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hasher.finalize().into()
}

async fn stage_and_enqueue_unknown(
    f: &Fixture,
    local_id: Uuid,
    size_bytes: u64,
    expected_sha: [u8; 32],
) -> (papo_sdk::AccountId, u64) {
    let handle = f.client().handle();
    let server = handle
        .add_server(&f.server().base_url(), UnixMicros(0))
        .await
        .unwrap()
        .id;
    let auth = handle.auth(server).unwrap();
    register(&auth, "memstream").await.unwrap();
    let channel: Uuid = f
        .server()
        .create_channel("memstream")
        .await
        .unwrap()
        .id
        .unwrap()
        .parse()
        .unwrap();
    let session = login(&auth, "memstream").await.unwrap();
    fixture::online(&session).await.unwrap();
    let queue = session.outgoing();

    let staged = queue
        .stage_attachment(PatternReader::new(size_bytes, 0x5a))
        .await
        .unwrap();
    assert_eq!(staged.size_bytes(), size_bytes);
    assert_eq!(staged.digest(), &expected_sha);
    let stage_hwm_kib = read_status_kib("VmHWM:");

    let rev = queue
        .save_draft(
            channel,
            DraftBody {
                message: OutgoingMessage {
                    content: "memstream".into(),
                    attachments: vec![OutgoingAttachment::from_staged("blob.bin", staged)],
                    ..Default::default()
                },
                ..Default::default()
            },
            UnixMicros(1),
            None,
        )
        .await
        .unwrap();
    queue
        .enqueue_draft(channel, local_id, rev, UnixMicros(2))
        .await
        .unwrap();
    let mut rx = queue.subscribe().await.unwrap();
    loop {
        if rx
            .borrow_and_update()
            .operations
            .iter()
            .any(|op| op.id == local_id && op.state == OperationState::Unknown)
        {
            break;
        }
        rx.changed().await.unwrap();
    }
    (session.account(), stage_hwm_kib)
}

async fn recover_and_upload(
    f: &Fixture,
    account: papo_sdk::AccountId,
    local_id: Uuid,
    size_bytes: u64,
    expected_sha: [u8; 32],
) -> (u64, u64, u64) {
    let session = f.client().handle().session(account).unwrap();
    fixture::online(&session).await.unwrap();
    let queue = session.outgoing();
    let mut rx = queue.subscribe().await.unwrap();
    let recovered = loop {
        if let Some(op) = rx
            .borrow_and_update()
            .operations
            .iter()
            .find(|op| op.id == local_id && op.state == OperationState::Unknown)
            .cloned()
        {
            break op;
        }
        rx.changed().await.unwrap();
    };
    let OperationKind::Message(body) = recovered.kind.as_ref() else {
        panic!("expected message");
    };
    assert!(body.attachments[0].inline_bytes().is_none());
    assert_eq!(
        body.attachments[0].staged().unwrap().size_bytes(),
        size_bytes
    );
    let recover_rss_kib = read_status_kib("VmRSS:");
    let recover_hwm_kib = read_status_kib("VmHWM:");

    queue
        .retry(local_id, RetryDecision::AcceptPossibleDuplicate)
        .await
        .unwrap();
    loop {
        if rx
            .borrow_and_update()
            .operations
            .iter()
            .any(|op| op.id == local_id && op.state == OperationState::Sent)
        {
            break;
        }
        rx.changed().await.unwrap();
    }
    let received = f.server().take_received_upload().await.unwrap();
    assert_eq!(received.files.len(), 1);
    assert_eq!(received.files[0].size_bytes, size_bytes);
    assert_eq!(received.files[0].sha256, expected_sha);
    let upload_hwm_kib = read_status_kib("VmHWM:");
    (recover_rss_kib, recover_hwm_kib, upload_hwm_kib)
}

#[test]
#[ignore = "memory probe subprocess, run only by outgoing_memory parent"]
fn memory_probe_child() {
    let size_mib: u64 = std::env::var("PAPO_MEMORY_MIB").unwrap().parse().unwrap();
    let size_bytes = size_mib * 1024 * 1024;
    let expected_sha = expected_pattern_sha(size_bytes, 0x5a);
    let mut f = Fixture::new(Config {
        rest: vec![RestStep {
            method: "POST".into(),
            path: "/messages".into(),
            delay_ms: 0,
            failure: Some(Failure {
                status: 500,
                slug: "internal".into(),
            }),
        }],
        ..Default::default()
    })
    .unwrap();
    let local_id = Uuid::from_u128(0xe000_0000_0000_4000_8000_0000_0000_0001);
    let (account, stage_hwm_kib) = f
        .run(stage_and_enqueue_unknown(
            &f,
            local_id,
            size_bytes,
            expected_sha,
        ))
        .unwrap();
    f.reopen().unwrap();
    let (recover_rss_kib, recover_hwm_kib, upload_hwm_kib) = f
        .run(recover_and_upload(
            &f,
            account,
            local_id,
            size_bytes,
            expected_sha,
        ))
        .unwrap();
    f.finish().unwrap();
    println!(
        "PAPO_MEM stage_hwm={stage_hwm_kib} recover_rss={recover_rss_kib} recover_hwm={recover_hwm_kib} upload_hwm={upload_hwm_kib}"
    );
}
