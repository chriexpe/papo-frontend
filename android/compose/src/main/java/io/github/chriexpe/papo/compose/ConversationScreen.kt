@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class, androidx.compose.foundation.ExperimentalFoundationApi::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.animateScrollBy
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import kotlinx.coroutines.delay
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.lazy.layout.LazyLayoutCacheWindow
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.zIndex
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.repeatOnLifecycle
import dev.chrisbanes.haze.hazeSource
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import io.github.chriexpe.papo.core.Action

@Composable
internal fun ConversationScreen(server: ServerState, model: PapoViewModel, onBack: () -> Unit) {
    val channel = server.conversation ?: return
    val draftKey = "${server.url}/${server.me}/${channel.id}"
    var composer by rememberSaveable(draftKey,stateSaver=ComposerDraft.Saver) { mutableStateOf(ComposerDraft.restore(model.draft(draftKey),model.draftMentions(draftKey))) }
    val text=composer.text
    fun updateComposer(next:ComposerDraft) { composer=next; model.saveDraft(draftKey,next.text,next.json()) }

    var reply by rememberSaveable(draftKey) { mutableStateOf<String?>(null) }
    var emojiPicker by remember(draftKey) { mutableStateOf(false) }
    var emojiTarget by remember(draftKey) { mutableStateOf<ChatMessage?>(null) }
    var gifPicker by remember(draftKey) { mutableStateOf(false) }
    var gifPreparing by remember(draftKey) { mutableStateOf(false) }
    var reactionPeople by remember(draftKey) { mutableStateOf<Pair<String,EmojiChoice>?>(null) }
    val expressionStates by model.expressions.states.collectAsState()
    val expressions = expressionStates[model.expressions.owner(server)] ?: model.expressions.state(model.expressions.owner(server))
    var selected by remember { mutableStateOf<ChatMessage?>(null) }
    var edit by remember { mutableStateOf<ChatMessage?>(null) }
    var delete by remember { mutableStateOf<ChatMessage?>(null) }
    var retry by remember { mutableStateOf<ChatMessage?>(null) }
    var members by remember { mutableStateOf(false) }
    var descriptionExpanded by rememberSaveable(draftKey) { mutableStateOf(false) }
    var activeMedia by remember(draftKey) { mutableStateOf<String?>(null) }
    var viewedMedia by remember(draftKey) {mutableStateOf<ViewerMedia?>(null)}
    var submitting by remember(draftKey) { mutableStateOf(false) }
    var recording by remember(draftKey) { mutableStateOf(false) }
    var composerFocused by remember(draftKey) {mutableStateOf(false)}
    var confirmAttachmentRetry by remember(draftKey) { mutableStateOf(false) }
    val attachmentDrafts by model.attachments.drafts.collectAsState()
    val attachmentDraft = attachmentDrafts[draftKey] ?: AttachmentDraft()
    val listState = rememberLazyListState(cacheWindow=remember {LazyLayoutCacheWindow(aheadFraction=2f,behindFraction=2f)})
    val playbackBookmarks=remember(draftKey) {PlaybackBookmarks()}
    val mediaVisibility=remember(draftKey) {MediaVisibilityRegistry()}
    var listBounds by remember {mutableStateOf(Rect.Zero)}
    val scope = rememberCoroutineScope()
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val mediaLifecycle by lifecycle.currentStateFlow.collectAsState()
    val density = LocalDensity.current
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    var headerHeight by remember { mutableStateOf(90.dp) }
    var composerHeight by remember { mutableStateOf(90.dp) }
    var handledNavigation by remember(draftKey) {mutableStateOf(0uL)}
    var highlighted by remember(draftKey) {mutableStateOf<Pair<String,ULong>?>(null)}
    var returningLatest by remember(draftKey) {mutableStateOf(false)}
    val nearBottom by remember { derivedStateOf { listState.firstVisibleItemIndex == 0 && listState.firstVisibleItemScrollOffset < 100 } }
    val clipboard = LocalClipboardManager.current
    val latestServer by rememberUpdatedState(server)
    val viewport=with(density) {Rect(listBounds.left,listBounds.top+(headerHeight+8.dp).toPx(),listBounds.right,(listBounds.bottom-(composerHeight+8.dp).toPx()).coerceAtLeast(listBounds.top+(headerHeight+8.dp).toPx()))}
    val visibleRows by remember {derivedStateOf {
        val layout=listState.layoutInfo
        val height=(layout.viewportSize.height-layout.beforeContentPadding-layout.afterContentPadding).coerceAtLeast(0)
        layout.visibleItemsInfo.filter {it.offset<height && it.offset+it.size>0}.mapNotNull {it.key as? String}.toSet()
    }}
    LaunchedEffect(mediaVisibility,listState) {
        snapshotFlow {Triple(listState.layoutInfo,latestServer.messages.asReversed().map {it.id},viewedMedia==null && mediaLifecycle.isAtLeast(Lifecycle.State.RESUMED))}.collect {(layout,order,enabled) ->
            val height=(layout.viewportSize.height-layout.beforeContentPadding-layout.afterContentPadding).coerceAtLeast(0)
            kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Main.immediate) {
                mediaVisibility.update(layout.visibleItemsInfo.filter {it.offset<height && it.offset+it.size>0}.mapNotNull {it.key as? String}.toSet(),enabled)
                mediaVisibility.trim(layout,order)
            }
        }
    }
    DisposableEffect(mediaVisibility) {onDispose {mediaVisibility.close()}}

    LaunchedEffect(draftKey, server.messages.lastOrNull()?.id) {
        if (nearBottom && !server.loadingOlder && !server.browsingHistory && !server.loadingContext &&
            (server.navigationTarget.isBlank() || handledNavigation==server.navigationSerial)) listState.animateScrollToItem(0)
    }
    LaunchedEffect(draftKey) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            snapshotFlow {
                val ids = latestServer.messages.map { it.id }.toSet()
                listState.layoutInfo.visibleItemsInfo.mapNotNull { it.key as? String }.filter { it in ids }
            }.distinctUntilChanged().collect { ids ->
                if (ids.isNotEmpty()) model.command(Action.Visible(channel.id, ids), url = server.url)
            }
        }
    }

    fun openOriginal(id:String) {
        if(id.isNotBlank()) {focus.clearFocus();keyboard?.hide();model.command(Action.JumpToMessage(channel.id,id),url=server.url)}
    }
    fun latestMessages() {
        highlighted=null
        if(server.browsingHistory || server.loadingContext) {returningLatest=true;model.command(Action.Latest(channel.id),url=server.url)}
        else scope.launch {listState.animateScrollToItem(0)}
    }
    LaunchedEffect(draftKey,server.navigationSerial,server.navigationTarget,server.loadingContext,server.messages.map {it.id},server.canLoadNewer) {
        if(server.navigationTarget.isBlank() || server.navigationSerial==handledNavigation) return@LaunchedEffect
        val position=server.messages.indexOfFirst {it.id==server.navigationTarget}
        if(position<0) return@LaunchedEffect
        val index=server.messages.lastIndex-position+if(server.canLoadNewer) 1 else 0
        val layout=listState.layoutInfo
        val item=layout.visibleItemsInfo.firstOrNull {it.key==server.navigationTarget}
        val available=(layout.viewportSize.height-layout.beforeContentPadding-layout.afterContentPadding).coerceAtLeast(1)
        if(item==null) {
            // A single animation preserves the direction of travel. Snapping to
            // the row first and then centering it makes an older reply move down.
            listState.animateScrollToItem(index,-available/3)
        } else if(item.size<=available) {
            // Lazy-list offsets follow the list's scroll direction, including
            // reverseLayout. Leave visible rows still; reveal clipped rows only
            // as far as needed to clear the floating header or composer.
            val distance=when {
                item.offset<0 -> item.offset
                item.offset+item.size>available -> item.offset+item.size-available
                else -> 0
            }
            if(distance!=0) listState.animateScrollBy(distance.toFloat())
        }
        handledNavigation=server.navigationSerial
        highlighted=server.navigationTarget to server.navigationSerial
    }
    LaunchedEffect(draftKey,highlighted) {
        val shown=highlighted ?: return@LaunchedEffect
        delay(1800)
        if(highlighted==shown) highlighted=null
    }
    LaunchedEffect(draftKey,returningLatest,server.browsingHistory,server.loadingContext) {
        if(returningLatest && !server.browsingHistory && !server.loadingContext) {
            withFrameNanos { }
            try {listState.animateScrollToItem(0)} finally {returningLatest=false}
        }
    }

    val submitMessage: (Boolean) -> Unit = { attachmentsOnly ->
        val original = text
        val originalDraft = composer
        val sent = if(GifChoice.marker(text)!=null) text else Regex("(?m)^giphy:([A-Za-z0-9_-]{1,200})$").replace(text) {"https://giphy.com/gifs/${it.groupValues[1]}"}
        val sentFiles = attachmentDraft.files.map {it.path}.toSet()
        submitting = true
        val onResult: (Boolean) -> Unit = { accepted ->
            submitting = false
            if (accepted) {
                model.recordSent(server,draftKey,sent,sentFiles)
                if (composer.text == original && composer.mentions == originalDraft.mentions) updateComposer(ComposerDraft())
                reply = null
                latestMessages()
            }
        }
        val sentReply = reply
        scope.launch {
            try {
                val canonical=model.encodeMessage(server,originalDraft.text,originalDraft.mentions)
                val encoded=if(GifChoice.marker(canonical)!=null) canonical else Regex("(?m)^giphy:([A-Za-z0-9_-]{1,200})$").replace(canonical) {"https://giphy.com/gifs/${it.groupValues[1]}"}
                if (!attachmentsOnly && attachmentDraft.files.isEmpty()) model.send(server,encoded,sentReply,onResult)
                else model.attachments.send(draftKey,server,encoded,sentReply,onResult)
            } catch(e:kotlinx.coroutines.CancellationException) {throw e}
            catch(e:Exception) {submitting=false; model.attachments.report(draftKey,e.message ?: "Couldn’t prepare this message")}
        }
    }

    CompositionLocalProvider(LocalMediaViewer provides {media ->activeMedia=if(media.playback!=null) media.id else null;viewedMedia=media},LocalViewedMedia provides viewedMedia,LocalInlinePlayer provides activeMedia,LocalPlaybackBookmarks provides playbackBookmarks,LocalMediaVisibilityRegistry provides mediaVisibility) {
    Box(Modifier.fillMaxSize().imePadding().navigationBarsPadding()) {
        LazyColumn(state = listState, reverseLayout = true,
            modifier = Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("message-list").onGloballyPositioned {listBounds=it.boundsInWindow()},
            contentPadding = PaddingValues(top = headerHeight + 8.dp, bottom = composerHeight + 8.dp),
        ) {
            if (server.messages.isEmpty()) item("empty") {
                EmptyState(if (server.connection == "online") "The start of something good." else "No saved messages yet.",
                    if (server.connection == "online") "Say hello in ${channel.name}." else "Messages will appear when the server is reachable.")
            }
            if(server.canLoadNewer) item("newer") {
                TextButton(onClick={model.command(Action.Newer(channel.id),url=server.url)},enabled=!server.loadingOlder,modifier=Modifier.fillMaxWidth().padding(12.dp)) {Text("Load later messages")}
            }
            items(server.messages.asReversed(), key = { it.id }) { message ->
                CompositionLocalProvider(LocalMediaViewport provides viewport,LocalTimelineMediaVisible provides (message.id in visibleRows && viewedMedia==null),LocalMediaRowId provides message.id) {
                MessageRow(message, server, model, highlight=highlighted?.first==message.id, onJump=::openOriginal, onActions = { selected = message }, onReaction = { r ->
                    model.react(server,message,EmojiChoice(r.emoji,r.custom,server.emojis.firstOrNull {it.id==r.emoji}?.name ?: r.emoji))
                }, onReactionPeople = { r -> reactionPeople = message.id to EmojiChoice(r.emoji,r.custom,server.emojis.firstOrNull {it.id==r.emoji}?.name ?: r.emoji) }, media = { MessageMedia(message, server, model, activeMedia, { activeMedia = it },{selected=message}) })
                }
            }
            if (server.canLoadOlder) item("history") {
                TextButton(onClick = { model.command(Action.Older) }, enabled = !server.loadingOlder, modifier = Modifier.fillMaxWidth().padding(12.dp)) {
                    if (server.loadingOlder) CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp) else Text("Load earlier messages")
                }
            }
        }

        // Content can continue behind glass, while system status symbols keep
        // a quiet, legible background even when a message scrolls under them.
        Box(Modifier.align(Alignment.TopCenter).fillMaxWidth()
            .height(WindowInsets.statusBars.asPaddingValues().calculateTopPadding() + 20.dp)
            .background(Brush.verticalGradient(listOf(MaterialTheme.colorScheme.background, MaterialTheme.colorScheme.background, Color.Transparent))))
        Column(Modifier.align(Alignment.TopCenter).statusBarsPadding().padding(horizontal = 12.dp, vertical = 8.dp)
            .onSizeChanged { headerHeight = with(density) { it.height.toDp() } + 32.dp }) {
            GlassPill(Modifier.fillMaxWidth()) {
                Column(Modifier.fillMaxWidth().testTag("channel-header")
                    .animateContentSize()
                    .clickable(role = Role.Button, onClickLabel = if (descriptionExpanded) "Hide channel description" else "Show channel description") {
                        descriptionExpanded = !descriptionExpanded
                    }
                    .semantics { stateDescription = if (descriptionExpanded) "Expanded" else "Collapsed" }) {
                    Row(Modifier.fillMaxWidth().padding(6.dp), verticalAlignment = Alignment.CenterVertically) {
                        IconButton(onClick = onBack) { Icon(Icons.Outlined.ArrowBack, "Back to conversations") }
                        Text(channel.name, modifier = Modifier.weight(1f).padding(horizontal = 4.dp), textAlign = TextAlign.Center,
                            style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        IconButton(onClick = { members = true }) { Icon(Icons.Outlined.PeopleOutline, "Community members") }
                    }
                    if (descriptionExpanded) {
                        Text(channel.topic.ifBlank {
                            if (channel.kind == "direct") "Direct conversation with ${channel.name}." else "No channel description yet."
                        }, modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp).padding(top = 2.dp, bottom = 18.dp)
                            .heightIn(max = 160.dp).verticalScroll(rememberScrollState()),
                            textAlign = TextAlign.Center, style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            if (server.connection != "online") GlassPill(Modifier.padding(top = 8.dp), radius = 24) {
                ConnectionBanner(server.connection) { model.command(Action.Refresh) }
            }
        }

        Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(12.dp)
            .onSizeChanged { composerHeight = with(density) { it.height.toDp() } }) {
            if ((!nearBottom || server.browsingHistory) && server.messages.isNotEmpty()) Box(Modifier.fillMaxWidth().padding(bottom = 8.dp), contentAlignment = Alignment.Center) {
                GlassPill {
                    TextButton(onClick = ::latestMessages) {
                        Icon(Icons.Outlined.ArrowDownward, null, Modifier.size(18.dp)); Spacer(Modifier.width(6.dp)); Text("Latest messages")
                    }
                }
            }
            if(server.loadingContext) GlassPill(Modifier.fillMaxWidth().padding(bottom=8.dp),radius=24) {
                Row(Modifier.padding(start=16.dp,end=6.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(10.dp)) {
                    CircularProgressIndicator(Modifier.size(18.dp),strokeWidth=2.dp)
                    Text("Opening original message…",style=MaterialTheme.typography.bodySmall,modifier=Modifier.weight(1f))
                    TextButton(onClick=::latestMessages) {Text("Cancel")}
                }
            }
            if (server.typing.isNotEmpty()) Text("${server.typing.joinToString()} ${if (server.typing.size == 1) "is" else "are"} typing…",
                style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 18.dp, bottom = 6.dp))
            if(composerFocused && !recording && !emojiPicker && !gifPicker) ComposerSuggestions(composer,server,model) {updateComposer(it)}
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.Bottom) {
                AttachmentActions(draftKey, model, enabled = !recording && !submitting && !attachmentDraft.sending && !attachmentDraft.preparing, onRecord = { focus.clearFocus(); keyboard?.hide(); recording = true }, modifier = Modifier.zIndex(1f))
                GlassPill(Modifier.weight(1f), radius = 28) {
                    if (recording) RecordingComposer(draftKey, model, onClose = { recording = false }, onSend = {
                        if (attachmentDraft.uncertain) confirmAttachmentRetry = true else submitMessage(true)
                    })
                    else Column {
                        PendingAttachments(attachmentDraft) { model.attachments.remove(draftKey, it) }
                        if (reply != null) Row(Modifier.testTag("composer-reply").clickable(role=Role.Button,onClickLabel="Open original message") {reply?.let(::openOriginal)}.padding(start = 18.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                            Icon(Icons.Outlined.Reply, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.primary)
                            Text(server.messages.find { it.id == reply }?.let { "Reply to ${it.author}: ${it.previewText()}" } ?: "Reply to message",
                                maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.labelMedium, modifier = Modifier.weight(1f).padding(start = 8.dp))
                            IconButton(onClick = { reply = null }) { Icon(Icons.Outlined.Close, "Cancel reply", Modifier.size(18.dp)) }
                        }
                        Row(Modifier.padding(6.dp), verticalAlignment = Alignment.CenterVertically) {
                            ComposerInput(composer,server,model,"Message ${if(channel.kind=="direct") channel.name else "#${channel.name}"}",onChange={updateComposer(it); model.typing(server)},modifier=Modifier.weight(1f),onFocusChange={composerFocused=it},animationsAllowed=viewedMedia==null && !emojiPicker && !gifPicker)
                            IconButton(onClick = { focus.clearFocus(); keyboard?.hide(); emojiTarget=null; emojiPicker=true }, enabled=!submitting && !attachmentDraft.sending, modifier=Modifier.size(40.dp)) { Icon(Icons.Outlined.SentimentSatisfiedAlt,"Emoji and stickers",Modifier.size(22.dp)) }
                            IconButton(onClick = { focus.clearFocus(); keyboard?.hide(); gifPicker=true }, enabled=!submitting && !attachmentDraft.sending && !gifPreparing, modifier=Modifier.size(40.dp)) { Icon(Icons.Outlined.GifBox,"GIF picker",Modifier.size(24.dp)) }
                            FilledIconButton(onClick = {
                                if (attachmentDraft.files.isNotEmpty() && attachmentDraft.uncertain) confirmAttachmentRetry = true
                                else submitMessage(false)
                            }, enabled = (text.isNotBlank() || attachmentDraft.files.isNotEmpty()) && !submitting && !attachmentDraft.sending && !attachmentDraft.preparing && !gifPreparing,
                                modifier = Modifier.size(48.dp)) {
                                if (submitting || attachmentDraft.sending) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp) else Icon(Icons.Outlined.ArrowUpward, "Send message")
                            }
                        }
                    }
                }
            }
        }
    }

    if(emojiPicker) EmojiPicker(server,model,onDismiss={emojiPicker=false},onPick={emoji ->
        val target=emojiTarget
        if(target!=null) { model.react(server,server.messages.firstOrNull {it.id==target.id} ?: target,emoji); emojiPicker=false }
        else updateComposer(composer.insert(emoji.token))
    })
    if(gifPicker) GifPicker(server,model,onDismiss={gifPicker=false},onPick={gif ->
        gifPreparing=true
        model.pickGif(server,draftKey,gif,onMarker={ marker ->
            val next=if(text.isBlank()) marker else text.trimEnd()+"\nhttps://giphy.com/gifs/${gif.providerId}"
            updateComposer(composer.edited(androidx.compose.ui.text.input.TextFieldValue(next,androidx.compose.ui.text.TextRange(next.length))))
        },onDone={gifPreparing=false; gifPicker=false}).invokeOnCompletion { scope.launch {gifPreparing=false} }
    })
    reactionPeople?.let { (id,emoji) -> ReactionPeople(server,model,id,emoji,onDismiss={reactionPeople=null}) }

    if (confirmAttachmentRetry) AlertDialog(onDismissRequest = { confirmAttachmentRetry = false },
        title = { Text("Retry sending attachments?") },
        text = { Text("The server may already have received the previous upload. Check the conversation first: sending again could duplicate it.") },
        confirmButton = { TextButton(onClick = { confirmAttachmentRetry = false; submitMessage(false) }) { Text("Send again") } },
        dismissButton = { TextButton(onClick = { confirmAttachmentRetry = false }) { Text("Cancel") } })

    selected?.let { message -> ModalBottomSheet(onDismissRequest = { selected = null }) {
        Column(Modifier.navigationBarsPadding().heightIn(max=640.dp).verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
            Text(message.content.ifBlank { "Message" }, maxLines = 3, overflow = TextOverflow.Ellipsis, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp))
            if (!message.pending) {
                val favorites=expressions.favoriteStickers.mapNotNull {id -> server.emojis.firstOrNull {it.id==id}?.let {EmojiChoice(it.id,true,it.name)}}
                val frequent=favorites + (expressions.usage.map {it.first}.filter { !it.custom || server.emojis.any { e->e.id==it.value } } + server.emojis.map {EmojiChoice(it.id,true,it.name)} + listOf("👍","❤️","😂","🎉","👀").map {EmojiChoice(it)}).distinctBy {it.key}.filter {e -> favorites.none {it.key==e.key}}.take(5)
                Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically) {
                    LazyRow(Modifier.weight(1f),horizontalArrangement=Arrangement.spacedBy(4.dp)) {
                        items(frequent,key={it.key}) { emoji -> IconButton(onClick = { model.react(server,message,emoji); selected=null },modifier=Modifier.semantics {contentDescription="React with ${emoji.token}"}) { EmojiGlyph(server,emoji,28,model) } }
                    }
                    IconButton(onClick={emojiTarget=message; selected=null; emojiPicker=true}) {Icon(Icons.Outlined.AddReaction,"More reactions")}
                }
                SheetAction("Reply", Icons.Outlined.Reply) { reply = message.id; selected = null }
            }
            SheetAction("Copy text", Icons.Outlined.ContentCopy) { clipboard.setText(AnnotatedString(message.content)); selected = null }
            val links=(messageLinks(message.content).map {it.url}+message.embeds.map {it.url}+listOfNotNull(GifChoice.marker(message.content)?.let {"https://giphy.com/gifs/$it"})).filter(::isPublicLink).distinct()
            var copied by remember(message.id) {mutableStateOf("")}
            links.forEach {url ->
                ListItem(headlineContent={Text(url,maxLines=3,overflow=TextOverflow.Ellipsis,style=MaterialTheme.typography.bodySmall)},
                    supportingContent=if(copied==url) {{Text("Copied URL")}} else null,
                    trailingContent={IconButton(onClick={clipboard.setText(AnnotatedString(url));copied=url}) {Icon(Icons.Outlined.ContentCopy,"Copy URL: $url")}})
            }
            if (message.mine && !message.pending) {
                SheetAction("Edit message", Icons.Outlined.Edit) { edit = message; selected = null }
                SheetAction("Delete message", Icons.Outlined.DeleteOutline) { delete = message; selected = null }
            }
            if (message.delivery in listOf("failed", "unknown")) {
                SheetAction("Retry sending", Icons.Outlined.Refresh) { retry = message; selected = null }
                SheetAction("Discard unsent message", Icons.Outlined.DeleteOutline) { model.command(Action.Dismiss(message.id)); selected = null }
            }
            Spacer(Modifier.height(16.dp))
        }
    } }
    edit?.let { message ->
        var edited by remember(message.id) { mutableStateOf(message.content) }
        AlertDialog(onDismissRequest = { edit = null }, title = { Text("Edit message") }, text = {
            OutlinedTextField(edited, { edited = it }, label = { Text("Message") }, modifier = Modifier.fillMaxWidth(), maxLines = 8)
        }, confirmButton = { TextButton(onClick = { model.command(Action.Edit(message.id, edited)); edit = null }, enabled = edited.isNotBlank()) { Text("Save") } }, dismissButton = { TextButton(onClick = { edit = null }) { Text("Cancel") } })
    }
    delete?.let { message -> AlertDialog(onDismissRequest = { delete = null }, title = { Text("Delete this message?") }, text = { Text("This removes the message for everyone. You can’t undo it.") },
        confirmButton = { TextButton(onClick = { model.command(Action.Delete(message.id)); delete = null }) { Text("Delete", color = MaterialTheme.colorScheme.error) } }, dismissButton = { TextButton(onClick = { delete = null }) { Text("Cancel") } }) }
    retry?.let { message -> AlertDialog(onDismissRequest = { retry = null }, title = { Text("Retry sending?") }, text = { Text(if (message.delivery == "unknown") "The server may already have received this message. Retrying could send it twice." else "Try sending this message again.") },
        confirmButton = { TextButton(onClick = { model.command(Action.Retry(message.id)); retry = null }) { Text("Retry") } }, dismissButton = { TextButton(onClick = { retry = null }) { Text("Cancel") } }) }
    if (members) ModalBottomSheet(onDismissRequest = { members = false }) {
        Text("People in ${server.name}", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(horizontal = 24.dp, vertical = 12.dp))
        LazyColumn(Modifier.navigationBarsPadding(), contentPadding = PaddingValues(bottom = 24.dp)) {
            items(server.members, key = { it.id }) { person -> ListItem(headlineContent = { Text(person.name) }, supportingContent = { Text(person.presence) }, leadingContent = { Avatar(person.name) }, modifier = Modifier.clickable(enabled = person.id != server.me) { model.command(Action.OpenDm(person.id)); members = false }) }
        }
    }
    }
    viewedMedia?.let {media -> CompositionLocalProvider(LocalPlaybackBookmarks provides playbackBookmarks) {MediaViewer(server,model,media) {viewedMedia=null}}}
}

@Composable
private fun SheetAction(label: String, icon: androidx.compose.ui.graphics.vector.ImageVector, action: () -> Unit) {
    ListItem(headlineContent = { Text(label) }, leadingContent = { Icon(icon, null) }, modifier = Modifier.clip(RoundedCornerShape(16.dp)).clickable(onClick = action))
}

@Composable
private fun MessageRow(message: ChatMessage, server: ServerState, model: PapoViewModel, highlight:Boolean, onJump:(String)->Unit, onActions: () -> Unit, onReaction: (Reaction) -> Unit, onReactionPeople: (Reaction) -> Unit, media: @Composable () -> Unit) {
    val time = remember(message.at) { SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(message.at)) }
    val spotlight by animateColorAsState(if(highlight) MaterialTheme.colorScheme.primary.copy(alpha=.14f) else Color.Transparent,animationSpec=tween(250),label="Original message highlight")
    Row(Modifier.fillMaxWidth().padding(horizontal=8.dp).testTag("message-${message.id}")
        .background(spotlight,RoundedCornerShape(18.dp)).semantics {if(highlight) stateDescription="Original message"}
        .combinedClickable(onClick = {}, onLongClick = onActions, onLongClickLabel = "Message actions").padding(horizontal = 12.dp, vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Avatar(message.author)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(message.author, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                Text(time, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (message.edited) Text("edited", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                IconButton(onClick = onActions, modifier = Modifier.size(48.dp)) { Icon(Icons.Outlined.MoreHoriz, "Message actions", Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) }
            }
            if (message.replyTo.isNotBlank()) Row(Modifier.testTag("reply-preview-${message.id}").clip(RoundedCornerShape(8.dp)).clickable(role=Role.Button,onClickLabel="Open original message") {onJump(message.replyTo)}.background(MaterialTheme.colorScheme.surfaceContainer).padding(8.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Icon(Icons.Outlined.Reply, null, Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(server.messages.firstOrNull {it.id==message.replyTo}?.previewText() ?: message.replyContent.ifBlank { "Reply to an earlier message" }, maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            val content=remember(message.content,message.embeds) {message.visibleContent()}
            if (content.isNotBlank() && GifChoice.marker(message.content)==null) MessageText(content,server,model)
            media()
            if (message.reactions.isNotEmpty()) FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                message.reactions.forEach { reaction ->
                    val name=if(reaction.custom) server.emojis.firstOrNull {it.id==reaction.emoji}?.name ?: "sticker" else reaction.emoji
                    Surface(shape=RoundedCornerShape(16.dp),color=if(reaction.mine) MaterialTheme.colorScheme.primaryContainer else if(MaterialTheme.colorScheme.surface.luminance()<.5f) Color(0xFF2E2E32) else Color(0xFFECECEF),
                        modifier=Modifier.padding(vertical=4.dp).clip(RoundedCornerShape(16.dp)).combinedClickable(
                            onClick={onReaction(reaction)},onLongClick={onReactionPeople(reaction)},onLongClickLabel="See who reacted",role=Role.Button)
                            .semantics {contentDescription="Reaction $name"; stateDescription=if(reaction.mine) "You reacted, ${reaction.count} reactions" else "${reaction.count} reactions"}) {
                        Row(Modifier.padding(horizontal=10.dp,vertical=6.dp),verticalAlignment=Alignment.CenterVertically) { EmojiGlyph(server,EmojiChoice(reaction.emoji,reaction.custom),22,model); Text(" ${reaction.count}") }
                    }
                }
            }
            if (message.pending) Text(when (message.delivery) {
                "unknown" -> "Delivery unconfirmed · tap ⋯ to retry"
                "failed" -> "Not sent · tap ⋯ to retry"
                "sending" -> "Sending…"
                else -> "Queued · saved on this device"
            }, style = MaterialTheme.typography.labelSmall, color = if (message.delivery in listOf("failed", "unknown")) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun linkedText(content: String): AnnotatedString {
    val color = MaterialTheme.colorScheme.primary
    return remember(content, color) { buildAnnotatedString {
        var end = 0
        Regex("https?://[^\\s<>]+").findAll(content).forEach { match ->
            append(content.substring(end, match.range.first))
            withLink(LinkAnnotation.Url(match.value, TextLinkStyles(SpanStyle(color = color)))) { append(match.value) }
            end = match.range.last + 1
        }
        append(content.substring(end))
    } }
}
