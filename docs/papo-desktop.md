# Papo desktop on GPUI

Status: **phase 0 (bootstrap) in progress**. Written 2026-11-05, on `main` after
#141. This is the plan for `crates/papo-desktop`, the desktop client that will
replace the egui client in the root crate. Read it before working there. The
working rules for agents are in `crates/papo-desktop/AGENTS.md`. That file is
local and git-ignored, like the other agent files.

## 1. Why

Papo follows the split that Discord chose for its own clients, as reported on
2026-11-05. Discord's CTO confirmed a full desktop rewrite in Rust on GPUI
("Discord Phoenix"), with no Electron or Tauri. Mobile keeps native UI and shares the app code through
Rust. Papo already has the shared Rust half and the native mobile half:

| Layer | Papo |
| --- | --- |
| Shared Rust core | `papo-core` (protocol, state, sync, cache, policy), `papo-call` (calls) |
| Native mobile UI | `android/compose` over `papo-android` (UniFFI) |
| Native desktop UI | **`papo-desktop` on GPUI Kit: this plan** |

The Android client was rebuilt from scratch on the core, and it did not port the
egui client (`docs/android-compose.md`). The desktop client is built the same way,
for the same reason: nothing from the egui client's mistakes, extra work or bad
decisions is carried into it.

## 2. Decisions

These are settled. A change to one of them is a decision for Christian, and the
change is recorded here first.

1. **New crate, new code.** `crates/papo-desktop` is a library with a thin
   binary. Nothing in it is copied, translated or adapted from `src/` (the
   egui client). `src/` and `android/compose` are **behavioural references**:
   they show what Papo does, never how this crate does it.
2. **GPUI Kit with GPUI Fast.** The only GPUI dependency is
   `gpui-kit = { version = "=0.9.0", features = ["gpui-fast"] }`. Never add
   `gpui`, `gpui-pre` or `gpui-fast` directly, and never patch them. The Kit
   pins its GPUI snapshot (`gpui-pre =0.3.11`), and the feature swaps every
   layer to the GPUI Fast fork (0.1.13). A version change is its own commit,
   with a reason and the checks run.
3. **The core owns the truth.** Protocol, sessions, REST and WebSocket,
   canonical state, history, read state, outbox, cache, previews,
   notification policy and authorization belong to `papo-core`. Calls belong
   to `papo-call`. The desktop crate holds no second reducer, transport or
   policy.
4. **Shared logic moves down, never sideways.** When the desktop needs logic
   that lives in `papo-android` or in `src/` and that is the same for every
   client, it moves into `papo-core` or `papo-call` with tests. Examples are
   operation guards, account coordinators, call session guards, and the
   message projection that timeline, pins and search share. The other
   clients then use the moved code, without a change to the UniFFI contract.
   `papo-desktop` never depends on `papo-android` or on the root crate.
   Only logic that is independent of the platform moves. A test: would
   Android need this exact behaviour, with no GPUI or Compose concept in it?
   If not, it stays here. A view model shaped for one GPUI view, a
   presentation lifecycle, or a layout rule is desktop code, even when
   Android has something similar.
5. **No GPUI type in a shared crate.** `papo-core` and `papo-call` stay free
   of GPUI, egui and UniFFI types.
6. **One workspace, one lockfile.** `papo-desktop` is a workspace member, so
   it builds the same `papo-core` as the other clients. The workspace CI jobs
   exclude it (`--exclude papo-desktop`). Its own `papo-desktop` job runs
   `scripts/papo-desktop-check.sh` on Linux, macOS and Windows.
7. **Its own identity until cutover.** The window `app_id` is
   `io.github.chriexpe.Papo.Desktop`, not the egui client's
   `io.github.chriexpe.Papo`. The two clients can then run side by side
   without the compositor or the taskbar grouping them as one application
   (Compose did the same with its `.compose` debug suffix).
8. **The egui client keeps shipping until cutover.** No change to `src/`, to
   the `papo` binary, to packaging or to releases is part of this work until
   phase 7.
9. **Not a web app.** No Electron, Tauri or web shell, and no GPUI on Android.
   WebViews only embed provider players (phase 5).

## 3. Product decisions

Christian answered Q1, Q2 and Q4–Q7 on 2026-11-05, and Q3 (colours, then shape and type) on 2026-11-06. An
agent that reaches a new design question asks first. For a visual question,
it shows each option as a small ASCII mock-up. It ships the recommended
default only when the work cannot wait, and says so.

### 3.1 Decided

| # | Question | Decision |
| --- | --- | --- |
| Q1 | Data folder before cutover | Its own folder, `ProjectDirs("", "", "papo-desktop")`. On first launch the client offers a one-time import of egui's server list (`app.ron`) and sessions. It only reads egui's files and never changes them. The cache is not imported; it refills from the server. |
| Q2 | Sessions and server passwords | The OS credential store (GPUI's `write_credentials` / `read_credentials`: Keychain, Credential Manager, Secret Service). The file fallback is opt-in and follows §4.5. |
| — | App identity before cutover | `io.github.chriexpe.Papo.Desktop` (decision 7). |
| Q4 | Font | Inter everywhere, bundled from `assets/fonts/` through `text_system().add_fonts`. A glyph Inter does not have (CJK, emoji, other scripts) comes from the OS's own font fallback; Papo keeps no per-OS font list. A setting, "Use the system font", switches every text style to the OS's default UI font. Papo adds no font packages or checks of its own: what the system font cannot draw is the system's to provide. |
| Q5 | Window chrome | The system title bar: server-side decorations where the desktop offers them (KDE, Windows, macOS), and GPUI's client-side fallback where it does not (GNOME). No custom title bar. |
| Q6 | Shell layout | The rail and three panes, as in egui (§3.2). |
| Q7 | Translations | `rust-i18n` (the Kit's mechanism), with pt-BR and en-US files. A test fails when a key exists in one locale and not in the other, so a missing translation is caught before release. The core returns semantic values, and the shell translates them (`docs/egui-compose-handoff.md` §3). |

### 3.2 Shell layout (Q6)

```text
≥ 1000 px: rail | channels | conversation | members
┌────┬──────────────┬─────────────────────────────────────┬───────────┐
│ 💬 │(◉ Server   ⚙)│( # channel )            ( 🔍 📌 👥 )│ Online — 3│
│ 🔔 │ # geral      │                                     │ ● ana     │
│ ── │ # mídia      │  ana   olá!                         │ ● bia     │
│ ◉  │ 🔊 voz       │  さくら 日本語も表示される          │ ○ caio    │
│ ○  │              │                                     │           │
│ +  │(◐ me       ⚙)│(📎)(🎙)( Message #geral   😊 ▣ GIF ➤)│           │
└────┴──────────────┴─────────────────────────────────────┴───────────┘
(…) = floating pill
```

- **Rail:** Direct (💬) and Notifications (🔔) at the top, then the servers,
  then add (+). Direct opens the cross-server DM list in the channel pane,
  with the server's name under each person. Notifications opens the
  cross-server inbox there too, like Compose's global Direct and
  Notifications.
- **Channel pane:** a server card pill with ⚙ at the top, the channels, and
  the account pill at the bottom.
- **Header, split in two ends:** the title pill is anchored left, and the
  actions pill (🔍 search, 📌 pins, 👥 members) is anchored right. Search
  and Pins grow out of the actions pill.
- **Members pane:** 👥 shows or hides it. The choice is remembered per window,
  and the pane starts shown when the window is wide enough.
- **Composer:** attach and voice note are separate squircles beside the text
  pill. Emoji, stickers, GIF and Send are inside the text pill.
- **Narrow windows (variant C):**

  ```text
  ≥ 1000 px    rail | channels | conversation | members
  700–999 px   rail | channels | conversation        (👥 grows a sheet from its pill)
  < 700 px     conversation; rail and channels fold together
               ( ◉ Server )( # channel… )    ( 🔍 📌 👥 )
  < 520 px     ( ◉ )( # channel… )            ( 🔍 📌 👥 )
  ```

  - The rail and the channel list fold together, as one sidebar.
  - Folded, the server pill sits at the left end of the header, before the
    title. Clicking it grows the whole sidebar out of the pill, over the
    dimmed conversation.
  - Picking a channel, clicking the dimmed area or pressing Escape folds it
    back. Picking a server keeps it open.
  - Below 520 px the pill shows only the server icon. The window's 360 px
    minimum always leaves room for it, so no second control is needed.
  - On a wide window the user can also fold the sidebar by choice, with
    `secondary-\` only. The server pill then appears, and `secondary-\`
    again docks the sidebar back. There is no fold or dock button: Christian
    rejected one as ugly and unnecessary (2026-11-05).
  - The thresholds (1000, 700 and 520 px) are from the mock-up. Check them
    against the real panes once those exist.

### 3.3 Colour tokens (Q3, colours)

Christian chose this colour system on 2026-11-06, after iterating on it in a
token lab. egui's light theme was the problem to solve: users reported it as
"grey on white", with greys stacked on greys and grey text. The fix is fewer
surface steps, each with a job, and text whose contrast is measured. All values
are OKLCH lightness (L, 0–1). The `design` module computes the sRGB colours from
them; no hex value is written by hand.

**Themes.** The theme is its own setting: OLED, Graphite, Light or Soft light.

| Token | OLED | Graphite | Light | Soft light | Used for |
| --- | --- | --- | --- | --- | --- |
| `surface.rail` | 0.000 | 0.205 | 0.952 | 0.925 | Server rail, title bar |
| `surface.sidebar` | 0.000 | 0.228 | 0.972 | 0.950 | Channel list, members, embeds |
| `surface.content` | 0.000 | 0.252 | 1.000 | 0.975 | Message list |
| `surface.pill` | 0.175 | 0.305 | 1.000 | 1.000 | Floating pills |
| `surface.hover` | 0.195 | 0.295 | 0.940 | 0.918 | Row under the pointer |
| `surface.code` | 0.135 | 0.222 | 0.968 | 0.945 | Code blocks |
| `line.subtle` | 0.225 | 0.310 | 0.918 | 0.900 | Pane edges, dividers |
| `line.strong` | 0.320 | 0.390 | 0.840 | 0.820 | Dashed and emphasised edges |
| `line.pill` | 0.255 | 0.360 | 0.905 | 0.890 | Pill outline |
| `text.primary` | 0.965 | 0.955 | 0.215 | 0.215 | Message text, names, unread channels |
| `text.secondary` | 0.800 | 0.800 | 0.400 | 0.395 | Read channels, reactions |
| `text.muted` | 0.645 | 0.665 | 0.530 | 0.505 | Timestamps, labels, offline members |

- OLED is true black on every large surface. Its panes are separated by
  lines, and its pills are solid, because glass over black shows nothing.
- Light keeps the message list pure white, and its white pills carry a soft
  shadow and an outline. Soft light sets every surface a step below white, so
  the white pills stand out more.
- `surface.selected` mixes the accent into `surface.sidebar`: 22 % on dark
  themes, 13 % on light themes. `accent.soft` mixes it into
  `surface.content`: 20 % and 12 %.

**Contrast floors** (WCAG ratio, on the surface where the text sits). A token
change that breaks one of these is a bug, and a unit test checks them for every
theme at every setting:
`text.primary` at least 13:1, `text.secondary` at least 7:1, `text.muted` at
least 4.5:1, links at least 4.5:1. The values above were measured over every
theme, every 5° of hue, strengths and vividness from 0 to 1, and all six
accents (2026-11-06). The lowest ratios were 13.70:1, 8.08:1, 4.82:1 and
4.65:1.

**Accent.** One accent hue: Blue 256° (the default), Indigo 278°, Teal 196°,
Green 152°, Pink 352° or Orange 52°. Its chroma is 0.18, and its lightness is
0.70 on dark themes and 0.555 on light themes. A link uses the same hue at
chroma 0.15, with lightness 0.76 on dark themes and 0.50 on light themes. Teal
and green look brighter than other hues at one lightness, and at 0.52 a Teal
link on a fully tinted Soft light fell to 4.29:1.
Text drawn on the accent (`accent.on`) is black or white, whichever contrasts
more.

**Background colour.** These are separate settings, all user-chosen:

- **Background colour:** up to three colours, each a hue (0–360°) and a
  vividness (0–1), picked on a 2-D field. The angle sets the hue, and the
  distance from the centre sets the vividness. There are presets, and a
  harmony action that places the extra colours (opposite hue, three evenly
  spaced hues, neighbours). The default is one colour: 255°, vividness 0.45.
- **Colour strength** (0–1, default 0.40): how much of the background colour
  reaches the surfaces. Its slider track fades from neutral grey to the
  colour, and the handle shows the result.
- **Grain** (0–1, default 0): a fine noise over the surfaces. Its slider track
  runs from clean to coarse, and the handle shows the grain. Grain lifts
  OLED's true black, and the setting says so.

How the background colour reaches the tokens:

- Every neutral token takes the first colour's hue, with chroma
  `strength × vividness × 0.075 × (0.3 + 0.7 × (1 − |2L − 1|))`. The tint
  is strongest in the mid-tones and weaker towards black and white.
- The chroma fades to zero over the last 0.03 of L at both ends, so pure
  black and pure white stay untinted.
- Text tokens take half that chroma.
- A second or third colour becomes a gradient (165°) across the chrome only:
  rail, sidebar, members and title bar. The message list keeps the first
  colour, so text contrast does not change while you scroll.

**Semantic colours** keep fixed hues: mention red (25°), online green (150°),
and the mention-row marker amber (82°, mixed at 12–13 % into
`surface.content` for the row background).

These replace the Kit's theme. They are restated in one `design` module, and
the Kit's theme tokens are set from it, so the Kit's components match.

### 3.4 Shape and type tokens (Q3, shape)

Christian accepted the **Balanced** direction on 2026-11-06. These are Papo's
defaults. The user's text-scale setting multiplies every text size on top of
them. The base values come from the token lab's mock-up, not from egui.

| Setting | Default | Range in the lab |
| --- | --- | --- |
| Text size (`type.body`) | 14 px | 12–16 px |
| Roundness (multiplies every radius) | ×1.00 | ×0–×1.6 |
| Spacing (multiplies every space) | ×1.00 | ×0.8–×1.25 |
| Glass blur | 16 px | 0–32 px |
| Shadow depth | ×1.00 | ×0–×1.5 |

The lab offered two other directions, kept here for reference: Compact (13 px,
radii ×0.75, spacing ×0.88, blur 12 px, shadow ×0.80) and Airy (15 px, ×1.30,
×1.15, 22 px, ×1.15).

**Type.** Every size is a ratio of `type.body`, so one setting scales them all.
Body text uses a line height of 1.45.

| Token | Ratio | At 14 px | Used for |
| --- | --- | --- | --- |
| `type.caption` | 11/13 | 11.8 px | Timestamps, group labels, sub-lines |
| `type.small` | 12/13 | 12.9 px | Replies, embed text, typing indicator |
| `type.body` | 1 | 14 px | Message text, channel names |
| `type.title` | 14/13 | 15.1 px | Server card, rail letters |
| `type.display` | 18/13 | 19.4 px | Large glyphs |

Weights: 400 for running text, 500 for selected and secondary emphasis, 600 for
names, titles and unread channels.

**Radii** (at ×1):

| Token | Value | Used for |
| --- | --- | --- |
| `radius.indicator` | 3 px | Selection bar on the rail |
| `radius.control` | 8 px | Channel rows, hover, reactions, code blocks |
| `radius.card` | 10 px | Embeds, cards |
| `radius.avatar` | 11 px | Message avatars (rounded squares) |
| `radius.icon` | 12 px | Rail icons, window corner |
| `radius.pill` | 14 px | Header pills, server and account cards |
| `radius.composer` | 16 px | Message field |

Presence dots and the colour-picker dots stay circles at any roundness.

**Spacing** (at ×1): `space.xs` 2 px, `space.sm` 4 px, `space.md` 8 px,
`space.lg` 12 px, `space.xl` 16 px, `space.xxl` 20 px. Pill heights scale with
spacing: header pills 36 px, composer 44 px, Send 30 px. Pane widths do not:
rail 60 px, channel list 232 px, members 214 px.

**Glass and shadow.** Pills blur what is behind them by 16 px and saturate it by
1.5. Their shadow is one soft lift and one tight contact shadow, both scaled by
depth:

- dark themes: `0 7px 24px` at 40 % black (60 % on OLED), plus `0 1px 3px` at
  30 %;
- light themes: the same offsets in a blue-grey ink (`rgb(25,30,45)`), at 10 %
  and 8 %.

Depth 0 removes the shadow, and the pill outline (`line.pill`) remains. OLED's
pills are solid, with no blur (§3.3).

### 3.5 Open

None. Every question the first screens need is answered. New design
questions follow the rule at the top of §3.

## 4. Architecture

### 4.1 Dependency direction

```text
papo-desktop (GPUI views, platform adapters)
   │  projections: narrow, typed view models built from core state
   ▼
papo-core (+ papo-call for calls)   ← the only owner of canonical state
```

Views read projections and send commands. Platform backends (tray,
notifications, file dialogs, WebViews) implement desktop-owned interfaces and
know nothing about feature views. GPUI entities and handles never cross into a
shared crate.

### 4.2 Target layout

This is a guide, not a freeze on file names. A module is added only when a
phase needs it, and it is named after the one thing it does.

```text
crates/papo-desktop/
  src/main.rs          the binary: calls papo_desktop::run, nothing else
  src/lib.rs           crate header, module list, run()
  src/app.rs           application lifetime, app-wide actions, exit rule
  src/window.rs        window options, identity, backend log
  src/shell.rs         root view: layout state only
  src/session/         one ServerRuntime per server, the wake bridge (phase 1)
  src/projection/      core state → view models (phase 1)
  src/design/          Papo tokens over the Kit theme (phase 1, Q3)
  src/strings/         pt-BR and en-US (phase 1, Q7)
  src/timeline/        virtualized messages, anchor, read reporting (phase 2)
  src/composer/        text, IME, mentions, emoji, attachments (phases 2–3)
  src/media/           decode budgets, GPUI image adapter (phase 4)
  src/embed/           WebView ownership and lifecycle (phase 5)
  src/platform/{linux,macos,windows}/   tray, notifications, portals (phase 6)
```

### 4.3 State and threads

What is known (measured 2026-11-05 in a throwaway build on Linux):

- `ServerRuntime::open` starts one `papo-net` thread per server, with its own
  Tokio runtime. GPUI runs its own executor on the main thread. The two
  coexist in one process.
- `Wake` is called on the `papo-net` thread. A `Wake` that does
  `try_send(())` on a bounded(1) `async-channel` wakes a task started with
  `cx.spawn` on the main thread. That task drains with
  `drain_until_idle(DEFAULT_DRAIN_LIMIT)` and notifies the view. This worked
  against an unreachable server (the store reported `Offline`).

What is not decided, and is the first job of phase 1:

- **Where `ServerRuntime` lives.** The default is **(b), a session actor**:
  one session thread per server owns the `ServerRuntime`, as Android's actor
  does, and publishes immutable `Arc` projections to GPUI. Store updates,
  cache hydration and network events then never cost frame time, and the
  responsibilities are split the same way on both clients. The alternative,
  (a), keeps `ServerRuntime` in a GPUI entity and drains it on the main
  thread, as egui does every frame.
  Measure both before you build on either, and record the numbers here.
  Measure a 10k-message channel, and also a burst: a reconnect that replays
  many events, and cache hydration at startup. Choose (a) only if it is
  clearly better in every scenario. A small gain in one benchmark does not
  outweigh frame time under bursty updates.
- **How `SecretStore` (synchronous, `papo-net` thread) reaches an async OS
  credential store** (Q2, §4.5).

Rules that hold either way:

- Nothing polls per frame. Work starts from a wake, an input or a timer.
- Projections change incrementally where it is practical. A new message
  changes the rows it touches. It does not copy or rebuild the whole
  conversation snapshot. Unchanged rows keep their identity (the same `Arc`),
  so GPUI Fast keeps their views.
- Every async result carries the server, account and channel (or request)
  identity it was asked for. A result that no longer matches is dropped.
  This covers navigation, logout and a replaced session.
- Decoding, hashing, file I/O and network work never run on the main thread.
- GPUI Fast keeps unchanged views. A view sees a change only through an
  entity, a global, list or scroll state, or `cx.notify()` (gpui-fast
  `docs/retained-mode.md`). Never mutate shared state through interior
  mutability behind GPUI's back. Notify only when a value really changed.
  Notifying in prepaint or paint rebuilds every frame.
- Shutdown is explicit: closing the last window stops each runtime, joins the
  `papo-net` threads and flushes the cache before the process exits.

### 4.4 Identity and data

- Window identity: `io.github.chriexpe.Papo.Desktop` (decision 7).
- Data folder, sessions and import: Q1 and Q2. Until they are decided, the
  crate opens no data folder.
- The egui client's layout, for reference only: `ProjectDirs("", "", "papo")`.
  Sessions are in `<data>/sessions/<server-key>.token` and `.server`, the
  cache in `<data>/papo-cache.db`, and the server list in eframe's `app.ron`.
  `server_key` comes from `papo-core`. The egui files are plaintext.

### 4.5 Credential storage policy

This applies whatever Q2 decides.

- **No silent downgrade.** When the OS store fails or is missing, the client
  never writes a credential to a file on its own. It tells the user, and the
  session stays in memory for this run only.
- **The file fallback is the user's choice.** It is a setting the user turns
  on explicitly, with a plain statement that the token is then stored on
  disk unencrypted. The setting can be turned off again.
- **Files, when used:** mode `0600` in a folder with mode `0700` (on Windows,
  an ACL for the user only), written atomically (temporary file, `fsync`,
  rename), and never logged.
- **Migration is explicit.** An imported egui token (Q1) moves into the OS
  store. The client deletes its own copy of the file only once the store has
  confirmed the write. The egui client's files are never changed before
  cutover. Signing out and forgetting a server delete the credential from
  every place it is stored, and the user learns of any delete that fails.
- Every one of these rules has a test.

## 5. Behavioural references

What Papo does is already written down. Read the part a task touches before
writing it. These describe behaviour; they are not code to copy.

| Area | Read |
| --- | --- |
| Everything the Android client does, and why | `docs/android-compose.md` |
| Lessons learnt and contracts: state ownership, reading and hourglass, history and cache, media lifetimes, composer and identity | `docs/egui-compose-handoff.md` |
| History, paging and message navigation | `docs/message-history-audit.md` |
| Media cache contract | `docs/media-cache-audit.md` |
| Calls: ownership, lifecycle, ordering | `docs/android-calls.md` |
| Appearance parity items | `docs/android-aero-parity.md` |
| Glass, rendering and KWin notes from egui (what not to repeat) | `docs/rendering.md` |
| Chat quality-of-life list | `docs/chat-qol.md` |
| macOS state | `docs/macos.md` |

When a reference and the core disagree, the core's tests decide. When two
references disagree, ask.

## 6. Phases

Each phase ends with something a person can run against a real server. Each PR
is one vertical slice that builds and is tested. A PR is never only
scaffolding.

### Phase 0: bootstrap (this commit)

Done:

- the crate, with lints, its own `clippy.toml` and a module header on every file;
- a GPUI Kit + Fast window with Papo's own `app_id`, title and the egui sizes;
- logging, with the display backend and GPU recorded on every run;
- Quit (`secondary-q`), and exit when the last window closes;
- four tests, which run headless;
- `scripts/papo-desktop-check.sh`, used by CI's `papo-desktop` job;
- `libxkbcommon-x11` added to `scripts/deps.sh`. GPUI links it, and without it
  the link fails on Ubuntu 24.04.

Verified on 2026-11-05: Arch (CachyOS) on native Wayland and on XWayland
(AMD RADV, Vulkan); Ubuntu 24.04 in a container (build, Clippy, tests).

Remaining for phase 0:

- the macOS and Windows legs of the CI job are green, and then they block;
- a manual run on GNOME (Zorin, Ubuntu 24.04) on Wayland and X11.

### Phase 1: session and first screens

First, before any authenticated screen: choose where `ServerRuntime` lives
(§4.3), with measurements. §3 fixes the data,
credentials, threads, tokens, chrome and shell layout that every screen
stands on. Then build: add a server, sign in, register and unlock; several
servers; the server and channel lists with live updates; offline and
reconnect; a clean shutdown that joins the transport.

Done when: a person adds a real server, signs in, sees channels update live,
pulls the network cable and gets it back, and quits with no thread left.

### Phase 2: timeline and basic composer

A virtualized timeline with stable message IDs and variable row heights. It
keeps the anchor on prepend and reflow, and it follows the bottom only when it
should. Paging at both ends uses the core's history API. Read reporting comes
from the rows actually visible. Also: unread and hourglass navigation, date
headers, author grouping, Markdown from `papo-core::markdown`, and text sends
through the durable outbox (the draft stays until the core acknowledges it).
The Kit's `MessageScroller` does not implement Papo's read or unread semantics.
Use it only if it can carry them.

Done when: a 10k-message channel scrolls in both directions without a jump,
and reading moves the core's read state exactly as on Android.

### Phase 3: composer and interactions

Reply, edit and delete. Mentions bound to user IDs. Unicode and server emoji,
stickers and GIFs. Attachments, paste and drag-and-drop. Search and pins.
Role-aware actions from the core's policy. DMs, menus, popovers and keyboard
access. IME, selection, clipboard and undo, tested on each OS.

### Phase 4: media

Images and animated media, direct video and audio through GStreamer, with
explicit budgets: decoded frames, textures and active decoders. Each item is
visible, near, offscreen or released.

Done when, after warm-up, repeated equivalent cycles of open, close, scroll
and reconnect converge to a stable memory range. A bounded cache is allowed,
and so is memory the allocator keeps after a release. When their owners are
released, active decoders, GPU textures, WebViews, subscriptions and queued
work return to their idle budgets, and each of these is counted on its own.

### Phase 5: embeds

Provider players in native WebViews, with an explicit lifecycle: create, show,
suspend and destroy. Menus, tooltips and dialogs must draw above them, and
clipping must work while scrolling. Not verified yet: `gpui-webview` 0.9.0
uses `lb-wry` with GTK on Linux. GPUI Fast documents composition over a
Wayland subsurface, but an earlier note (closed PR #140) says the WebView
wrapper does not support native Wayland. Prove which is true on each OS before
designing against it.

### Phase 6: the rest

Settings, account and server administration, moderation and audit,
notifications, calls through `papo-call`, presence, updates, tray and
close-to-tray, file dialogs, window appearance and packaging. Keep a
feature-by-feature parity table against egui and Compose in this file.
Neither of them is an unquestioned design reference.

### Phase 7: cutover

Only after the gates in §7 pass. Switch the release entry point and the app id.
Migrate the data (Q1). Remove egui, its vendor patches, the winit preparation
and its shims. Then update the README, CI, packaging and these docs.

## 7. Gates

**Automated, every PR:** `scripts/papo-desktop-check.sh`. Add `--shared` when
the PR touches `papo-core`, `papo-call` or `Cargo.lock`. Unit tests cover
projections, identity scoping of async results, teardown, read and unread
boundaries, and media and WebView lifecycle states. UI integration tests
(`#[gpui_kit::test]`) cover row identity, anchor preservation, changing
heights and composer keys. A headless test does not replace a run on a real
desktop.

**Manual, on real machines:** sign in and out; several servers;
offline and reconnect; switching channels while history loads; a new message
arriving while reading; the hourglass; DPI and font scaling; density
variants; embeds under menus and dialogs; call join, leave and rejoin; repeated
open and close of profiles, settings, pickers and the media viewer; suspend and
resume; close-to-tray. Run them on Linux (KDE and GNOME, Wayland and X11),
Windows and macOS.

**Memory and performance:** compare a release build of this crate with the
egui client, with the same data on the same machine. Measure RSS, PSS or
private bytes, GPU memory, GStreamer buffers and WebView processes separately.
Record the scenario, its duration and the graphics backend. The criterion is
convergence, not identical numbers: after warm-up, repeated cycles stay in a
stable range, and the resources counted in phase 4 return to their idle
budgets. Growth without a bound is a failure. A bounded cache, or memory the
allocator keeps after a release, is not one. Discord reported about 290 MB for an early
Windows build of its rewrite. That is a single screenshot, not a target, and no
memory win is assumed from the framework alone.

## 8. GPUI Kit facts verified here

Checked against the sources in `~/.cargo/registry` on 2026-11-05. They change
with every Kit release, so check them again after a version bump.

- `gpui-kit 0.9.0` with `gpui-fast` builds **both** `gpui-pre` and `gpui-fast`.
  Cargo features are additive, so this is expected.
- Adding it, with its `test-support` feature for tests, adds 387 package
  entries to `Cargo.lock` (782 → 1169). It also moved some existing crates up
  within their semver range (`icu_*` 2.3 → 2.4, `zerovec`, `yoke`,
  `tinystr`, `toml`). After the change, the rest of the workspace passes
  Clippy and its 892 tests (`scripts/papo-desktop-check.sh --shared`).
- `scripts/flatpak-sources.sh` declares every crate in `Cargo.lock` as a
  Flatpak source. The egui Flatpak therefore downloads the GPUI crates'
  sources too, but it does not build them. This costs download size only.
- GPUI's macros emit `gpui::` paths, and with `gpui-fast` the Kit does not
  rewrite them. `lib.rs` therefore declares `extern crate gpui_kit as gpui;`
  once.
- `KeyBinding::new` accepts only explicit modifiers and **panics** on
  `secondary-`. Use `KeyBinding::load(.., cx)`. A unit test found this bug.
- `cx.compositor_name()` returns `Wayland` or `X11`. `window.gpu_specs()`
  returns the adapter and driver.
- At `info`, zbus and the accessibility bridge log every D-Bus handshake. The
  default filter keeps other crates at `warn`.
- Linux needs `libxkbcommon-x11` at build and at run time.
- `#[gpui_kit::test]` runs on GPUI's test platform. It needs no display, so it
  runs in CI and in containers.

## 9. Follow-ups

Agents add items here instead of growing a PR beyond its task. Each item names
its phase.

- (phase 0) Make the macOS and Windows legs of the `papo-desktop` CI job block
  once they are green.
- (phase 0) Run the bootstrap on GNOME (Zorin, Ubuntu 24.04), on Wayland and
  X11, and record the backend log line here.
- (release) `packaging/cargo-sources.json` must be regenerated for the next
  release, because `Cargo.lock` changed (`scripts/flatpak-sources.sh`).
