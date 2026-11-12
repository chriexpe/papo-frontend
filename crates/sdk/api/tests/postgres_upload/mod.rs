//! Proves multipart embed references against production handlers; leaves cursor seeding to the parent.

use papo_api::{
    Error, HistoryQuery, MessageAttachment, MessageUpload, RestClient, Uuid,
    wire::{CreateChannelRequest, EmbedInput, EmbedMediaInput},
};

pub(super) async fn verify(client: &RestClient) {
    let channel = client
        .create_channel(&CreateChannelRequest {
            name: "upload-proof".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .id
        .unwrap();
    let files = [MessageAttachment {
        filename: "SPOILER_pixel.png",
        bytes: PIXEL,
    }];
    let embeds = encode(media(None, Some("SPOILER_pixel.png")));
    let request = MessageUpload {
        channel_id: &channel,
        content: None,
        reply_to: None,
        embeds: Some(&embeds),
        attachments: &files,
    };
    let first = client.create_message(&request).await.unwrap();
    assert!(first.attachments.as_ref().is_none_or(Vec::is_empty));
    let embed = &first.embeds.as_ref().unwrap()[0];
    assert_eq!(embed.source_type.as_deref(), Some("custom"));
    let image = embed.image.as_ref().unwrap();
    assert_eq!(image.mime_type.as_deref(), Some("image/png"));
    assert_eq!(image.size_bytes, Some(i64::try_from(PIXEL.len()).unwrap()));
    // The pin fills original attachment dimensions for video; image dimensions may be absent.
    assert_eq!((image.width, image.height), (None, None));
    let fetched = client
        .embed(embed.id.as_ref().unwrap().parse().unwrap())
        .await
        .unwrap();
    assert!(fetched.image_data.is_some_and(|data| !data.is_empty()));

    // Human message creation has no nonce: identical successful POSTs create distinct IDs.
    let second = client.create_message(&request).await.unwrap();
    assert!(first.id.is_some());
    assert_ne!(first.id, second.id);

    reject(
        client,
        &channel,
        &encode(media(None, Some("missing.png"))),
        &files,
    )
    .await;
    reject(
        client,
        &channel,
        &encode(media(
            Some("https://example.invalid/pixel.png"),
            Some("SPOILER_pixel.png"),
        )),
        &files,
    )
    .await;
    let duplicates = [
        MessageAttachment {
            filename: "SPOILER_pixel.png",
            bytes: PIXEL,
        },
        MessageAttachment {
            filename: "SPOILER_pixel.png",
            bytes: PIXEL,
        },
    ];
    reject(client, &channel, &embeds, &duplicates).await;
    let messages = client
        .messages(channel.parse::<Uuid>().unwrap(), &HistoryQuery::default())
        .await
        .unwrap()
        .messages
        .unwrap();
    assert_eq!(messages.len(), 2);
}

fn media(url: Option<&str>, attachment: Option<&str>) -> EmbedMediaInput {
    EmbedMediaInput {
        url: url.map(str::to_owned),
        attachment_name: attachment.map(str::to_owned),
        mime_type: Some("image/png".into()),
    }
}

fn encode(image: EmbedMediaInput) -> String {
    serde_json::to_string(&[EmbedInput {
        image: Some(image),
        ..Default::default()
    }])
    .unwrap()
}

async fn reject(client: &RestClient, channel: &str, embeds: &str, files: &[MessageAttachment<'_>]) {
    let result = client
        .create_message(&MessageUpload {
            channel_id: channel,
            content: None,
            reply_to: None,
            embeds: Some(embeds),
            attachments: files,
        })
        .await;
    assert!(matches!(
        result,
        Err(Error::Problem { status: 400, .. } | Error::Http { status: 400 })
    ));
}

// A synthetic one-pixel RGBA PNG; no external fetch or user media is involved.
const PIXEL: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 0,
    5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
