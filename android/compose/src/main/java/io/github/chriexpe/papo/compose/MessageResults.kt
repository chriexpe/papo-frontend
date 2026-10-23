@file:OptIn(
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.ExperimentalFoundationApi::class,
)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.layout.LazyLayoutCacheWindow
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import java.text.SimpleDateFormat
import java.util.Date
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** Chat, search and pins share the complete message body and media renderer. */
@Composable
internal fun MessageContent(
    message: ChatMessage,
    server: ServerState,
    model: PapoViewModel,
    onActions: () -> Unit,
    media: @Composable () -> Unit,
) {
    Column(
        verticalArrangement =
            Arrangement.spacedBy(if (LocalClientPreferences.current.compact) 2.dp else 4.dp)
    ) {
        val content = remember(message.content, message.embeds) { message.visibleContent() }
        if (content.isNotBlank() && GifChoice.marker(message.content) == null)
            MessageText(message, server, model, onActions)
        media()
    }
}

/**
 * A result list owns its viewport and player lifetime, using the same bounded retention registry,
 * cache, shared PlayerView and fullscreen viewer as chat.
 */
@Composable
internal fun MessageResultsMediaHost(
    server: ServerState,
    model: PapoViewModel,
    list: LazyListState,
    keys: List<String>,
    sessionKey: String = "",
    content: @Composable (String?, (String?) -> Unit, Set<String>) -> Unit,
) {
    val owner = "${server.url}/${server.me}"
    val registry = remember(owner, sessionKey) { MediaVisibilityRegistry() }
    val bookmarks = remember(owner) { PlaybackBookmarks() }
    var active by remember(owner, sessionKey) { mutableStateOf<String?>(null) }
    var viewed by remember(owner, sessionKey) { mutableStateOf<ViewerMedia?>(null) }
    var bounds by remember { mutableStateOf(Rect.Zero) }
    val lifecycle by LocalLifecycleOwner.current.lifecycle.currentStateFlow.collectAsState()
    val blocked = LocalMediaOverlayBlocking.current
    val currentKeys by rememberUpdatedState(keys)
    val enabled = viewed == null && !blocked && lifecycle.isAtLeast(Lifecycle.State.RESUMED)
    val visible by
        remember(list) {
            derivedStateOf {
                list.layoutInfo.let { layout ->
                    layout.visibleItemsInfo
                        .filter {
                            it.offset < layout.viewportEndOffset &&
                                it.offset + it.size > layout.viewportStartOffset
                        }
                        .mapNotNull { it.key as? String }
                        .toSet()
                }
            }
        }
    LaunchedEffect(registry, list, enabled) {
        snapshotFlow { list.layoutInfo to currentKeys }
            .collect { (layout, order) ->
                registry.sync(
                    layout,
                    order,
                    layout.visibleItemsInfo
                        .filter {
                            it.offset < layout.viewportEndOffset &&
                                it.offset + it.size > layout.viewportStartOffset
                        }
                        .mapNotNull { it.key as? String }
                        .toSet(),
                    enabled,
                )
            }
    }
    DisposableEffect(registry) { onDispose { registry.close() } }
    CompositionLocalProvider(
        LocalMediaViewer provides
            { media ->
                active = if (media.playback != null) media.id else null
                viewed = media
            },
        LocalViewedMedia provides viewed,
        LocalInlinePlayer provides active,
        LocalPlaybackBookmarks provides bookmarks,
        LocalMediaVisibilityRegistry provides registry,
        LocalMediaViewport provides bounds,
    ) {
        Box(Modifier.fillMaxSize().onGloballyPositioned { bounds = it.boundsInWindow() }) {
            content(active, { active = it }, if (enabled) visible else emptySet())
        }
        viewed?.let { media -> MediaViewer(server, model, media) { viewed = null } }
    }
}

@Composable
internal fun PinMessageAction(
    server: ServerState,
    model: PapoViewModel,
    channel: String,
    message: ChatMessage,
    onDone: () -> Unit,
) {
    val allowed =
        server.capabilities.pinMessage ||
            server.channels.any { it.id == channel && it.kind == "direct" }
    if (!allowed || message.pending) return
    val pending by model.pendingPins.collectAsState()
    val busy = model.pinKey(server, channel, message.id) in pending
    var error by remember(server.url, server.me, channel, message.id) { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    ListItem(
        headlineContent = {
            Text(if (message.pinned) ui(R.string.unpin_message) else ui(R.string.pin_message))
        },
        leadingContent = { Icon(Icons.Outlined.PushPin, null) },
        trailingContent =
            if (busy) {
                { CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp) }
            } else null,
        supportingContent =
            if (error.isNotBlank()) {
                { Text(error, color = MaterialTheme.colorScheme.error) }
            } else null,
        modifier =
            Modifier.clip(RoundedCornerShape(16.dp)).clickable(enabled = !busy) {
                error = ""
                scope.launch {
                    try {
                        model.setPinned(server, channel, message.id, !message.pinned)
                        onDone()
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        error = e.message ?: ui(R.string.couldn_t_update_this_pin_try_again)
                    }
                }
            },
    )
}

@Composable
internal fun MessageResultCard(
    message: ChatMessage,
    channelName: String,
    server: ServerState,
    model: PapoViewModel,
    active: String?,
    onPlayer: (String?) -> Unit,
    onOpen: () -> Unit,
    onActions: () -> Unit,
    tag: String,
) {
    val openProfile = LocalOpenProfile.current
    val external = message.externalAuthor
    val openExternal = LocalOpenExternalProfile.current
    val date =
        remember(message.at, AppLanguage.locale) {
            SimpleDateFormat("dd MMM yyyy · HH:mm", AppLanguage.locale).format(Date(message.at))
        }
    Column(
        Modifier.fillMaxWidth()
            .padding(horizontal = 4.dp, vertical = 6.dp)
            .testTag(tag)
            .clip(RoundedCornerShape(18.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerLow)
            .combinedClickable(
                onClick = onOpen,
                onClickLabel = ui(R.string.go_to_original_message),
                onLongClick = onActions,
                onLongClickLabel = ui(R.string.message_actions),
            )
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            val avatarAction =
                Modifier.combinedClickable(
                    onClick = {
                        if (external != null) openExternal(external)
                        else openProfile(message.authorId)
                    },
                    onLongClick = onActions,
                )
            // Allocate the whole minimum touch target so it cannot overlap the
            // channel/date labels and steal their conversation-navigation taps.
            Box(Modifier.size(48.dp).then(avatarAction), contentAlignment = Alignment.Center) {
                if (external != null) ExternalAvatar(server, external, Modifier.size(40.dp))
                else UserAvatar(message.authorId, message.author, Modifier.size(40.dp))
            }
            Box(
                Modifier.weight(1f).heightIn(min = 48.dp).then(avatarAction),
                contentAlignment = Alignment.CenterStart,
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Text(
                        message.author,
                        color =
                            if (external == null) userNameColor(message.authorId)
                            else MaterialTheme.colorScheme.onSurface,
                        fontWeight = FontWeight.SemiBold,
                        modifier = Modifier.weight(1f, false),
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (external != null) AppBadge()
                    if (message.pinned)
                        Icon(
                            Icons.Outlined.PushPin,
                            ui(R.string.pinned_message),
                            Modifier.size(14.dp),
                            tint = MaterialTheme.colorScheme.primary,
                        )
                }
            }
            Text(
                date,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                modifier = Modifier.widthIn(max = 100.dp),
            )
        }
        if (channelName.isNotBlank())
            Text(
                "#$channelName",
                modifier = Modifier.padding(start = 58.dp),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.primary,
            )
        MessageContent(message, server, model, onActions) {
            MessageMedia(message, server, model, active, onPlayer, onActions)
        }
    }
}

@Composable
internal fun ResultMessageActions(
    message: ChatMessage,
    channel: String,
    server: ServerState,
    model: PapoViewModel,
    onDismiss: () -> Unit,
) {
    var deleting by remember(message.id) { mutableStateOf(false) }
    if (deleting) {
        DeleteMessageDialog(
            message,
            server,
            model,
            onDismiss = { deleting = false },
            onDeleted = onDismiss,
        )
        return
    }
    val clipboard = LocalClipboardManager.current
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier.navigationBarsPadding()
                .heightIn(max = 640.dp)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp)
        ) {
            Text(
                message.author,
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.padding(16.dp),
            )
            SheetAction(ui(R.string.copy_text), Icons.Outlined.ContentCopy) {
                clipboard.setText(AnnotatedString(message.content))
                onDismiss()
            }
            MessageLinkActions(message)
            PinMessageAction(server, model, channel, message, onDismiss)
            if (message.canDelete)
                SheetAction(ui(R.string.delete_message), Icons.Outlined.DeleteOutline) {
                    deleting = true
                }
            Spacer(Modifier.height(16.dp))
        }
    }
}

@Composable
internal fun PinnedMessages(
    server: ServerState,
    model: PapoViewModel,
    onDismiss: () -> Unit,
    onOpen: (String) -> Unit,
) {
    val channel = server.selected
    var error by remember(server.url, server.me, channel) { mutableStateOf("") }
    var loading by remember(server.url, server.me, channel) { mutableStateOf(true) }
    var refresh by remember { mutableIntStateOf(0) }
    var selected by remember { mutableStateOf<ChatMessage?>(null) }
    LaunchedEffect(server.url, server.me, channel, refresh) {
        loading = true
        error = ""
        try {
            model.loadPins(server, channel)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = e.message ?: ui(R.string.couldn_t_load_pins_try_again)
        } finally {
            loading = false
        }
    }
    DisposableEffect(server.url, server.me, channel) {
        model.watchPins(server, channel, true)
        onDispose { model.watchPins(server, channel, false) }
    }
    val loadError = server.pinsError.ifBlank { if (!server.pinsKnown) error else "" }
    val waiting = loading || server.pinsLoading
    val list =
        rememberLazyListState(
            cacheWindow =
                remember { LazyLayoutCacheWindow(aheadFraction = 2f, behindFraction = 2f) }
        )
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(
            Modifier.fillMaxWidth()
                .fillMaxHeight(.9f)
                .navigationBarsPadding()
                .testTag("pinned-messages")
        ) {
            Row(
                Modifier.fillMaxWidth().padding(start = 20.dp, end = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(ui(R.string.pinned_messages), style = MaterialTheme.typography.titleLarge)
                    Text(
                        server.conversation?.name.orEmpty(),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (waiting) CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
                IconButton(onClick = onDismiss) {
                    Icon(Icons.Outlined.Close, ui(R.string.close_pinned_messages))
                }
            }
            if (loadError.isNotBlank() && !waiting)
                Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(loadError, Modifier.weight(1f), color = MaterialTheme.colorScheme.error)
                    TextButton(onClick = { refresh++ }) { Text(ui(R.string.retry)) }
                }
            if (server.connection != "online")
                Text(
                    ui(R.string.offline_showing_saved_pins),
                    Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            if (!waiting && server.pinnedMessages.isEmpty() && loadError.isBlank())
                EmptyState(
                    ui(R.string.no_pinned_messages_yet),
                    ui(R.string.pins_empty_description),
                )
            Box(Modifier.weight(1f)) {
                MessageResultsMediaHost(
                    server,
                    model,
                    list,
                    server.pinnedMessages.map { "pin/${it.id}" },
                ) { active, onPlayer, visible ->
                    LazyColumn(
                        state = list,
                        modifier = Modifier.fillMaxSize().testTag("pinned-message-list"),
                        contentPadding = PaddingValues(8.dp),
                    ) {
                        items(server.pinnedMessages, key = { "pin/${it.id}" }) { message ->
                            CompositionLocalProvider(
                                LocalMediaRowId provides "pin/${message.id}",
                                LocalTimelineMediaVisible provides ("pin/${message.id}" in visible),
                            ) {
                                MessageResultCard(
                                    message,
                                    "",
                                    server,
                                    model,
                                    active,
                                    onPlayer,
                                    {
                                        onDismiss()
                                        onOpen(message.id)
                                    },
                                    { selected = message },
                                    "pin-${message.id}",
                                )
                            }
                        }
                    }
                    selected?.let { stale ->
                        ResultMessageActions(
                            server.pinnedMessages.firstOrNull { it.id == stale.id } ?: stale,
                            channel,
                            server,
                            model,
                            { selected = null },
                        )
                    }
                }
            }
        }
    }
}

@Composable
internal fun SearchMessageResult(
    hit: io.github.chriexpe.papo.core.SearchHit,
    server: ServerState,
    model: PapoViewModel,
    visible: Boolean,
    active: String?,
    onPlayer: (String?) -> Unit,
    onOpen: () -> Unit,
    onActions: (ChatMessage) -> Unit,
) {
    var attempt by remember(server.url, server.me, hit.id) { mutableIntStateOf(0) }
    var error by remember(server.url, server.me, hit.id) { mutableStateOf("") }
    var loading by remember(server.url, server.me, hit.id) { mutableStateOf(false) }
    if (visible && hit.message == null)
        LaunchedEffect(
            server.url,
            server.me,
            server.searchQuery,
            hit.id,
            attempt,
            server.connection,
        ) {
            if (server.connection != "online") return@LaunchedEffect
            loading = true
            error = ""
            try {
                model.loadResultMessage(server, hit.channelId, hit.id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = e.message ?: ui(R.string.couldn_t_load_this_message)
            } finally {
                loading = false
            }
        }
    val message = hit.message
    if (message != null)
        MessageResultCard(
            message,
            hit.channelName,
            server,
            model,
            active,
            onPlayer,
            onOpen,
            { onActions(message) },
            "search-hit-${hit.id}",
        )
    else
        Column(
            Modifier.fillMaxWidth()
                .padding(12.dp)
                .testTag("search-hit-${hit.id}")
                .clip(RoundedCornerShape(18.dp))
                .clickable(onClick = onOpen)
                .padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(hit.author.ifBlank { ui(R.string.message) }, fontWeight = FontWeight.SemiBold)
            Text(
                "#${hit.channelName}",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.primary,
            )
            Text(
                hit.content.ifBlank { ui(R.string.attachment) },
                style = MaterialTheme.typography.bodyLarge,
            )
            if (loading) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (error.isNotBlank())
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        error,
                        Modifier.weight(1f),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                    TextButton(
                        onClick = {
                            error = ""
                            attempt++
                        }
                    ) {
                        Text(ui(R.string.retry))
                    }
                }
        }
}

@Composable
internal fun MessageLinkActions(message: ChatMessage) {
    val clipboard = LocalClipboardManager.current
    val links =
        (message.contextLinks() +
                message.embeds.map { it.url } +
                listOfNotNull(
                    GifChoice.marker(message.content)?.let { "https://giphy.com/gifs/$it" }
                ))
            .filter(::isPublicLink)
            .distinct()
    var copied by remember(message.id) { mutableStateOf("") }
    links.forEach { url ->
        ListItem(
            headlineContent = {
                Text(
                    url,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.bodySmall,
                )
            },
            supportingContent =
                if (copied == url) {
                    { Text(ui(R.string.copied_url)) }
                } else null,
            trailingContent = {
                IconButton(
                    onClick = {
                        clipboard.setText(AnnotatedString(url))
                        copied = url
                    }
                ) {
                    Icon(Icons.Outlined.ContentCopy, ui(R.string.copy_url_1_s, url))
                }
            },
        )
    }
}
