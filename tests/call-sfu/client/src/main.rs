//! Loopback real-media proof. Signalling is typed and intentionally lives outside papo-call.

mod endpoint;
#[cfg(feature = "rune-proof")]
mod rune_proof;

use endpoint::Endpoint;
use papo_api::{
    RestClient,
    wire::{CreateChannelRequest, CreateServerRequest, LoginRequest, RegisterRequest},
};
use papo_sdk::{MediaCommand, PlaybackSettings, Uuid, VideoCodec, VideoKind};
use std::{error::Error, time::Duration};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

const FIXTURE_CHANNEL: &str = "00000000-0000-4000-8000-000000000010";
const FIXTURE_RED: &str = "00000000-0000-4000-8000-000000000101";
const FIXTURE_GREEN: &str = "00000000-0000-4000-8000-000000000102";
const FIXTURE_BLUE: &str = "00000000-0000-4000-8000-000000000103";

async fn login(address: &str, name: &str, password: &str, register: bool) -> Result<RestClient> {
    let rest = RestClient::loopback(address)?;
    if register {
        rest.register(&RegisterRequest {
            username: name.into(),
            password: password.into(),
        })
        .await?;
    }
    rest.login(&LoginRequest {
        username: name.into(),
        password: password.into(),
    })
    .await?;
    Ok(rest)
}

fn frame_chroma(
    endpoint: &Endpoint,
    publisher: Uuid,
    kind: VideoKind,
    expected: (u8, u8),
) -> Result<bool> {
    let Some(frame) = endpoint
        .video
        .get(&(publisher, kind))
        .and_then(|dest| dest.take())
    else {
        return Ok(false);
    };
    let row = usize::try_from(frame.height_px / 4)?;
    let stride = usize::try_from(frame.strides[1])?;
    let offset = frame.offsets[1] + row * stride + 20;
    let pair = frame
        .bytes
        .get(offset..offset + 2)
        .ok_or("invalid NV12 frame layout")?;
    if pair[0].abs_diff(expected.0) > 35 || pair[1].abs_diff(expected.1) > 35 {
        return Err(format!(
            "presented frame has the wrong chroma: expected {expected:?}, found {pair:?}"
        )
        .into());
    }
    Ok(true)
}

fn tone(samples: &[i16], frequency_hz: f64) -> f64 {
    let coefficient = 2.0 * (std::f64::consts::TAU * frequency_hz / 48_000.0).cos();
    let (mut one, mut two) = (0.0, 0.0);
    for &sample in samples {
        let next = f64::from(sample) + coefficient * one - two;
        two = one;
        one = next;
    }
    (one * one + two * two - coefficient * one * two)
        .max(0.0)
        .sqrt()
        / f64::from(u32::try_from(samples.len()).unwrap_or(1))
}

async fn fixture(address: &str) -> Result {
    let rest = login(address, "fixture", "", false).await?;
    let mut client = Endpoint::join(&rest, FIXTURE_CHANNEL, 660.0, "red").await?;
    while !client.connected {
        client.step().await?;
    }
    client.camera_on().await?;
    client.mute(false).await?;
    let red = FIXTURE_RED.parse()?;
    let green = FIXTURE_GREEN.parse()?;
    let blue = FIXTURE_BLUE.parse()?;
    client.interest(red, VideoKind::Camera, true).await?;
    client.interest(green, VideoKind::Camera, true).await?;
    client.interest(blue, VideoKind::Screen, true).await?;
    let targets = [
        (red, VideoKind::Camera, (90, 240)),
        (green, VideoKind::Camera, (91, 81)),
        (blue, VideoKind::Screen, (240, 110)),
    ];
    let mut seen = [false; 3];
    let mut pcm_ok = false;
    while !seen.iter().all(|flag| *flag) || !pcm_ok || client.answer_count < 2 {
        tokio::select! {
            result = client.step() => result?,
            () = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
        for (index, (who, kind, pattern)) in targets.iter().enumerate() {
            seen[index] |= frame_chroma(&client, *who, *kind, *pattern)?;
        }
        while let Ok(samples) = client.pcm.try_recv() {
            pcm_ok |= tone(&samples, 440.0) > 100.0 && tone(&samples, 880.0) > 100.0;
        }
    }
    let first_slot = client.publisher_tracks[&(red, VideoKind::Camera)].0.clone();
    let gain = PlaybackSettings::new(160, false);
    client.lease.send(MediaCommand::SetPlayback {
        user: red,
        playback: gain,
    })?;
    client.interest(red, VideoKind::Camera, false).await?;
    client.interest(blue, VideoKind::Camera, true).await?;
    while client.rejected_subscriptions == 0 {
        client.step().await?;
    }
    client.interest(blue, VideoKind::Camera, true).await?;
    loop {
        tokio::select! {
            result = client.step() => result?,
            () = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
        frame_chroma(&client, red, VideoKind::Camera, (90, 240))?;
        if frame_chroma(&client, blue, VideoKind::Camera, (202, 222))? && client.audio_revision >= 2
        {
            let (track, first_seq) = &client.publisher_tracks[&(blue, VideoKind::Camera)];
            assert_eq!(track, &first_slot);
            assert!(first_seq.is_some());
            break;
        }
    }
    for (settings, threshold) in [
        (PlaybackSettings::new(160, false), 1.35),
        (PlaybackSettings::new(160, true), 0.0),
    ] {
        client.lease.send(MediaCommand::SetPlayback {
            user: red,
            playback: settings,
        })?;
        let mut frames = 0;
        while frames < 25 {
            tokio::select! {
                result = client.step() => result?,
                () = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
            while let Ok(samples) = client.pcm.try_recv() {
                frames += 1;
                if frames >= 20 {
                    let red_power = tone(&samples, 440.0);
                    let green_power = tone(&samples, 880.0);
                    assert!(green_power > 100.0);
                    if settings.muted {
                        assert!(red_power < green_power * 0.12);
                    } else {
                        assert!(red_power > green_power * threshold);
                    }
                }
            }
        }
    }
    client.leave().await?;
    println!(
        "Synthetic SFU: three video destinations, forced RTP slot reuse, rejected subscription, routed PCM gain and mute"
    );
    Ok(())
}

async fn backend_room(
    address: &str,
    password: &str,
) -> Result<(RestClient, RestClient, Uuid, String)> {
    let a = login(address, "publisher", password, true).await?;
    a.create_server(&CreateServerRequest {
        name: "Call proof".into(),
        public: Some(true),
        ..Default::default()
    })
    .await?;
    let b = login(address, "receiver", password, true).await?;
    let owner: Uuid = a
        .whoami()
        .await?
        .id
        .ok_or("user identity missing")?
        .parse()?;
    let channel = a
        .create_channel(&CreateChannelRequest {
            name: "media-proof".into(),
            r#type: Some("voice".into()),
            ..Default::default()
        })
        .await?
        .id
        .ok_or("channel missing")?;
    Ok((a, b, owner, channel))
}

async fn backend(address: &str, password: &str, codec: VideoCodec) -> Result {
    let (a, b, owner, channel) = backend_room(address, password).await?;
    let mut publisher = Endpoint::join(&a, &channel, 520.0, "red").await?;
    let mut receiver = Endpoint::join(&b, &channel, 760.0, "green").await?;
    while !publisher.connected || !receiver.connected {
        drive(&mut publisher, &mut receiver).await?;
    }
    assert_eq!(publisher.codec, Some(codec));
    assert_eq!(receiver.codec, Some(codec));
    publisher.camera_on().await?;
    publisher.mute(false).await?;
    receiver.mute(false).await?;
    while publisher.answer_count < 2 {
        drive(&mut publisher, &mut receiver).await?;
    }
    let mut seen = false;
    let mut hear_publisher = false;
    let mut hear_receiver = false;
    let mut retries = 0;
    let limit = tokio::time::Instant::now() + Duration::from_secs(12);
    while !seen || !hear_publisher || !hear_receiver {
        if tokio::time::Instant::now() > limit {
            return Err(format!(
                "video={seen} hear_publisher={hear_publisher} hear_receiver={hear_receiver}; subscriptions_rejected={} routes_received={}",
                receiver.rejected_subscriptions, receiver.route_revision
            )
            .into());
        }
        if retries == 0 || retries % 50 == 0 && !seen {
            receiver.interest(owner, VideoKind::Camera, true).await?;
        }
        retries += 1;
        drive(&mut publisher, &mut receiver).await?;
        seen |= frame_chroma(&receiver, owner, VideoKind::Camera, (90, 240))?;
        while let Ok(samples) = receiver.pcm.try_recv() {
            hear_publisher |= tone(&samples, 520.0) > 100.0;
        }
        while let Ok(samples) = publisher.pcm.try_recv() {
            hear_receiver |= tone(&samples, 760.0) > 100.0;
        }
    }
    let first_slot = receiver.publisher_tracks[&(owner, VideoKind::Camera)]
        .0
        .clone();
    publisher.leave().await?;
    while receiver
        .publisher_tracks
        .contains_key(&(owner, VideoKind::Camera))
    {
        receiver.step().await?;
    }

    let c = login(address, "replacement", password, true).await?;
    let replacement: Uuid = c
        .whoami()
        .await?
        .id
        .ok_or("user identity missing")?
        .parse()?;
    let mut publisher = Endpoint::join(&c, &channel, 620.0, "blue").await?;
    while !publisher.connected {
        drive(&mut publisher, &mut receiver).await?;
    }
    let previous_rejections = receiver.rejected_subscriptions;
    receiver
        .interest(replacement, VideoKind::Camera, true)
        .await?;
    while receiver.rejected_subscriptions == previous_rejections {
        drive(&mut publisher, &mut receiver).await?;
    }
    publisher.camera_on().await?;
    while publisher.answer_count < 2 {
        drive(&mut publisher, &mut receiver).await?;
    }

    let mut retry_at = tokio::time::Instant::now();
    loop {
        if tokio::time::Instant::now() >= retry_at
            && !receiver
                .publisher_tracks
                .contains_key(&(replacement, VideoKind::Camera))
        {
            receiver
                .interest(replacement, VideoKind::Camera, true)
                .await?;
            retry_at = tokio::time::Instant::now() + Duration::from_millis(500);
        }
        drive(&mut publisher, &mut receiver).await?;
        assert!(receiver.video[&(owner, VideoKind::Camera)].take().is_none());
        if frame_chroma(&receiver, replacement, VideoKind::Camera, (240, 110))? {
            let (track, first_seq) = &receiver.publisher_tracks[&(replacement, VideoKind::Camera)];
            assert_eq!(track, &first_slot);
            assert!(first_seq.is_some());
            break;
        }
    }
    receiver.leave().await?;
    publisher.leave().await?;
    println!(
        "Pinned backend: {codec:?} negotiated, duplex PCM, publisher churn, rejected subscription, routed slot reuse and drain passed"
    );
    Ok(())
}

async fn drive(a: &mut Endpoint, b: &mut Endpoint) -> Result {
    tokio::select! {
        result = a.step() => result?,
        result = b.step() => result?,
        () = tokio::time::sleep(Duration::from_millis(20)) => {}
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn plateau(address: &str) -> Result {
    fn count(path: &str) -> Result<usize> {
        Ok(std::fs::read_dir(path)?.count())
    }
    let rest = login(address, "fixture", "", false).await?;
    let mut gateway = papo_api::gateway::Gateway::connect(&rest).await?;
    let mut samples = Vec::new();
    for cycle in 0..110 {
        let mut client = Endpoint::rejoin(gateway, FIXTURE_CHANNEL, 660.0, "red").await?;
        while !client.connected {
            client.step().await?;
        }
        client.mute(false).await?;
        client.camera_on().await?;
        while client.answer_count < 2 {
            client.step().await?;
        }
        gateway = client.leave_open().await?;
        if cycle >= 10 {
            samples.push((count("/proc/self/task")?, count("/proc/self/fd")?));
        }
    }
    gateway.close().await?;
    let (first_threads, first_fds) = samples[0];
    let (last_threads, last_fds) = samples[samples.len() - 1];
    let max_threads = samples
        .iter()
        .map(|sample| sample.0)
        .max()
        .ok_or("no samples")?;
    let max_fds = samples
        .iter()
        .map(|sample| sample.1)
        .max()
        .ok_or("no samples")?;
    println!(
        "100 joined, negotiated and drained leases: threads {first_threads} → {last_threads} (peak {max_threads}), FDs {first_fds} → {last_fds} (peak {max_fds})"
    );
    assert!(last_threads <= first_threads + 2 && last_fds <= first_fds + 2);
    assert!(max_threads <= first_threads + 6 && max_fds <= first_fds + 6);
    Ok(())
}

struct CodecLog;
impl log::Log for CodecLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().starts_with("papo_call")
    }
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{}", record.args());
        }
    }
    fn flush(&self) {}
}

#[tokio::main]
async fn main() -> Result {
    log::set_logger(&CodecLog).map_err(|_| "logger already installed")?;
    log::set_max_level(log::LevelFilter::Info);
    let address = std::env::var("PAPO_CALL_PROOF_ADDRESS")?;
    #[cfg(feature = "rune-proof")]
    if std::env::var_os("PAPO_CALL_PROOF_RUNE").is_some() {
        return rune_proof::run(address, std::env::var("PAPO_PROOF_PASSWORD")?);
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("PAPO_CALL_PROOF_PLATEAU").is_some() {
        return tokio::time::timeout(Duration::from_secs(150), plateau(&address)).await?;
    }
    let exercise = async {
        match std::env::var("PAPO_CALL_PROOF_CODEC") {
            Ok(raw) => {
                backend(
                    &address,
                    &std::env::var("PAPO_PROOF_PASSWORD")?,
                    VideoCodec::from_wire(&raw),
                )
                .await
            }
            Err(_) => fixture(&address).await,
        }
    };
    tokio::time::timeout(Duration::from_secs(45), exercise).await?
}
