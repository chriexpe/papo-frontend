@file:OptIn(
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.ExperimentalFoundationApi::class,
)

package io.github.chriexpe.papo.compose

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.animateScrollBy
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
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
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.encodeEmbedDrafts
import io.github.chriexpe.papo.core.validateEmbedDrafts
import java.text.SimpleDateFormat
import java.util.Date
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

@Composable
internal fun ConversationScreen(server: ServerState, model: PapoViewModel, onBack: () -> Unit) {
    val channel = server.conversation ?: return
    val draftKey = "${server.url}/${server.me}/${channel.id}"
    var composer by
        rememberSaveable(draftKey, stateSaver = ComposerDraft.Saver) {
            mutableStateOf(
                ComposerDraft.restore(model.draft(draftKey), model.draftMentions(draftKey))
            )
        }
    var embedJson by rememberSaveable(draftKey) { mutableStateOf(model.embedDrafts(draftKey)) }
    val embedDrafts = remember(embedJson) { restoredEmbeds(embedJson) }
    var embedEditor by rememberSaveable(draftKey) { mutableStateOf(false) }
    fun updateEmbeds(next: List<io.github.chriexpe.papo.core.EmbedDraft>) {
        embedJson = encodeEmbedDrafts(next)
        model.saveEmbedDrafts(draftKey, embedJson)
    }
    val openProfile = LocalOpenProfile.current
    val composerFocus = remember { androidx.compose.ui.focus.FocusRequester() }
    val text = composer.text
    fun updateComposer(next: ComposerDraft) {
        composer = next
        model.saveDraft(draftKey, next.text, next.json())
    }

    var reply by rememberSaveable(draftKey) { mutableStateOf<String?>(null) }
    // The device preference initializes each new reply; a per-reply override survives recreation.
    var notifyReply by rememberSaveable(draftKey) { mutableStateOf(true) }
    var emojiPicker by remember(draftKey) { mutableStateOf(false) }
    var emojiTarget by remember(draftKey) { mutableStateOf<ChatMessage?>(null) }
    var gifPicker by remember(draftKey) { mutableStateOf(false) }
    var gifPreparing by remember(draftKey) { mutableStateOf(false) }
    var reactionPeople by remember(draftKey) { mutableStateOf<Pair<String, EmojiChoice>?>(null) }
    val expressionStates by model.expressions.states.collectAsState()
    val expressions =
        expressionStates[model.expressions.owner(server)]
            ?: model.expressions.state(model.expressions.owner(server))
    var selected by remember { mutableStateOf<ChatMessage?>(null) }
    var edit by remember { mutableStateOf<ChatMessage?>(null) }
    var delete by remember { mutableStateOf<ChatMessage?>(null) }
    var retry by remember { mutableStateOf<ChatMessage?>(null) }
    var pins by remember(draftKey) { mutableStateOf(false) }
    var members by remember { mutableStateOf(false) }
    val appearance = LocalClientPreferences.current
    var descriptionExpanded by
        rememberSaveable(draftKey) {
            mutableStateOf(appearance.showDescription && channel.topic.isNotBlank())
        }
    var descriptionTouched by remember(draftKey) { mutableStateOf(false) }
    LaunchedEffect(draftKey) {
        if (descriptionExpanded) {
            kotlinx.coroutines.delay(6000)
            if (!descriptionTouched) descriptionExpanded = false
        }
    }
    var activeMedia by remember(draftKey) { mutableStateOf<String?>(null) }
    // Keep the editing snapshot alive until Apply finishes. Replacing a cropped
    // draft file must not dispose the sheet/cancel its callback mid-commit.
    var attachmentOptions by remember(draftKey) { mutableStateOf<PendingAttachment?>(null) }
    var viewedMedia by remember(draftKey) { mutableStateOf<ViewerMedia?>(null) }
    var submitting by remember(draftKey) { mutableStateOf(false) }
    var recording by remember(draftKey) { mutableStateOf(false) }
    var composerFocused by remember(draftKey) { mutableStateOf(false) }
    var confirmAttachmentRetry by remember(draftKey) { mutableStateOf(false) }
    val attachmentDrafts by model.attachments.drafts.collectAsState()
    val attachmentDraft = attachmentDrafts[draftKey] ?: AttachmentDraft()
    val listState =
        rememberLazyListState(
            cacheWindow =
                remember { LazyLayoutCacheWindow(aheadFraction = 2f, behindFraction = 2f) }
        )
    val playbackBookmarks = remember(draftKey) { PlaybackBookmarks() }
    val mediaVisibility = remember(draftKey) { MediaVisibilityRegistry() }
    var listBounds by remember { mutableStateOf(Rect.Zero) }
    val scope = rememberCoroutineScope()
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val mediaLifecycle by lifecycle.currentStateFlow.collectAsState()
    val overlayBlocking by rememberUpdatedState(LocalMediaOverlayBlocking.current)
    val density = LocalDensity.current
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    // Check the platform at dispatch time: Compose's IME insets can lag (or
    // stay zero) after a dialog on newer Android versions. Keep this callback
    // registered so a stale visibility snapshot cannot fall through to leaving.
    val nativeView = androidx.compose.ui.platform.LocalView.current
    val imeInsets = WindowInsets.ime
    androidx.activity.compose.BackHandler(!overlayBlocking) {
        val keyboardVisible =
            androidx.core.view.ViewCompat.getRootWindowInsets(nativeView)
                ?.isVisible(androidx.core.view.WindowInsetsCompat.Type.ime()) == true
        if (keyboardVisible || imeInsets.getBottom(density) > 0) {
            keyboard?.hide()
            focus.clearFocus()
        } else onBack()
    }
    var headerHeight by remember { mutableStateOf(90.dp) }
    var composerHeight by remember { mutableStateOf(90.dp) }
    var handledNavigation by
        rememberSaveable(
            draftKey,
            stateSaver =
                androidx.compose.runtime.saveable.Saver<ULong, String>(
                    save = { it.toString() },
                    restore = { it.toULong() },
                ),
        ) {
            mutableStateOf(0uL)
        }
    var highlighted by remember(draftKey) { mutableStateOf<Pair<String, ULong>?>(null) }
    var returningLatest by remember(draftKey) { mutableStateOf(false) }
    val nearBottom by remember {
        derivedStateOf {
            listState.firstVisibleItemIndex == 0 && listState.firstVisibleItemScrollOffset == 0
        }
    }
    var positionReady by remember(draftKey) { mutableStateOf(false) }
    var jumping by remember(draftKey) { mutableStateOf(false) }
    var followingLatest by remember(draftKey) { mutableStateOf(true) }
    var autoFollowing by remember(draftKey) { mutableStateOf(false) }
    var latestPhase by remember(draftKey) { mutableStateOf(LatestPhase.FirstNew) }
    var knownLive by remember(draftKey) { mutableStateOf(server.reading.liveIds) }
    var arrived by remember(draftKey) { mutableStateOf(setOf<String>()) }
    // The divider reflects the shared unread cursor. A remembered arrival ID
    // outlives its acknowledgement and creates a second, stale unread marker.
    val entryUnread = server.reading.unreadStart
    val groupedMessages =
        remember(
            server.messages,
            entryUnread,
            server.reading.gapStart,
            server.reading.gapEnd,
            appearance.catchUpHourglass,
        ) {
            messageContinuations(
                server.messages,
                setOf(entryUnread) +
                    if (appearance.catchUpHourglass) setOf(server.reading.gapStart) else emptySet(),
                if (appearance.catchUpHourglass) setOf(server.reading.gapEnd) else emptySet(),
            )
        }
    val readingBlocked =
        attachmentOptions != null ||
            embedEditor ||
            overlayBlocking ||
            viewedMedia != null ||
            selected != null ||
            edit != null ||
            delete != null ||
            retry != null ||
            members ||
            pins ||
            emojiPicker ||
            gifPicker ||
            reactionPeople != null
    val currentBlocked by rememberUpdatedState(readingBlocked)
    LaunchedEffect(draftKey, server.reading.liveIds) {
        val live = server.reading.liveIds
        val previousTail = knownLive.lastOrNull()
        val index = live.indexOf(previousTail)
        val added =
            if (index >= 0) live.drop(index + 1).filter { it !in knownLive }
            else if (positionReady && knownLive.isEmpty()) live else emptyList()
        arrived = arrived + added
        knownLive = live
    }
    val clipboard = LocalClipboardManager.current
    val latestServer by rememberUpdatedState(server)
    val viewport =
        with(density) {
            Rect(
                listBounds.left,
                listBounds.top + (headerHeight + 8.dp).toPx(),
                listBounds.right,
                (listBounds.bottom - (composerHeight + 8.dp).toPx()).coerceAtLeast(
                    listBounds.top + (headerHeight + 8.dp).toPx()
                ),
            )
        }
    val visibleRows by remember {
        derivedStateOf {
            val layout = listState.layoutInfo
            val height =
                (layout.viewportSize.height -
                        layout.beforeContentPadding -
                        layout.afterContentPadding)
                    .coerceAtLeast(0)
            layout.visibleItemsInfo
                .filter { it.offset < height && it.offset + it.size > 0 }
                .mapNotNull { it.key as? String }
                .toSet()
        }
    }
    LaunchedEffect(mediaVisibility, listState) {
        snapshotFlow {
            Triple(
                listState.layoutInfo,
                latestServer.messages.asReversed().map { it.id },
                viewedMedia == null &&
                    attachmentOptions == null &&
                    !embedEditor &&
                    edit == null &&
                    !pins &&
                    !overlayBlocking &&
                    mediaLifecycle.isAtLeast(Lifecycle.State.RESUMED),
            )
        }
            .collect { (layout, order, enabled) ->
                val height =
                    (layout.viewportSize.height -
                            layout.beforeContentPadding -
                            layout.afterContentPadding)
                        .coerceAtLeast(0)
                mediaVisibility.sync(
                    layout,
                    order,
                    layout.visibleItemsInfo
                        .filter { it.offset < height && it.offset + it.size > 0 }
                        .mapNotNull { it.key as? String }
                        .toSet(),
                    enabled,
                )
            }
    }
    DisposableEffect(mediaVisibility) { onDispose { mediaVisibility.close() } }

    LaunchedEffect(draftKey, server.messages.lastOrNull()?.id) {
        if (
            positionReady &&
                !jumping &&
                followingLatest &&
                !server.loadingOlder &&
                !server.browsingHistory &&
                !server.loadingContext &&
                (server.navigationTarget.isBlank() || handledNavigation == server.navigationSerial)
        ) {
            autoFollowing = true
            try {
                listState.animateScrollToItem(0)
            } finally {
                autoFollowing = false
            }
        }
    }
    // Restore before reporting any visible row: the initial live window must
    // not mark the channel read while we are still locating a saved position.
    val savedPosition =
        remember(draftKey) { if (appearance.openNewest) null else model.readingPosition(draftKey) }
    LaunchedEffect(
        draftKey,
        server.loadingContext,
        server.timeline,
        server.messages.map { it.id },
    ) {
        if (positionReady || server.messages.isEmpty() || server.loadingContext)
            return@LaunchedEffect
        if (server.connection == "online" && server.timeline != "fresh") return@LaunchedEffect
        if (server.navigationTarget.isNotBlank() && server.navigationSerial != handledNavigation) {
            positionReady = true
            return@LaunchedEffect
        }
        val saved = savedPosition
        if (saved != null) {
            val position = server.messages.indexOfFirst { it.id == saved.first }
            if (position < 0) {
                jumping = true
                model.command(
                    Action.JumpToMessage(channel.id, saved.first),
                    url = server.url,
                    onFailure = { jumping = false },
                )
                positionReady = true
                return@LaunchedEffect
            }
            listState.scrollToItem(
                server.messages.lastIndex - position + if (server.canLoadNewer) 1 else 0,
                saved.second,
            )
        } else listState.scrollToItem(0)
        withFrameNanos {}
        followingLatest = nearBottom && !server.browsingHistory
        positionReady = true
    }
    LaunchedEffect(draftKey) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            model.command(Action.BreakReadSpan(channel.id), url = server.url)
            var wasAllowed = false
            snapshotFlow {
                val current = latestServer
                val layout = listState.layoutInfo
                val height =
                    (layout.viewportSize.height -
                            layout.beforeContentPadding -
                            layout.afterContentPadding)
                        .coerceAtLeast(0)
                val messages = current.messages.map { it.id }.toSet()
                val exposed =
                    layout.visibleItemsInfo
                        .filter { row ->
                            val shown =
                                (minOf(row.offset + row.size, height) - maxOf(row.offset, 0))
                                    .coerceAtLeast(0)
                            shown >=
                                minOf(row.size / 2, with(density) { 48.dp.roundToPx() })
                                    .coerceAtLeast(1)
                        }
                        .mapNotNull { it.key as? String }
                        .filter { it in messages }
                val allowed =
                    positionReady &&
                        !jumping &&
                        !returningLatest &&
                        !currentBlocked &&
                        !current.loadingContext &&
                        !current.reading.seekingCatchUp &&
                        (current.navigationTarget.isBlank() ||
                            handledNavigation == current.navigationSerial) &&
                        (current.connection != "online" ||
                            current.timeline == "fresh" && !current.reading.initializing)
                val anchor =
                    if (!listState.isScrollInProgress)
                        layout.visibleItemsInfo
                            .firstOrNull { it.key in exposed }
                            ?.let { it.key as String to -it.offset }
                    else null
                ReadViewport(
                    exposed,
                    nearBottom && !listState.isScrollInProgress && !current.browsingHistory,
                    allowed,
                    current.timeline,
                    current.conversation?.mentions ?: 0,
                    current.notificationCount,
                    anchor,
                    arrived,
                )
            }
                .distinctUntilChanged()
                .collect { report ->
                    if (!report.allowed && wasAllowed)
                        model.command(Action.BreakReadSpan(channel.id), url = server.url)
                    wasAllowed = report.allowed
                    if (report.allowed && report.ids.isNotEmpty()) {
                        model.command(
                            Action.Visible(channel.id, report.ids, report.atLatest, server.me),
                            url = server.url,
                        )
                        arrived = if (report.atLatest) emptySet() else arrived - report.ids.toSet()
                        if (report.atLatest) {
                            followingLatest = true
                            if (latestPhase == LatestPhase.Latest)
                                latestPhase = LatestPhase.FirstNew
                        }
                        report.anchor?.let { (id, offset) ->
                            model.saveReadingPosition(draftKey, id, offset)
                        }
                    }
                }
        }
    }

    DisposableEffect(draftKey) {
        onDispose {
            model.command(Action.CancelCatchUp(channel.id), url = server.url)
            model.command(Action.BreakReadSpan(channel.id), url = server.url)
        }
    }

    fun jumpToMessage(id: String) {
        if (id.isNotBlank()) {
            followingLatest = false
            jumping = true
            focus.clearFocus()
            keyboard?.hide()
            model.command(
                Action.JumpToMessage(channel.id, id),
                url = server.url,
                onFailure = { jumping = false },
            )
        }
    }
    fun openOriginal(id: String) {
        latestPhase = LatestPhase.FirstNew
        jumpToMessage(id)
    }
    fun beginReply(id: String, openKeyboard: Boolean = false) {
        if (reply != id) notifyReply = appearance.replyNotifications
        reply = id
        selected = null
        if (openKeyboard && !recording) {
            emojiPicker = false
            gifPicker = false
            scope.launch {
                withFrameNanos {}
                composerFocus.requestFocus()
                keyboard?.show()
            }
        }
    }
    fun jumpToLatest(armClear: Boolean = false) {
        highlighted = null
        latestPhase = if (armClear) LatestPhase.ClearCatchUp else LatestPhase.FirstNew
        followingLatest = true
        jumping = true
        model.command(Action.BreakReadSpan(channel.id), url = server.url)
        if (server.browsingHistory || server.loadingContext || server.reading.seekingCatchUp) {
            returningLatest = true
            model.command(Action.Latest(channel.id), url = server.url)
        } else
            scope.launch {
                try {
                    listState.animateScrollToItem(0)
                } finally {
                    model.command(Action.BreakReadSpan(channel.id), url = server.url)
                    jumping = false
                }
            }
    }
    fun latestMessages() {
        if (
            appearance.catchUpHourglass &&
                nearBottom &&
                !server.browsingHistory &&
                latestPhase == LatestPhase.ClearCatchUp &&
                server.reading.hourglassTarget.isNotBlank()
        ) {
            model.command(Action.ClearCatchUp(channel.id, server.me), url = server.url)
            latestPhase = LatestPhase.FirstNew
        } else if (arrived.isNotEmpty() && latestPhase == LatestPhase.FirstNew) {
            val target = server.reading.liveIds.firstOrNull { it in arrived } ?: arrived.first()
            latestPhase = LatestPhase.Latest
            jumpToMessage(target)
        } else jumpToLatest(armClear = true)
    }
    LaunchedEffect(
        draftKey,
        server.navigationSerial,
        server.navigationTarget,
        server.loadingContext,
        server.messages.map { it.id },
        server.canLoadNewer,
    ) {
        if (server.navigationTarget.isBlank()) {
            handledNavigation = server.navigationSerial
            return@LaunchedEffect
        }
        if (server.navigationSerial == handledNavigation) return@LaunchedEffect
        val position = server.messages.indexOfFirst { it.id == server.navigationTarget }
        if (position < 0) return@LaunchedEffect
        val index = server.messages.lastIndex - position + if (server.canLoadNewer) 1 else 0
        val layout = listState.layoutInfo
        val item = layout.visibleItemsInfo.firstOrNull { it.key == server.navigationTarget }
        val available =
            (layout.viewportSize.height - layout.beforeContentPadding - layout.afterContentPadding)
                .coerceAtLeast(1)
        if (item == null) {
            // A single animation preserves the direction of travel. Snapping to
            // the row first and then centering it makes an older reply move down.
            listState.animateScrollToItem(index, -available / 3)
        } else if (item.size <= available) {
            // Lazy-list offsets follow the list's scroll direction, including
            // reverseLayout. Leave visible rows still; reveal clipped rows only
            // as far as needed to clear the floating header or composer.
            val distance =
                when {
                    item.offset < 0 -> item.offset
                    item.offset + item.size > available -> item.offset + item.size - available
                    else -> 0
                }
            if (distance != 0) listState.animateScrollBy(distance.toFloat())
        }
        handledNavigation = server.navigationSerial
        model.command(Action.BreakReadSpan(channel.id), url = server.url)
        jumping = false
        highlighted = server.navigationTarget to server.navigationSerial
    }
    LaunchedEffect(draftKey, highlighted) {
        val shown = highlighted ?: return@LaunchedEffect
        delay(1800)
        if (highlighted == shown) highlighted = null
    }
    LaunchedEffect(
        draftKey,
        returningLatest,
        server.browsingHistory,
        server.loadingContext,
        server.reading.seekingCatchUp,
    ) {
        if (
            returningLatest &&
                !server.browsingHistory &&
                !server.loadingContext &&
                !server.reading.seekingCatchUp
        ) {
            withFrameNanos {}
            try {
                listState.animateScrollToItem(0)
            } finally {
                model.command(Action.BreakReadSpan(channel.id), url = server.url)
                returningLatest = false
                jumping = false
            }
        }
    }

    LaunchedEffect(
        draftKey,
        jumping,
        server.navigationTarget,
        server.loadingContext,
        server.error,
    ) {
        if (jumping && !returningLatest && !server.loadingContext && server.error.isNotBlank())
            jumping = false
    }
    LaunchedEffect(draftKey, nearBottom, listState.isScrollInProgress) {
        if (positionReady && !jumping && !returningLatest && !autoFollowing) {
            followingLatest = nearBottom && !listState.isScrollInProgress
            if (listState.isScrollInProgress && latestPhase == LatestPhase.ClearCatchUp)
                latestPhase = LatestPhase.FirstNew
        }
    }

    val submitMessage: (Boolean) -> Unit = { attachmentsOnly ->
        val original = text
        val originalDraft = composer
        val sentEmbeds = embedDrafts
        val sent =
            if (GifChoice.marker(text) != null) text
            else
                Regex("(?m)^giphy:([A-Za-z0-9_-]{1,200})$").replace(text) {
                    "https://giphy.com/gifs/${it.groupValues[1]}"
                }
        val sentFiles = attachmentDraft.files.map { it.path }.toSet()
        submitting = true
        val onResult: (Boolean) -> Unit = { accepted ->
            submitting = false
            if (accepted) {
                model.recordSent(server, draftKey, sent, sentFiles)
                if (composer.text == original && composer.mentions == originalDraft.mentions)
                    updateComposer(ComposerDraft())
                // A send acknowledgement can outlive this composition. Compare the
                // persisted owner draft too, so re-entering and editing cannot be erased.
                if (
                    restoredEmbeds(model.embedDrafts(draftKey)) == sentEmbeds &&
                        restoredEmbeds(embedJson) == sentEmbeds
                )
                    updateEmbeds(emptyList())
                reply = null
                jumpToLatest()
            }
        }
        val sentNotifyReply = notifyReply
        val sentReply = reply
        scope.launch {
            try {
                val canonical =
                    model.encodeMessage(server, originalDraft.text, originalDraft.mentions)
                val encoded =
                    if (GifChoice.marker(canonical) != null) canonical
                    else
                        Regex("(?m)^giphy:([A-Za-z0-9_-]{1,200})$").replace(canonical) {
                            "https://giphy.com/gifs/${it.groupValues[1]}"
                        }
                if (!attachmentsOnly && attachmentDraft.files.isEmpty())
                    model.send(server, encoded, sentReply, sentNotifyReply, sentEmbeds, onResult)
                else
                    model.attachments.send(
                        draftKey,
                        server,
                        encoded,
                        sentReply,
                        sentNotifyReply,
                        sentEmbeds,
                        onResult,
                    )
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                submitting = false
                model.attachments.report(
                    draftKey,
                    e.message ?: ui(R.string.couldn_t_prepare_this_message),
                )
            }
        }
    }

    CompositionLocalProvider(
        LocalMediaViewer provides
            { media ->
                activeMedia = if (media.playback != null) media.id else null
                viewedMedia = media
            },
        LocalViewedMedia provides viewedMedia,
        LocalInlinePlayer provides activeMedia,
        LocalPlaybackBookmarks provides playbackBookmarks,
        LocalMediaVisibilityRegistry provides mediaVisibility,
    ) {
        Box(Modifier.fillMaxSize().imePadding().navigationBarsPadding()) {
            LazyColumn(
                state = listState,
                reverseLayout = true,
                modifier =
                    Modifier.fillMaxSize()
                        .hazeSource(LocalHaze.current)
                        .testTag("message-list")
                        .onGloballyPositioned { listBounds = it.boundsInWindow() },
                contentPadding =
                    PaddingValues(top = headerHeight + 8.dp, bottom = composerHeight + 8.dp),
            ) {
                if (server.messages.isEmpty())
                    item("empty") {
                        EmptyState(
                            if (server.connection == "online")
                                ui(R.string.the_start_of_something_good)
                            else ui(R.string.no_saved_messages_yet),
                            if (server.connection == "online")
                                ui(R.string.say_hello_in_1_s, channel.name)
                            else ui(R.string.messages_will_appear_when_the_server_is_reachable),
                        )
                    }
                if (server.canLoadNewer)
                    item("newer") {
                        TextButton(
                            onClick = { model.command(Action.Newer(channel.id), url = server.url) },
                            enabled = !server.loadingOlder,
                            modifier = Modifier.fillMaxWidth().padding(12.dp),
                        ) {
                            Text(ui(R.string.load_later_messages))
                        }
                    }
                items(server.messages.asReversed(), key = { it.id }) { message ->
                    CompositionLocalProvider(
                        LocalMediaViewport provides viewport,
                        LocalTimelineMediaVisible provides
                            (message.id in visibleRows &&
                                viewedMedia == null &&
                                !embedEditor &&
                                edit == null &&
                                !pins &&
                                !overlayBlocking),
                        LocalMediaRowId provides message.id,
                    ) {
                        Column {
                            if (
                                appearance.catchUpHourglass && message.id == server.reading.gapStart
                            )
                                UnreadDivider(ui(R.string.unread_below), "unread-gap-start")
                            else if (message.id == entryUnread)
                                UnreadDivider(ui(R.string.new_messages), "unread-new")
                            MessageRow(
                                message,
                                server,
                                model,
                                grouped = message.id in groupedMessages,
                                canReply =
                                    !recording &&
                                        !submitting &&
                                        !attachmentDraft.sending &&
                                        !overlayBlocking &&
                                        viewedMedia == null,
                                onReply = { beginReply(message.id, true) },
                                highlight = highlighted?.first == message.id,
                                onMention = { person ->
                                    updateComposer(composer.mention(person))
                                    if (!recording) {
                                        composerFocus.requestFocus()
                                        keyboard?.show()
                                    }
                                },
                                onJump = ::openOriginal,
                                onActions = { selected = message },
                                onReaction = { r ->
                                    model.react(
                                        server,
                                        message,
                                        EmojiChoice(
                                            r.emoji,
                                            r.custom,
                                            server.emojis.firstOrNull { it.id == r.emoji }?.name
                                                ?: r.emoji,
                                        ),
                                    )
                                },
                                onReactionPeople = { r ->
                                    reactionPeople =
                                        message.id to
                                            EmojiChoice(
                                                r.emoji,
                                                r.custom,
                                                server.emojis.firstOrNull { it.id == r.emoji }?.name
                                                    ?: r.emoji,
                                            )
                                },
                                media = {
                                    MessageMedia(
                                        message,
                                        server,
                                        model,
                                        activeMedia,
                                        { activeMedia = it },
                                        { selected = message },
                                    )
                                },
                            )
                            if (appearance.catchUpHourglass && message.id == server.reading.gapEnd)
                                UnreadDivider(ui(R.string.unread_above), "unread-gap-end")
                        }
                    }
                }
                if (server.canLoadOlder)
                    item("history") {
                        TextButton(
                            onClick = { model.command(Action.Older) },
                            enabled = !server.loadingOlder,
                            modifier = Modifier.fillMaxWidth().padding(12.dp),
                        ) {
                            if (server.loadingOlder)
                                CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                            else Text(ui(R.string.load_earlier_messages))
                        }
                    }
            }

            // Content can continue behind glass, while system status symbols keep
            // a quiet, legible background even when a message scrolls under them.
            Box(
                Modifier.align(Alignment.TopCenter)
                    .fillMaxWidth()
                    .height(WindowInsets.statusBars.asPaddingValues().calculateTopPadding() + 20.dp)
                    .background(
                        Brush.verticalGradient(
                            listOf(
                                MaterialTheme.colorScheme.background,
                                MaterialTheme.colorScheme.background,
                                Color.Transparent,
                            )
                        )
                    )
            )
            Column(
                Modifier.align(Alignment.TopCenter)
                    .statusBarsPadding()
                    .padding(horizontal = 12.dp, vertical = 8.dp)
                    .onSizeChanged { headerHeight = with(density) { it.height.toDp() } + 32.dp }
            ) {
                GlassPill(Modifier.fillMaxWidth()) {
                    Column(
                        Modifier.fillMaxWidth()
                            .testTag("channel-header")
                            .animateContentSize()
                            .clickable(
                                role = Role.Button,
                                onClickLabel =
                                    if (descriptionExpanded) ui(R.string.hide_channel_description)
                                    else ui(R.string.show_channel_description),
                            ) {
                                descriptionTouched = true
                                descriptionExpanded = !descriptionExpanded
                            }
                            .semantics {
                                stateDescription =
                                    if (descriptionExpanded) ui(R.string.expanded)
                                    else ui(R.string.collapsed)
                            }
                    ) {
                        Row(
                            Modifier.fillMaxWidth().padding(6.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Box(Modifier.width(96.dp)) {
                                IconButton(onClick = onBack) {
                                    Icon(
                                        Icons.Outlined.ArrowBack,
                                        ui(R.string.back_to_conversations),
                                    )
                                }
                            }
                            Text(
                                channel.name,
                                modifier = Modifier.weight(1f).padding(horizontal = 4.dp),
                                textAlign = TextAlign.Center,
                                style = MaterialTheme.typography.titleMedium,
                                fontWeight = FontWeight.SemiBold,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            IconButton(
                                onClick = {
                                    keyboard?.hide()
                                    focus.clearFocus()
                                    pins = true
                                }
                            ) {
                                Icon(Icons.Outlined.PushPin, ui(R.string.pinned_messages))
                            }
                            IconButton(onClick = { members = true }) {
                                Icon(Icons.Outlined.PeopleOutline, ui(R.string.community_members))
                            }
                        }
                        if (descriptionExpanded) {
                            Text(
                                channel.topic.ifBlank {
                                    if (channel.kind == "direct")
                                        ui(R.string.direct_conversation_with_1_s, channel.name)
                                    else ui(R.string.no_channel_description_yet)
                                },
                                modifier =
                                    Modifier.fillMaxWidth()
                                        .padding(horizontal = 20.dp)
                                        .padding(top = 2.dp, bottom = 18.dp)
                                        .heightIn(max = 160.dp)
                                        .verticalScroll(rememberScrollState()),
                                textAlign = TextAlign.Center,
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
                if (server.connection != "online")
                    GlassPill(Modifier.padding(top = 8.dp), radius = 24) {
                        ConnectionBanner(server.connection) { model.command(Action.Refresh) }
                    }
            }

            Column(
                Modifier.align(Alignment.BottomCenter)
                    .fillMaxWidth()
                    .onSizeChanged { composerHeight = with(density) { it.height.toDp() } }
                    .testTag("chat-composer")
                    .padding(12.dp)
            ) {
                if (server.loadingContext || server.reading.seekingCatchUp)
                    GlassPill(Modifier.fillMaxWidth().padding(bottom = 8.dp), radius = 24) {
                        Row(
                            Modifier.padding(start = 16.dp, end = 6.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                            Text(
                                if (server.reading.seekingCatchUp)
                                    ui(R.string.loading_earlier_unread_messages)
                                else ui(R.string.opening_original_message),
                                style = MaterialTheme.typography.bodySmall,
                                modifier = Modifier.weight(1f),
                            )
                            TextButton(onClick = { jumpToLatest() }) { Text(ui(R.string.cancel)) }
                        }
                    }
                if (composerFocused && !recording && !emojiPicker && !gifPicker)
                    ComposerSuggestions(composer, server, model) { updateComposer(it) }
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    verticalAlignment = Alignment.Bottom,
                ) {
                    AttachmentActions(
                        draftKey,
                        model,
                        enabled =
                            !recording &&
                                !submitting &&
                                !attachmentDraft.sending &&
                                !attachmentDraft.preparing,
                        onRecord = {
                            focus.clearFocus()
                            keyboard?.hide()
                            recording = true
                        },
                        modifier = Modifier.zIndex(1f),
                    )
                    Column(Modifier.weight(1f)) {
                        TypingIndicator(server.typing, server.typingPhrase)
                        GlassPill(radius = 28) {
                            if (recording)
                                RecordingComposer(
                                    draftKey,
                                    model,
                                    onClose = { recording = false },
                                    onSend = {
                                        if (attachmentDraft.uncertain) confirmAttachmentRetry = true
                                        else submitMessage(true)
                                    },
                                )
                            else
                                Column {
                                    PendingAttachments(
                                        attachmentDraft,
                                        edit = { file ->
                                            focus.clearFocus()
                                            keyboard?.hide()
                                            attachmentOptions = file
                                        },
                                    ) {
                                        model.attachments.remove(draftKey, it)
                                    }
                                    if (embedDrafts.isNotEmpty()) {
                                        val valid =
                                            remember(embedDrafts, attachmentDraft.files) {
                                                validateEmbedDrafts(
                                                        embedDrafts,
                                                        attachmentDraft.files.map { it.wireName },
                                                    )
                                                    .isEmpty()
                                            }
                                        AssistChip(
                                            onClick = {
                                                focus.clearFocus()
                                                keyboard?.hide()
                                                embedEditor = true
                                            },
                                            label = {
                                                Text(
                                                    uiQuantity(
                                                        if (valid) R.plurals.embed_draft_count
                                                        else R.plurals.embed_draft_needs_attention,
                                                        embedDrafts.size,
                                                    )
                                                )
                                            },
                                            leadingIcon = {
                                                Icon(
                                                    if (valid) Icons.Outlined.ViewAgenda
                                                    else Icons.Outlined.WarningAmber,
                                                    null,
                                                )
                                            },
                                            modifier =
                                                Modifier.padding(start = 14.dp)
                                                    .testTag("composer-embeds"),
                                        )
                                    }
                                    if (reply != null)
                                        Row(
                                            Modifier.testTag("composer-reply")
                                                .clickable(
                                                    role = Role.Button,
                                                    onClickLabel =
                                                        ui(R.string.open_original_message),
                                                ) {
                                                    reply?.let(::openOriginal)
                                                }
                                                .padding(start = 18.dp, top = 8.dp),
                                            verticalAlignment = Alignment.CenterVertically,
                                        ) {
                                            Icon(
                                                Icons.Outlined.Reply,
                                                null,
                                                Modifier.size(18.dp),
                                                tint = MaterialTheme.colorScheme.primary,
                                            )
                                            Text(
                                                server.messages
                                                    .find { it.id == reply }
                                                    ?.let {
                                                        ui(
                                                            R.string.reply_to_1_s_2_s,
                                                            it.author,
                                                            it.previewText(),
                                                        )
                                                    } ?: ui(R.string.reply_to_message),
                                                maxLines = 1,
                                                overflow = TextOverflow.Ellipsis,
                                                style = MaterialTheme.typography.labelMedium,
                                                modifier =
                                                    Modifier.weight(1f).padding(start = 8.dp),
                                            )
                                            IconToggleButton(
                                                checked = notifyReply,
                                                onCheckedChange = { notifyReply = it },
                                                modifier =
                                                    Modifier.testTag("reply-notification-toggle"),
                                            ) {
                                                Icon(
                                                    if (notifyReply) Icons.Outlined.AlternateEmail
                                                    else Icons.Outlined.NotificationsOff,
                                                    if (notifyReply) ui(R.string.reply_mention_on)
                                                    else ui(R.string.reply_mention_off),
                                                    Modifier.size(18.dp),
                                                    tint =
                                                        if (notifyReply)
                                                            MaterialTheme.colorScheme.primary
                                                        else
                                                            MaterialTheme.colorScheme
                                                                .onSurfaceVariant,
                                                )
                                            }
                                            IconButton(onClick = { reply = null }) {
                                                Icon(
                                                    Icons.Outlined.Close,
                                                    ui(R.string.cancel_reply),
                                                    Modifier.size(18.dp),
                                                )
                                            }
                                        }
                                    Row(
                                        Modifier.padding(6.dp),
                                        verticalAlignment = Alignment.CenterVertically,
                                    ) {
                                        ComposerInput(
                                            composer,
                                            server,
                                            model,
                                            ui(
                                                R.string.message_to,
                                                if (channel.kind == "direct") channel.name
                                                else "#${channel.name}",
                                            ),
                                            onChange = {
                                                updateComposer(it)
                                                model.typing(server)
                                            },
                                            modifier =
                                                Modifier.weight(1f).focusRequester(composerFocus),
                                            onFocusChange = { composerFocused = it },
                                            animationsAllowed =
                                                appearance.animations &&
                                                    viewedMedia == null &&
                                                    !emojiPicker &&
                                                    !gifPicker,
                                        )
                                        IconButton(
                                            onClick = {
                                                focus.clearFocus()
                                                keyboard?.hide()
                                                emojiTarget = null
                                                emojiPicker = true
                                            },
                                            enabled = !submitting && !attachmentDraft.sending,
                                            modifier = Modifier.size(40.dp),
                                        ) {
                                            Icon(
                                                Icons.Outlined.SentimentSatisfiedAlt,
                                                ui(R.string.emoji_and_stickers),
                                                Modifier.size(22.dp),
                                            )
                                        }
                                        IconButton(
                                            onClick = {
                                                focus.clearFocus()
                                                keyboard?.hide()
                                                gifPicker = true
                                            },
                                            enabled =
                                                !submitting &&
                                                    !attachmentDraft.sending &&
                                                    !gifPreparing,
                                            modifier = Modifier.size(40.dp),
                                        ) {
                                            Icon(
                                                Icons.Outlined.GifBox,
                                                ui(R.string.gif_picker),
                                                Modifier.size(24.dp),
                                            )
                                        }
                                        ComposerSendButton(
                                            canSend =
                                                text.isNotBlank() ||
                                                    attachmentDraft.files.isNotEmpty() ||
                                                    embedDrafts.isNotEmpty(),
                                            enabled =
                                                !submitting &&
                                                    !attachmentDraft.sending &&
                                                    !attachmentDraft.preparing &&
                                                    !gifPreparing,
                                            busy = submitting || attachmentDraft.sending,
                                            onEmbeds = {
                                                focus.clearFocus()
                                                keyboard?.hide()
                                                embedEditor = true
                                            },
                                            onSend = {
                                                if (
                                                    validateEmbedDrafts(
                                                            embedDrafts,
                                                            attachmentDraft.files.map {
                                                                it.wireName
                                                            },
                                                        )
                                                        .isNotEmpty()
                                                )
                                                    embedEditor = true
                                                else if (
                                                    attachmentDraft.files.isNotEmpty() &&
                                                        attachmentDraft.uncertain
                                                )
                                                    confirmAttachmentRetry = true
                                                else submitMessage(false)
                                            },
                                        )
                                    }
                                }
                        }
                    }
                }
            }
            Column(
                Modifier.align(Alignment.BottomEnd)
                    .padding(end = 18.dp, bottom = composerHeight + 8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
                horizontalAlignment = Alignment.End,
            ) {
                if (server.reading.mentions.isNotEmpty())
                    BadgedBox(
                        badge = {
                            if (server.reading.mentions.size > 1)
                                Badge(Modifier.testTag("mention-count")) {
                                    Text(shortcutCount(server.reading.mentions.size))
                                }
                        }
                    ) {
                        GlassPill(radius = 18) {
                            IconButton(
                                onClick = { openOriginal(server.reading.mentions.first()) },
                                modifier = Modifier.size(48.dp).testTag("mention-shortcut"),
                            ) {
                                Icon(
                                    Icons.Outlined.Notifications,
                                    uiQuantity(
                                        R.plurals.mention_count,
                                        server.reading.mentions.size,
                                    ),
                                )
                            }
                        }
                    }
                if (appearance.catchUpHourglass && server.reading.hourglassTarget.isNotBlank())
                    GlassPill(radius = 18) {
                        IconButton(
                            onClick = {
                                followingLatest = false
                                latestPhase = LatestPhase.FirstNew
                                jumping = true
                                focus.clearFocus()
                                keyboard?.hide()
                                model.command(
                                    Action.CatchUp(channel.id, server.me),
                                    url = server.url,
                                    onFailure = { jumping = false },
                                )
                            },
                            modifier = Modifier.size(48.dp).testTag("hourglass-shortcut"),
                        ) {
                            Icon(
                                Icons.Outlined.HourglassEmpty,
                                ui(R.string.go_to_oldest_unread_message_in_catch_up_block),
                            )
                        }
                    }
                if (
                    (!nearBottom ||
                        server.browsingHistory ||
                        appearance.catchUpHourglass &&
                            server.reading.hourglassTarget.isNotBlank()) &&
                        server.messages.isNotEmpty()
                )
                    BadgedBox(
                        badge = {
                            if (arrived.isNotEmpty())
                                Badge(Modifier.testTag("latest-count")) {
                                    Text(shortcutCount(arrived.size))
                                }
                        }
                    ) {
                        GlassPill(Modifier.testTag("latest-messages"), radius = 18) {
                            IconButton(
                                onClick = ::latestMessages,
                                modifier = Modifier.size(48.dp),
                            ) {
                                Icon(
                                    Icons.Outlined.ArrowDownward,
                                    if (
                                        appearance.catchUpHourglass &&
                                            nearBottom &&
                                            latestPhase == LatestPhase.ClearCatchUp &&
                                            server.reading.hourglassTarget.isNotBlank()
                                    )
                                        ui(R.string.clear_catch_up_block)
                                    else if (
                                        arrived.isNotEmpty() && latestPhase == LatestPhase.FirstNew
                                    )
                                        ui(R.string.go_to_first_new_message)
                                    else ui(R.string.latest_messages),
                                )
                            }
                        }
                    }
            }
        }

        if (pins)
            PinnedMessages(server, model, onDismiss = { pins = false }, onOpen = ::openOriginal)
        if (emojiPicker)
            EmojiPicker(
                server,
                model,
                onDismiss = { emojiPicker = false },
                onPick = { emoji ->
                    val target = emojiTarget
                    if (target != null) {
                        model.react(
                            server,
                            server.messages.firstOrNull { it.id == target.id } ?: target,
                            emoji,
                        )
                        emojiPicker = false
                    } else updateComposer(composer.insert(emoji.token))
                },
            )
        if (gifPicker)
            GifPicker(
                server,
                model,
                onDismiss = { gifPicker = false },
                onPick = { gif ->
                    gifPreparing = true
                    model
                        .pickGif(
                            server,
                            draftKey,
                            gif,
                            onMarker = { marker ->
                                val next =
                                    if (text.isBlank()) marker
                                    else
                                        text.trimEnd() +
                                            "\nhttps://giphy.com/gifs/${gif.providerId}"
                                updateComposer(
                                    composer.edited(
                                        androidx.compose.ui.text.input.TextFieldValue(
                                            next,
                                            androidx.compose.ui.text.TextRange(next.length),
                                        )
                                    )
                                )
                            },
                            onDone = {
                                gifPreparing = false
                                gifPicker = false
                            },
                        )
                        .invokeOnCompletion { scope.launch { gifPreparing = false } }
                },
            )
        reactionPeople?.let { (id, emoji) ->
            ReactionPeople(server, model, id, emoji, onDismiss = { reactionPeople = null })
        }

        if (confirmAttachmentRetry)
            AlertDialog(
                onDismissRequest = { confirmAttachmentRetry = false },
                title = { Text(ui(R.string.retry_sending_attachments)) },
                text = {
                    Text(ui(R.string.upload_retry_confirmation))
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            confirmAttachmentRetry = false
                            submitMessage(false)
                        }
                    ) {
                        Text(ui(R.string.send_again))
                    }
                },
                dismissButton = {
                    TextButton(onClick = { confirmAttachmentRetry = false }) {
                        Text(ui(R.string.cancel))
                    }
                },
            )

        selected?.let { stale ->
            val message = server.messages.firstOrNull { it.id == stale.id } ?: stale
            ModalBottomSheet(onDismissRequest = { selected = null }) {
                Column(
                    Modifier.navigationBarsPadding()
                        .heightIn(max = 640.dp)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 16.dp)
                ) {
                    Text(
                        "${message.author} · ${remember(message.at,AppLanguage.locale) {SimpleDateFormat("dd MMM yyyy · HH:mm",AppLanguage.locale).format(Date(message.at))}}",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                    Text(
                        message.content.ifBlank { ui(R.string.message) },
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                    )
                    if (!message.pending) {
                        val favorites =
                            expressions.favoriteStickers.mapNotNull { id ->
                                server.emojis
                                    .firstOrNull { it.id == id }
                                    ?.let { EmojiChoice(it.id, true, it.name) }
                            }
                        val frequent =
                            favorites +
                                (expressions.usage
                                        .map { it.first }
                                        .filter {
                                            !it.custom ||
                                                server.emojis.any { e -> e.id == it.value }
                                        } +
                                        server.emojis.map { EmojiChoice(it.id, true, it.name) } +
                                        listOf("👍", "❤️", "😂", "🎉", "👀").map {
                                            EmojiChoice(it)
                                        })
                                    .distinctBy { it.key }
                                    .filter { e -> favorites.none { it.key == e.key } }
                                    .take(5)
                        Row(
                            Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            LazyRow(
                                Modifier.weight(1f),
                                horizontalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                items(frequent, key = { it.key }) { emoji ->
                                    IconButton(
                                        onClick = {
                                            model.react(server, message, emoji)
                                            selected = null
                                        },
                                        modifier =
                                            Modifier.semantics {
                                                contentDescription =
                                                    ui(R.string.react_with_1_s, emoji.token)
                                            },
                                    ) {
                                        EmojiGlyph(server, emoji, 28, model)
                                    }
                                }
                            }
                            IconButton(
                                onClick = {
                                    emojiTarget = message
                                    selected = null
                                    emojiPicker = true
                                }
                            ) {
                                Icon(Icons.Outlined.AddReaction, ui(R.string.more_reactions))
                            }
                        }
                        SheetAction(ui(R.string.reply), Icons.Outlined.Reply) {
                            beginReply(message.id)
                        }
                    }
                    PinMessageAction(
                        server,
                        model,
                        channel.id,
                        server.messages.firstOrNull { it.id == message.id } ?: message,
                    ) {
                        selected = null
                    }
                    SheetAction(ui(R.string.copy_text), Icons.Outlined.ContentCopy) {
                        clipboard.setText(AnnotatedString(message.content))
                        selected = null
                    }
                    MessageLinkActions(message)
                    if (message.canEdit) {
                        SheetAction(ui(R.string.edit_message), Icons.Outlined.Edit) {
                            edit = message
                            selected = null
                        }
                    }
                    if (message.canDelete) {
                        SheetAction(ui(R.string.delete_message), Icons.Outlined.DeleteOutline) {
                            delete = message
                            selected = null
                        }
                    }
                    if (message.delivery in listOf("failed", "unknown")) {
                        SheetAction(ui(R.string.retry_sending), Icons.Outlined.Refresh) {
                            retry = message
                            selected = null
                        }
                        SheetAction(
                            ui(R.string.discard_unsent_message),
                            Icons.Outlined.DeleteOutline,
                        ) {
                            model.command(Action.Dismiss(message.id))
                            selected = null
                        }
                    }
                    Spacer(Modifier.height(16.dp))
                }
            }
        }
        edit?.let { message ->
            EditMessageDialog(message, server, model, onDismiss = { edit = null })
        }
        delete?.let { message ->
            DeleteMessageDialog(
                message,
                server,
                model,
                onDismiss = { delete = null },
                onDeleted = { delete = null },
            )
        }
        retry?.let { message ->
            AlertDialog(
                onDismissRequest = { retry = null },
                title = { Text(ui(R.string.retry_sending_2)) },
                text = {
                    Text(
                        if (message.delivery == "unknown") ui(R.string.message_retry_confirmation)
                        else ui(R.string.try_sending_this_message_again)
                    )
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            model.command(Action.Retry(message.id))
                            retry = null
                        }
                    ) {
                        Text(ui(R.string.retry))
                    }
                },
                dismissButton = {
                    TextButton(onClick = { retry = null }) { Text(ui(R.string.cancel)) }
                },
            )
        }
        if (members)
            ModalBottomSheet(onDismissRequest = { members = false }) {
                Text(
                    ui(R.string.people_in_1_s, server.name),
                    style = MaterialTheme.typography.titleLarge,
                    modifier = Modifier.padding(horizontal = 24.dp, vertical = 12.dp),
                )
                LazyColumn(
                    Modifier.navigationBarsPadding(),
                    contentPadding = PaddingValues(bottom = 24.dp),
                ) {
                    items(server.members, key = { it.id }) { person ->
                        ListItem(
                            headlineContent = {
                                Text(
                                    person.name,
                                    color = userNameColor(person.id),
                                    modifier = Modifier.testTag("member-name-${person.id}"),
                                )
                            },
                            supportingContent = { Text(person.presence) },
                            leadingContent = { UserAvatar(person.id, person.name) },
                            modifier =
                                Modifier.clickable {
                                    members = false
                                    openProfile(person.id)
                                },
                        )
                    }
                }
            }
    }
    if (embedEditor)
        EmbedDraftEditor(
            embedDrafts,
            attachmentDraft.files,
            server,
            model,
            onApply = {
                updateEmbeds(it)
                embedEditor = false
            },
            onDismiss = { embedEditor = false },
        )
    attachmentOptions?.let { file ->
        AttachmentOptions(file, draftKey, server, model, onDismiss = { attachmentOptions = null })
    }
    viewedMedia?.let { media ->
        CompositionLocalProvider(LocalPlaybackBookmarks provides playbackBookmarks) {
            MediaViewer(server, model, media) { viewedMedia = null }
        }
    }
}

@Composable
internal fun SheetAction(
    label: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    action: () -> Unit,
) {
    ListItem(
        headlineContent = { Text(label) },
        leadingContent = { Icon(icon, null) },
        modifier = Modifier.clip(RoundedCornerShape(16.dp)).clickable(onClick = action),
    )
}

private data class ReadViewport(
    val ids: List<String>,
    val atLatest: Boolean,
    val allowed: Boolean,
    val timeline: String,
    val mentions: Int,
    val notifications: UInt,
    val anchor: Pair<String, Int>?,
    val arrivals: Set<String>,
)

@Composable
private fun UnreadDivider(label: String, tag: String) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        HorizontalDivider(
            Modifier.weight(1f),
            color = MaterialTheme.colorScheme.primary.copy(alpha = .4f),
        )
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.primary,
        )
        HorizontalDivider(
            Modifier.weight(1f),
            color = MaterialTheme.colorScheme.primary.copy(alpha = .4f),
        )
    }
}

private fun shortcutCount(count: Int) = if (count > 99) "99+" else count.toString()

private enum class LatestPhase {
    FirstNew,
    Latest,
    ClearCatchUp,
}
