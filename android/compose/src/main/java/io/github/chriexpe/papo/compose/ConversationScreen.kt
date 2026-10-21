@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class, androidx.compose.foundation.ExperimentalFoundationApi::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
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
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
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
    var text by rememberSaveable(draftKey) { mutableStateOf(model.draft(draftKey)) }
    var reply by rememberSaveable(draftKey) { mutableStateOf<String?>(null) }
    var selected by remember { mutableStateOf<ChatMessage?>(null) }
    var edit by remember { mutableStateOf<ChatMessage?>(null) }
    var delete by remember { mutableStateOf<ChatMessage?>(null) }
    var retry by remember { mutableStateOf<ChatMessage?>(null) }
    var members by remember { mutableStateOf(false) }
    var descriptionExpanded by rememberSaveable(draftKey) { mutableStateOf(false) }
    var activeMedia by remember(draftKey) { mutableStateOf<String?>(null) }
    var submitting by remember(draftKey) { mutableStateOf(false) }
    var confirmAttachmentRetry by remember(draftKey) { mutableStateOf(false) }
    val attachmentDrafts by model.attachments.drafts.collectAsState()
    val attachmentDraft = attachmentDrafts[draftKey] ?: AttachmentDraft()
    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val density = LocalDensity.current
    var headerHeight by remember { mutableStateOf(90.dp) }
    var composerHeight by remember { mutableStateOf(90.dp) }
    val nearBottom by remember { derivedStateOf { listState.firstVisibleItemIndex == 0 && listState.firstVisibleItemScrollOffset < 100 } }
    val clipboard = LocalClipboardManager.current
    val latestServer by rememberUpdatedState(server)

    LaunchedEffect(draftKey, server.messages.lastOrNull()?.id) {
        if (nearBottom && !server.loadingOlder) listState.animateScrollToItem(0)
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

    val submitMessage: () -> Unit = {
        val sent = text
        submitting = true
        val onResult: (Boolean) -> Unit = { accepted ->
            submitting = false
            if (accepted) {
                if (text == sent) { text = ""; model.saveDraft(draftKey, "") }
                reply = null
                scope.launch { listState.animateScrollToItem(0) }
            }
        }
        if (attachmentDraft.files.isEmpty()) model.send(server, sent, reply, onResult)
        else model.attachments.send(draftKey, server, sent, reply, onResult)
    }

    Box(Modifier.fillMaxSize().imePadding().navigationBarsPadding()) {
        LazyColumn(state = listState, reverseLayout = true,
            modifier = Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("message-list"),
            contentPadding = PaddingValues(top = headerHeight + 8.dp, bottom = composerHeight + 8.dp),
        ) {
            if (server.messages.isEmpty()) item("empty") {
                EmptyState(if (server.connection == "online") "The start of something good." else "No saved messages yet.",
                    if (server.connection == "online") "Say hello in ${channel.name}." else "Messages will appear when the server is reachable.")
            }
            items(server.messages.asReversed(), key = { it.id }) { message ->
                MessageRow(message, onActions = { selected = message }, onReaction = { r ->
                    if (!r.custom) model.command(Action.React(channel.id, message.id, r.emoji, !r.mine))
                }, media = { MessageMedia(message, server, model, activeMedia) { activeMedia = it } })
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
            if (!nearBottom && server.messages.isNotEmpty()) Box(Modifier.fillMaxWidth().padding(bottom = 8.dp), contentAlignment = Alignment.Center) {
                GlassPill {
                    TextButton(onClick = { scope.launch { listState.animateScrollToItem(0) } }) {
                        Icon(Icons.Outlined.ArrowDownward, null, Modifier.size(18.dp)); Spacer(Modifier.width(6.dp)); Text("Latest messages")
                    }
                }
            }
            if (server.typing.isNotEmpty()) Text("${server.typing.joinToString()} ${if (server.typing.size == 1) "is" else "are"} typing…",
                style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 18.dp, bottom = 6.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.Bottom) {
                AttachmentActions(draftKey, model, enabled = !submitting && !attachmentDraft.sending && !attachmentDraft.preparing, modifier = Modifier.zIndex(1f))
                GlassPill(Modifier.weight(1f), radius = 28) {
                    Column {
                        PendingAttachments(attachmentDraft) { model.attachments.remove(draftKey, it) }
                        if (reply != null) Row(Modifier.padding(start = 18.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                            Icon(Icons.Outlined.Reply, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.primary)
                            Text(server.messages.find { it.id == reply }?.let { "Reply to ${it.author}: ${it.content}" } ?: "Reply to message",
                                maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.labelMedium, modifier = Modifier.weight(1f).padding(start = 8.dp))
                            IconButton(onClick = { reply = null }) { Icon(Icons.Outlined.Close, "Cancel reply", Modifier.size(18.dp)) }
                        }
                        Row(Modifier.padding(6.dp), verticalAlignment = Alignment.Bottom) {
                            BasicTextField(text, onValueChange = {
                                text = it
                                model.saveDraft(draftKey, it)
                                model.typing(server)
                            }, modifier = Modifier.weight(1f).padding(start = 14.dp, end = 8.dp, top = 12.dp, bottom = 12.dp).heightIn(min = 24.dp, max = 160.dp),
                                textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary), maxLines = 6,
                                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                                decorationBox = { inner -> Box { if (text.isEmpty()) Text("Message ${if (channel.kind == "direct") channel.name else "#${channel.name}"}", color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyLarge, maxLines = 1, softWrap = false, overflow = TextOverflow.Ellipsis); inner() } })
                            FilledIconButton(onClick = {
                                if (attachmentDraft.files.isNotEmpty() && attachmentDraft.uncertain) confirmAttachmentRetry = true
                                else submitMessage()
                            }, enabled = (text.isNotBlank() || attachmentDraft.files.isNotEmpty()) && !submitting && !attachmentDraft.sending && !attachmentDraft.preparing,
                                modifier = Modifier.size(48.dp)) {
                                if (submitting || attachmentDraft.sending) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp) else Icon(Icons.Outlined.ArrowUpward, "Send message")
                            }
                        }
                    }
                }
            }
        }
    }

    if (confirmAttachmentRetry) AlertDialog(onDismissRequest = { confirmAttachmentRetry = false },
        title = { Text("Retry sending attachments?") },
        text = { Text("The server may already have received the previous upload. Check the conversation first: sending again could duplicate it.") },
        confirmButton = { TextButton(onClick = { confirmAttachmentRetry = false; submitMessage() }) { Text("Send again") } },
        dismissButton = { TextButton(onClick = { confirmAttachmentRetry = false }) { Text("Cancel") } })

    selected?.let { message -> ModalBottomSheet(onDismissRequest = { selected = null }) {
        Column(Modifier.navigationBarsPadding().padding(horizontal = 16.dp)) {
            Text(message.content.ifBlank { "Message" }, maxLines = 3, overflow = TextOverflow.Ellipsis, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp))
            if (!message.pending) {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly) { listOf("👍", "❤️", "😂", "🎉", "👀").forEach { emoji -> TextButton(onClick = {
                    model.command(Action.React(channel.id, message.id, emoji, !message.reactions.any { it.emoji == emoji && it.mine })); selected = null
                }) { Text(emoji, fontSize = 24.sp) } } }
                SheetAction("Reply", Icons.Outlined.Reply) { reply = message.id; selected = null }
            }
            SheetAction("Copy text", Icons.Outlined.ContentCopy) { clipboard.setText(AnnotatedString(message.content)); selected = null }
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

@Composable
private fun SheetAction(label: String, icon: androidx.compose.ui.graphics.vector.ImageVector, action: () -> Unit) {
    ListItem(headlineContent = { Text(label) }, leadingContent = { Icon(icon, null) }, modifier = Modifier.clip(RoundedCornerShape(16.dp)).clickable(onClick = action))
}

@Composable
private fun MessageRow(message: ChatMessage, onActions: () -> Unit, onReaction: (Reaction) -> Unit, media: @Composable () -> Unit) {
    val time = remember(message.at) { SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(message.at)) }
    Row(Modifier.fillMaxWidth().combinedClickable(onClick = {}, onLongClick = onActions, onLongClickLabel = "Message actions").padding(horizontal = 20.dp, vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Avatar(message.author)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(message.author, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                Text(time, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (message.edited) Text("edited", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                IconButton(onClick = onActions, modifier = Modifier.size(48.dp)) { Icon(Icons.Outlined.MoreHoriz, "Message actions", Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) }
            }
            if (message.replyTo.isNotBlank()) Row(Modifier.clip(RoundedCornerShape(8.dp)).background(MaterialTheme.colorScheme.surfaceContainer).padding(8.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Icon(Icons.Outlined.Reply, null, Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(message.replyContent.ifBlank { "Reply to an earlier message" }, maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (message.content.isNotBlank()) SelectionContainer { Text(linkedText(message.content), style = MaterialTheme.typography.bodyLarge, lineHeight = 24.sp) }
            media()
            if (message.reactions.isNotEmpty()) FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                message.reactions.forEach { reaction -> FilterChip(reaction.mine, { onReaction(reaction) }, enabled = !reaction.custom, label = { Text("${reaction.emoji} ${reaction.count}") }) }
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
