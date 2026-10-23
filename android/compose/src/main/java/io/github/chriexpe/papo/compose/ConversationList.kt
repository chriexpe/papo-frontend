@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)

package io.github.chriexpe.papo.compose

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.layout.LazyLayoutCacheWindow
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.SearchAnalysis
import io.github.chriexpe.papo.core.SearchSuggestion
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay

/** The editor uses ranges returned by the core, including quoted token suffixes. */
internal fun applySearchSuggestion(value: TextFieldValue, hint: SearchSuggestion): TextFieldValue {
    val start = hint.start.toInt().coerceIn(0, value.text.length)
    val end = hint.end.toInt().coerceIn(start, value.text.length)
    val prefix = value.text.take(start) + hint.replacement
    val complete = !hint.replacement.endsWith(':')
    val text =
        prefix +
            (if (complete) " " else "") +
            value.text.drop(end).let { if (complete) it.trimStart() else it }
    return TextFieldValue(text, TextRange(prefix.length + if (complete) 1 else 0))
}

@Composable
internal fun ConversationList(
    server: ServerState,
    model: PapoViewModel,
    appError: String,
    direct: Boolean,
    onSettings: () -> Unit,
    onOpen: (String) -> Unit,
    onProfile: (String) -> Unit,
    onMessage: (String, String) -> Unit,
) {
    val dimensions = LocalUiDimensions.current
    // A server owns its search; changing servers never leaks terms/results.
    var query by
        rememberSaveable(server.url, stateSaver = TextFieldValue.Saver) {
            mutableStateOf(TextFieldValue())
        }
    var analysis by remember(server.url) { mutableStateOf<SearchAnalysis?>(null) }
    var focused by remember { mutableStateOf(false) }
    var analyzing by remember { mutableStateOf(false) }
    val searching = query.text.isNotBlank()
    var selectedResult by
        remember(server.url, server.me) { mutableStateOf<Pair<String, ChatMessage>?>(null) }
    val resultsList =
        rememberLazyListState(
            cacheWindow =
                remember { LazyLayoutCacheWindow(aheadFraction = 2f, behindFraction = 2f) }
        )
    val keyboard = LocalSoftwareKeyboardController.current
    val focus = LocalFocusManager.current
    fun update(value: TextFieldValue) {
        if (value.text != query.text) analysis = null
        query = value
    }
    fun pick(hint: SearchSuggestion) {
        update(applySearchSuggestion(query, hint))
    }
    BackHandler(searching) {
        update(TextFieldValue())
        focus.clearFocus()
        keyboard?.hide()
    }
    LaunchedEffect(query.text, query.selection, server.url, server.members, server.channels) {
        analyzing = true
        try {
            analysis = model.analyzeSearch(server, query.text, query.selection.end)
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            analysis = null
        } finally {
            analyzing = false
        }
    }
    LaunchedEffect(query.text, analysis?.valid, server.url, server.connection) {
        val text = query.text
        if (!searching || analysis?.valid != true) {
            model.command(Action.ClearSearch, server.url)
            return@LaunchedEffect
        }
        delay(300)
        if (server.connection == "online" && server.searchQuery != text)
            model.command(Action.Search(text, false), server.url)
    }
    val hints = if (focused) analysis?.suggestions.orEmpty() else emptyList()
    Column(Modifier.fillMaxSize().statusBarsPadding().imePadding()) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = dimensions.screenPadding, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(dimensions.contentGap),
        ) {
            ServerHeader(server, model, appError, Modifier.weight(1f))
            GlassPill(Modifier.size(dimensions.headerHeight), radius = 20) {
                Box(
                    Modifier.fillMaxSize()
                        .clickable(
                            onClickLabel = ui(R.string.client_settings),
                            onClick = onSettings,
                        )
                        .testTag("account-settings")
                        .semantics { contentDescription = ui(R.string.client_settings) }
                        .padding(4.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    UserAvatar(
                        server.me,
                        server.myName,
                        Modifier.fillMaxSize(),
                        shape = RoundedCornerShape(16.dp),
                    )
                }
            }
        }
        if (server.connection != "online")
            ConnectionBanner(server.connection) { model.command(Action.Refresh, server.url) }
        OutlinedTextField(
            query,
            ::update,
            placeholder = { Text(ui(R.string.search_channels_people_messages)) },
            leadingIcon = { Icon(Icons.Outlined.Search, null) },
            trailingIcon =
                if (query.text.isNotEmpty())
                    ({
                        IconButton(onClick = { update(TextFieldValue()) }) {
                            Icon(Icons.Outlined.Close, ui(R.string.clear_search))
                        }
                    })
                else null,
            singleLine = true,
            shape = RoundedCornerShape(24.dp),
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions =
                KeyboardActions(
                    onSearch = {
                        if (hints.isNotEmpty()) pick(hints.first())
                        else {
                            focus.clearFocus()
                            keyboard?.hide()
                        }
                    }
                ),
            modifier =
                Modifier.fillMaxWidth()
                    .padding(horizontal = dimensions.screenPadding, vertical = 8.dp)
                    .testTag("global-search")
                    .onFocusChanged { focused = it.isFocused },
        )
        if (hints.isNotEmpty())
            Surface(
                Modifier.padding(horizontal = dimensions.screenPadding).fillMaxWidth(),
                shape = RoundedCornerShape(18.dp),
                color = MaterialTheme.colorScheme.surfaceContainer,
            ) {
                LazyColumn(Modifier.heightIn(max = 216.dp).testTag("search-suggestions")) {
                    items(hints, key = { it.replacement }) { hint ->
                        ListItem(
                            headlineContent = {
                                Text(
                                    searchSuggestionLabel(hint),
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            },
                            leadingContent = { Icon(Icons.Outlined.FilterAlt, null) },
                            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                            modifier = Modifier.clickable { pick(hint) },
                        )
                    }
                }
            }
        val needle =
            if (searching && ':' !in query.text)
                query.text.trim().removePrefix("@").removePrefix("#")
            else ""
        val channels =
            server.channels.filter {
                it.kind == "text" &&
                    (!searching || (needle.isNotBlank() && it.name.contains(needle, true)))
            }
        val people =
            server.members.filter {
                it.id != server.me &&
                    (!searching ||
                        ((needle.isNotBlank() || query.text.trim() == "@") &&
                            (it.name.contains(needle, true) || it.username.contains(needle, true))))
            }
        val exact = server.searchQuery == query.text
        val results = if (exact) server.searchResults else emptyList()
        Box(Modifier.weight(1f)) {
            MessageResultsMediaHost(
                server,
                model,
                resultsList,
                results.map { "message/${it.id}" },
                sessionKey = query.text,
            ) { active, onPlayer, visible ->
                LazyColumn(
                    state = resultsList,
                    modifier =
                        Modifier.fillMaxSize()
                            .hazeSource(LocalHaze.current)
                            .testTag("conversation-results"),
                    contentPadding = PaddingValues(start = 12.dp, end = 12.dp, bottom = 128.dp),
                ) {
                    if (searching) {
                        if (channels.isNotEmpty()) item { SearchHeading("Channels") }
                        items(channels, key = { "channel/${it.id}" }) { ChannelItem(it, onOpen) }
                        if (people.isNotEmpty()) item { SearchHeading("People") }
                        items(people, key = { "person/${it.id}" }) { PersonItem(it, onProfile) }
                        item { SearchHeading("Messages") }
                        if (server.connection != "online")
                            item {
                                SearchNotice(
                                    ui(R.string.connect_to_search_messages_across_this_server)
                                )
                            }
                        else if (!analyzing && analysis?.valid == false)
                            item {
                                SearchNotice(
                                    ui(R.string.complete_a_filter_above_or_enter_a_search_term)
                                )
                            }
                        else if (analyzing || !exact || server.searching)
                            item {
                                Box(
                                    Modifier.fillMaxWidth().padding(24.dp),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    CircularProgressIndicator(Modifier.size(24.dp))
                                }
                            }
                        if (exact && !server.searching && server.error.isNotBlank())
                            item { SearchNotice(server.error) }
                        if (
                            exact &&
                                !server.searching &&
                                server.error.isBlank() &&
                                results.isEmpty() &&
                                analysis?.valid == true &&
                                server.connection == "online"
                        )
                            item { SearchNotice(ui(R.string.no_matching_messages)) }
                        items(results, key = { "message/${it.id}" }) { hit ->
                            val row = "message/${hit.id}"
                            CompositionLocalProvider(
                                LocalMediaRowId provides row,
                                LocalTimelineMediaVisible provides (row in visible),
                            ) {
                                SearchMessageResult(
                                    hit,
                                    server,
                                    model,
                                    row in visible,
                                    active,
                                    onPlayer,
                                    {
                                        focus.clearFocus()
                                        keyboard?.hide()
                                        onMessage(hit.channelId, hit.id)
                                    },
                                    { selectedResult = hit.channelId to it },
                                )
                            }
                        }
                        if (exact && server.searchHasMore)
                            item {
                                TextButton(
                                    onClick = {
                                        model.command(Action.Search(query.text, true), server.url)
                                    },
                                    enabled = !server.searching,
                                    modifier = Modifier.fillMaxWidth(),
                                ) {
                                    Text(ui(R.string.load_more_messages))
                                }
                            }
                    } else {
                        if (direct) item { SearchHeading(ui(R.string.direct_messages)) }
                        val conversations =
                            if (direct) server.dms
                            else
                                server.channels.filter { it.kind != "direct" && it.kind != "voice" }
                        if (conversations.isEmpty())
                            item {
                                EmptyState(
                                    ui(R.string.a_little_quiet_here),
                                    if (direct)
                                        ui(R.string.search_for_someone_to_start_a_direct_message)
                                    else ui(R.string.your_text_channels_will_appear_here),
                                )
                            }
                        items(conversations, key = { it.id }) { channel ->
                            if (channel.kind == "category") SearchHeading(channel.name.uppercase())
                            else if (direct)
                                DirectMessageItem(
                                    server,
                                    model,
                                    channel,
                                    onOpen = { onOpen(channel.id) },
                                    onProfile = { onProfile(channel.userId) },
                                )
                            else ChannelItem(channel, onOpen)
                        }
                    }
                }
                selectedResult?.let { (channel, stale) ->
                    val current =
                        server.searchResults.firstOrNull { it.id == stale.id }?.message ?: stale
                    ResultMessageActions(current, channel, server, model, { selectedResult = null })
                }
            }
        }
    }
}

@Composable
private fun SearchHeading(text: String) {
    Text(
        text,
        Modifier.padding(start = 20.dp, top = 20.dp, bottom = 8.dp),
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun SearchNotice(text: String) {
    Text(
        text,
        Modifier.padding(20.dp),
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun PersonItem(
    person: io.github.chriexpe.papo.core.MemberView,
    onProfile: (String) -> Unit,
) {
    DensityListItem(
        headlineContent = {
            Text(
                person.name,
                color = userNameColor(person.id),
                modifier = Modifier.testTag("member-name-${person.id}"),
                fontWeight = FontWeight.Medium,
            )
        },
        supportingContent = { Text("@${person.username}") },
        leadingContent = { UserAvatar(person.id, person.name) },
        trailingContent = { Icon(Icons.Outlined.PersonOutline, null) },
        modifier = Modifier.clip(RoundedCornerShape(18.dp)).clickable { onProfile(person.id) },
    )
}

@Composable
private fun ChannelItem(
    channel: io.github.chriexpe.papo.core.ChannelView,
    onOpen: (String) -> Unit,
) {
    DensityListItem(
        headlineContent = {
            Text(
                channel.name,
                fontWeight = if (channel.unread) FontWeight.Bold else FontWeight.Medium,
            )
        },
        supportingContent =
            if (channel.topic.isNotBlank())
                ({ Text(channel.topic, maxLines = 1, overflow = TextOverflow.Ellipsis) })
            else null,
        leadingContent = {
            Box(
                Modifier.size(LocalUiDimensions.current.listIcon)
                    .clip(RoundedCornerShape(14.dp))
                    .background(MaterialTheme.colorScheme.surfaceContainer),
                contentAlignment = Alignment.Center,
            ) {
                Text("#", fontSize = 23.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        },
        trailingContent = {
            if (channel.mentions > 0) Badge { Text(channel.mentions.toString()) }
            else if (channel.unread)
                Box(Modifier.size(8.dp).background(MaterialTheme.colorScheme.primary, CircleShape))
            else
                Icon(
                    Icons.Outlined.ChevronRight,
                    null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
        },
        modifier = Modifier.clip(RoundedCornerShape(18.dp)).clickable { onOpen(channel.id) },
    )
}

/**
 * Filter replacements are protocol syntax. Only their explanatory labels use the app locale;
 * server-provided names and usernames retain their spelling.
 */
internal fun searchSuggestionLabel(hint: io.github.chriexpe.papo.core.SearchSuggestion): String =
    when (hint.replacement) {
        "de:" -> ui(R.string.search_filter_author, hint.replacement)
        "em:" -> ui(R.string.search_filter_channel, hint.replacement)
        "tem:" -> ui(R.string.search_filter_content, hint.replacement)
        "mentions:" -> ui(R.string.search_filter_mention, hint.replacement)
        else -> hint.label
    }
