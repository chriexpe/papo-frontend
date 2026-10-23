@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.InboxItem

@Composable
internal fun GlobalAction(
    title: String,
    icon: ImageVector,
    count: Int,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    GlassPill(modifier, radius = 24) {
        Column(
            Modifier.fillMaxWidth()
                .heightIn(min = LocalUiDimensions.current.globalActionHeight)
                .clickable(onClick = onClick)
                .padding(LocalUiDimensions.current.rowPadding),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            BadgedBox(
                badge = {
                    if (count > 0) Badge { Text(if (count > 99) "99+" else count.toString()) }
                }
            ) {
                Icon(icon, null, Modifier.size(24.dp))
            }
            Text(
                title,
                style = MaterialTheme.typography.labelLarge,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/** Shared by server-scoped and global lists; only the scope label changes. */
@Composable
internal fun DirectMessageItem(
    server: ServerState,
    model: PapoViewModel,
    channel: Conversation,
    onOpen: () -> Unit,
    onProfile: () -> Unit,
    showServer: Boolean = false,
) {
    var options by remember(server.url, server.me, channel.id) { mutableStateOf(false) }
    CompositionLocalProvider(LocalProfileContext provides ProfileContext(server, model)) {
        DensityListItem(
            headlineContent = {
                Text(
                    channel.name,
                    color = userNameColor(channel.userId),
                    fontWeight = if (channel.unread) FontWeight.SemiBold else FontWeight.Medium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            },
            supportingContent =
                if (showServer)
                    ({ Text(server.name, maxLines = 1, overflow = TextOverflow.Ellipsis) })
                else null,
            leadingContent = {
                Box(
                    Modifier.size(48.dp).clickable(onClick = onProfile),
                    contentAlignment = Alignment.Center,
                ) {
                    UserAvatar(
                        channel.userId,
                        channel.name,
                        Modifier.size(LocalUiDimensions.current.listIcon),
                    )
                }
            },
            trailingContent = {
                if (channel.mentions > 0) Badge { Text(channel.mentions.toString()) }
                else if (channel.unread)
                    Box(
                        Modifier.size(8.dp)
                            .background(MaterialTheme.colorScheme.primary, CircleShape)
                    )
                else Icon(Icons.Outlined.ChevronRight, null)
            },
            modifier =
                Modifier.testTag("dm-${server.key}-${channel.id}")
                    .clip(RoundedCornerShape(18.dp))
                    .combinedClickable(
                        onClick = onOpen,
                        onLongClick = { options = true },
                        onLongClickLabel = ui(R.string.conversation_options),
                    ),
        )
    }
    if (options)
        key(server.url, server.me, channel.id) {
            DirectMessageOptions(server, model, channel) { options = false }
        }
}

@Composable
private fun GlobalHeader(title: String, onBack: () -> Unit, action: @Composable () -> Unit = {}) {
    Row(
        Modifier.fillMaxWidth().statusBarsPadding().padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        GlassPill {
            IconButton(onClick = onBack) {
                Icon(Icons.Outlined.ArrowBack, ui(R.string.back_to_servers))
            }
        }
        Text(
            title,
            Modifier.weight(1f),
            style = MaterialTheme.typography.headlineSmall,
            fontWeight = FontWeight.SemiBold,
        )
        action()
    }
}

internal data class ScopedDirect(val server: ServerState, val channel: Conversation)

internal fun globalDirects(state: PapoState, query: String) =
    state.servers
        .filter { it.screen == "chat" && it.me.isNotBlank() }
        .flatMap { server ->
            server.dms
                .filter { dm ->
                    listOf(
                            dm.name,
                            server.name,
                            server.members.firstOrNull { it.id == dm.userId }?.username.orEmpty(),
                        )
                        .any { it.contains(query.trim(), true) }
                }
                .map { ScopedDirect(server, it) }
        }
        .sortedWith(
            compareByDescending<ScopedDirect> { it.channel.unread }
                .thenBy(String.CASE_INSENSITIVE_ORDER) { it.channel.name }
                .thenBy { it.server.url }
        )

@Composable
internal fun GlobalDirectMessages(
    state: PapoState,
    model: PapoViewModel,
    onBack: () -> Unit,
    onProfile: (ServerState, String) -> Unit,
    onOpen: (ServerState, Conversation) -> Unit,
) {
    var query by rememberSaveable { mutableStateOf("") }
    BackHandler(onBack = onBack)
    Column(Modifier.fillMaxSize().testTag("global-direct-messages")) {
        GlobalHeader(ui(R.string.direct_messages), onBack)
        OutlinedTextField(
            query,
            { query = it },
            Modifier.fillMaxWidth()
                .padding(horizontal = 20.dp, vertical = 8.dp)
                .testTag("direct-search"),
            placeholder = { Text(ui(R.string.search_names_or_servers)) },
            singleLine = true,
            shape = RoundedCornerShape(24.dp),
            leadingIcon = { Icon(Icons.Outlined.Search, null) },
            trailingIcon = {
                if (query.isNotBlank())
                    IconButton(onClick = { query = "" }) {
                        Icon(Icons.Outlined.Close, ui(R.string.clear_direct_search))
                    }
            },
        )
        val rows = globalDirects(state, query)
        LazyColumn(
            Modifier.weight(1f).hazeSource(LocalHaze.current).testTag("global-direct-list"),
            contentPadding = PaddingValues(12.dp, 8.dp, 12.dp, 140.dp),
        ) {
            if (rows.isEmpty())
                item {
                    EmptyState(
                        if (query.isBlank()) ui(R.string.no_direct_messages_yet)
                        else ui(R.string.no_matching_conversations),
                        if (query.isBlank()) ui(R.string.direct_messages_empty_description)
                        else ui(R.string.search_by_nickname_username_or_server_name),
                    )
                }
            items(rows, key = { "${it.server.url}/${it.server.me}/${it.channel.id}" }) { row ->
                DirectMessageItem(
                    row.server,
                    model,
                    row.channel,
                    { onOpen(row.server, row.channel) },
                    { onProfile(row.server, row.channel.userId) },
                    showServer = true,
                )
            }
        }
    }
}

@Composable
internal fun GlobalNotifications(
    state: PapoState,
    model: PapoViewModel,
    onBack: () -> Unit,
    onOpen: (ServerState, InboxItem) -> Unit,
    inbox: NotificationInbox = model.notificationInbox,
) {
    val servers = state.servers.filter { it.screen == "chat" && it.me.isNotBlank() }
    val owners = servers.map { listOf(it.url, it.me, it.notificationCount, it.inboxRevision) }
    val snackbar = remember { SnackbarHostState() }
    DisposableEffect(inbox) {
        inbox.show(servers)
        onDispose { inbox.hide() }
    }
    LaunchedEffect(inbox) { inbox.messages.collect { snackbar.showSnackbar(it) } }
    BackHandler(onBack = onBack)
    LaunchedEffect(owners) { inbox.synchronize(servers) }
    val active = servers.mapNotNull { server ->
        inbox.pages[server.inboxOwner()]?.let { server to it }
    }
    val rows =
        active
            .flatMap { (server, page) -> page.items.map { ScopedNotice(server, it) } }
            .sortedWith(
                compareByDescending<ScopedNotice> { it.item.orderTime }
                    .thenBy { it.server.url }
                    .thenByDescending { it.item.id }
            )
    val unread = rows.count { !it.item.read }
    Box(Modifier.fillMaxSize().testTag("global-notifications")) {
        Column(Modifier.fillMaxSize()) {
            GlobalHeader(ui(R.string.notifications), onBack) {
                IconButton(
                    onClick = inbox::markLoaded,
                    enabled = unread > 0 && !inbox.marking && inbox.reading.isEmpty(),
                    modifier =
                        Modifier.testTag("mark-loaded-notifications").semantics {
                            contentDescription = uiQuantity(R.plurals.inbox_mark_loaded, unread)
                        },
                ) {
                    if (inbox.marking)
                        CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    else
                        Icon(
                            Icons.Outlined.DoneAll,
                            null,
                        )
                }
            }
            if (active.any { it.second.loading })
                LinearProgressIndicator(Modifier.fillMaxWidth().padding(horizontal = 20.dp))
            LazyColumn(
                Modifier.weight(1f).hazeSource(LocalHaze.current).testTag("notification-list"),
                contentPadding = PaddingValues(12.dp, 8.dp, 12.dp, 140.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                if (
                    rows.isEmpty() &&
                        active.none { it.second.loading || it.second.error.isNotBlank() }
                )
                    item {
                        EmptyState(
                            ui(R.string.all_quiet_for_now),
                            ui(R.string.notifications_from_your_signed_in_servers_will_appear_here),
                        )
                    }
                items(rows, key = { "${it.server.url}/${it.server.me}/${it.item.id}" }) { notice ->
                    NotificationInboxRow(
                        notice,
                        model,
                        inbox.marking ||
                            (notice.server.inboxOwner() to notice.item.id) in inbox.reading,
                        onRead = { inbox.readOne(notice.server.inboxOwner(), notice.item.id) },
                        onOpen = onOpen,
                    )
                }
                items(
                    active.filter { it.second.hasMore || it.second.error.isNotBlank() },
                    key = { "page/${it.first.url}/${it.first.me}" },
                ) { (server, page) ->
                    val owner = server.inboxOwner()
                    Column(
                        Modifier.fillMaxWidth().padding(12.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Text(server.name, style = MaterialTheme.typography.labelLarge)
                        if (page.error.isNotBlank()) {
                            Text(page.error, color = MaterialTheme.colorScheme.error)
                            TextButton(enabled = !page.loading, onClick = { inbox.retry(owner) }) {
                                Text(ui(R.string.retry))
                            }
                        } else {
                            LaunchedEffect(owner, page.data?.cursor, page.loading) {
                                if (!page.loading) inbox.more(owner)
                            }
                            TextButton(
                                enabled = !page.loading,
                                onClick = { inbox.more(owner) },
                                modifier = Modifier.testTag("inbox-more-${server.key}"),
                            ) {
                                if (page.loading)
                                    CircularProgressIndicator(
                                        Modifier.size(18.dp),
                                        strokeWidth = 2.dp,
                                    )
                                else Text(ui(R.string.inbox_load_older))
                            }
                        }
                    }
                }
            }
        }
        SnackbarHost(
            snackbar,
            Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(12.dp),
        )
    }
}
