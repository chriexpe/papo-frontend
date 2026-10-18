# Chat QoL checklist

Branch: `feat/chat-qol`, based on `origin/feat/unread-divider` at `8f407ef`.
The checkout was fast-forwarded by 11 commits; it already contained `origin/main`.

- [x] Render server stickers inside the composer, with a trailing space. Preserve shortcodes in drafts, clipboard text and API payloads.
- [x] Add that space only when completing a newly inserted shortcode. Backspace can remove the space and then the inline sticker without completion reinserting it.
- [x] Decode animated WebP/APNG as animations and preserve animation when preparing stickers.
- [x] Paste files/images without first focusing the composer. Receive native Wayland file drops. Keep attachments with their target channel.
- [x] With an empty composer, ↑ edits the latest editable message from the current user in that channel. Suggestions retain their arrow-key navigation.
- [x] Keep supported OS-native, per-window scaling through winit/eframe, with the slider as an additional multiplier. Checked platform documentation; no custom settings reader or scaling probe was added.
- [x] Edit attachments in the app: image crop, filename, spoiler and image anonymization. Other media support rename, spoiler and random filename. Edited files are copies; animations retain their frames and timing.
- [x] Remove button-sized minimum heights from message text rows and tighten the author/reply/body spacing.
- [x] Project sent text and attachments immediately with normal message styling and no sending footer. Confirm by local identity; show status and retry/dismiss controls only when delivery fails or becomes uncertain.
- [x] Show reaction users on hover and mobile hold, using the existing paginated endpoint.
- [x] Toggle emoji/sticker/GIF pickers from their composer buttons.
- [x] Focus emoji/sticker search automatically; use arrow keys to navigate results and Enter to select.
- [x] Keep @names as plain text in DMs and disable mention autocomplete there.
- [x] Align composer placeholder, typed text and caret; audit other text fields for matching fonts and vertical alignment.
- [x] Accept null search message content, reset search on close, and ignore responses from earlier searches.
- [x] Preserve spaces in channel search filters and keep invalid filters from becoming a broader search.
- [x] Fetch complete pinned messages on opening the panel, including messages outside loaded history.
- [x] Persist full pinned messages and attachments; restore them immediately from the existing channel cache while refreshing in the background.
- [x] Give media in search results separate widget identities from the timeline.
- [x] Add image/GIF context menus: copy image, save image, copy media link and open media link.
- [x] Apply wheel movement once across repeated layout passes. Preserve the visible message when content changes above it or older history is prepended, and keep row widget identities stable during culling.
- [x] Keep return-to-latest active until the newest viewport is actually drawn, including when the unread tail grows after its initial measurement.
- [x] Add “Prefer backend embeds” under Appearance → Chat, enabled by default. Backend cards own their metadata, including absent media; frontend previews remain available for links without backend cards. Disabling the setting restores frontend enrichment.
- [x] Flatten multiline reply snippets before layout so the single-line quote does not clip their first and last lines.
- [x] Render provider GIFs as standalone media using declared GIF fetch methods/MIME types, with legacy GIPHY, Tenor and KLIPY page compatibility. Reserve declared dimensions during loading, preserve aspect ratio and use the chat's media width instead of a small titled link card.

The crop controls reuse the existing original-ratio/square editor. Cropped or
anonymized WebP/APNG animations are encoded as GIF. Original files are untouched.

Manual checks still needed: Android sticker editing and reaction holds, attachment
editor layout, file-manager drops on the target desktop, and the reported avatar
flicker while search is open. Search decoding, reset, pinned snapshots, widget
identity and the image menu have regression coverage; the avatar flicker has not
been independently reproduced. Scaling work is
closed at the requested scope: retain the supported native path. The tester’s
specific mixed-monitor report has not been reproduced or claimed fixed.
