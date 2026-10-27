//! Provider-neutral presentation metadata published in HTML and oEmbed responses.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PostQuote {
    pub author: String,
    pub handle: String,
    pub caption: String,
}

pub(super) fn engagement(text: &str) -> PostMeta {
    let mut post = PostMeta::default();
    for (markers, target) in [
        (&["❤️", "❤", "♥"][..], &mut post.likes),
        (&["💬", "🗨"][..], &mut post.comments),
        (&["🔁", "🔄"][..], &mut post.shares),
        (&["👁", "👀"][..], &mut post.views),
    ] {
        for marker in markers {
            if let Some((_, after)) = text.split_once(marker) {
                let number = after
                    .trim_start_matches(|ch: char| ch.is_whitespace() || ch == '\u{fe0f}')
                    .split_whitespace()
                    .next()
                    .unwrap_or_default();
                if let Some(count) = parse_count(number) {
                    *target = Some(count);
                    break;
                }
            }
        }
    }
    post
}

pub(super) fn enrich(post: &mut PostMeta, html: &str, base: &Url, description: Option<&str>) {
    if post.author_name.is_none() && post.author_handle.is_none() {
        return;
    }
    if post.author_avatar.is_none() {
        post.author_avatar = meta_content(
            html,
            &["author:image", "og:author:image", "twitter:creator:image"],
        )
        .and_then(|raw| resolve_meta_url(base, &raw))
        .filter(|url| safe_remote_url(url));
        if post.author_avatar.is_none() {
            for raw in html.split('<').skip(1) {
                let tag = raw.split_once('>').map(|(tag, _)| tag).unwrap_or(raw);
                if !tag_name_is(tag, "link")
                    || !html_attr(tag, "rel").is_some_and(|rel| {
                        rel.split_whitespace()
                            .any(|part| part.eq_ignore_ascii_case("apple-touch-icon"))
                    })
                {
                    continue;
                }
                let Some(url) = html_attr(tag, "href")
                    .and_then(|raw| resolve_meta_url(base, &raw))
                    .filter(|url| safe_remote_url(url))
                else {
                    continue;
                };
                // An ordinary site/app icon is not an author's portrait. Social embedders
                // also advertise profile artwork here, with an explicit avatar/profile path.
                let path = Url::parse(&url)
                    .ok()
                    .map(|url| url.path().to_ascii_lowercase())
                    .unwrap_or_default();
                if path.contains("/avatar") || path.contains("/profile_images/") {
                    post.author_avatar = Some(url);
                    break;
                }
            }
        }
    }
    let Some(description) = description else {
        return;
    };
    post.fill_from(engagement(description.lines().next().unwrap_or_default()));
    for (index, _) in description.match_indices("Quoting ") {
        if index != 0 && !description[..index].ends_with('\n') {
            continue;
        }
        let tail = &description[index + "Quoting ".len()..];
        let Some((byline, caption)) = tail.split_once('\n') else {
            continue;
        };
        let Some((author, handle)) = title_byline(byline.trim()) else {
            continue;
        };
        if caption.trim().is_empty() {
            continue;
        }
        post.caption = Some(description[..index].trim().to_owned());
        post.quote = Some(PostQuote {
            author,
            handle,
            caption: caption.trim().to_owned(),
        });
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn social_metadata_separates_profile_artwork_quotes_and_counts() {
        let base = Url::parse("https://embed.example/post/123").unwrap();
        let mut post = PostMeta {
            author_name: Some("An author".into()),
            ..Default::default()
        };
        enrich(
            &mut post,
            r#"<link rel="apple-touch-icon" href="https://cdn.example/profile_images/author.jpg">"#,
            &base,
            Some("Main caption\n\nQuoting Someone (@someone)\n\nQuoted caption"),
        );
        assert_eq!(
            post.author_avatar.as_deref(),
            Some("https://cdn.example/profile_images/author.jpg")
        );
        assert_eq!(post.caption.as_deref(), Some("Main caption"));
        assert_eq!(post.quote.unwrap().caption, "Quoted caption");
        let counts = engagement("💬 48  🔁 38  ❤️ 850  👁️ 14.6K");
        assert_eq!(
            (counts.comments, counts.shares, counts.likes, counts.views),
            (Some(48), Some(38), Some(850), Some(14600))
        );
    }
    #[test]
    fn site_icons_and_prose_do_not_become_author_images_or_statistics() {
        let base = Url::parse("https://example.org/article").unwrap();
        let mut post = PostMeta {
            author_name: Some("Author".into()),
            ..Default::default()
        };
        enrich(
            &mut post,
            r#"<link rel="apple-touch-icon" href="/app-icon.png">"#,
            &base,
            Some("I ❤ my work"),
        );
        assert!(post.author_avatar.is_none());
        assert!(post.likes.is_none());
        assert!(post.quote.is_none());
    }
}
