package io.github.chriexpe.papo.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember

/**
 * The legacy text/media renderers take a ClientSnapshot, but only consume account and display
 * resources. Retain that input independently of read receipts, badges, calls, navigation and
 * background work. Actions use the live snapshot in ConversationScreen, never this drawing input.
 * Replies are separate row inputs so another message arriving does not invalidate every row.
 */
@Composable
internal fun rememberMessagePresentation(server: ServerState): ServerState =
    remember(
        server.url,
        server.key,
        server.me,
        server.selectedChannel,
        server.connection,
        server.capabilities,
        server.members,
        server.emojis,
        server.profiles,
        server.roles,
        server.blockedUsers,
    ) {
        server
    }

/** Native full snapshots also need stable identities for unchanged visible content. */
internal fun ServerState.retainPresentation(previous: ServerState?): ServerState {
    if (previous == null || previous.url != url || previous.me != me) return this
    fun <T> List<T>.retaining(old: List<T>): List<T> = if (this == old) old else this
    val retainedMessages =
        if (messages == previous.messages) previous.messages
        else {
            val old = previous.messages.associateBy { it.id }
            messages.map { next -> old[next.id]?.takeIf { it == next } ?: next }
        }
    return copy(
        messages = retainedMessages,
        members = members.retaining(previous.members),
        emojis = emojis.retaining(previous.emojis),
        profiles = profiles.retaining(previous.profiles),
        roles = roles.retaining(previous.roles),
        blockedUsers = blockedUsers.retaining(previous.blockedUsers),
    )
}

/** Timeline-derived lookups are rebuilt only when messages change, never on pixel scrolls. */
internal class MessageTimelineIndex(
    val messages: List<ChatMessage>,
    zone: java.time.ZoneId = java.time.ZoneId.systemDefault(),
) {
    val byId = messages.associateBy { it.id }
    val orderedIds = messages.map { it.id }
    val ids = orderedIds.toSet()
    val reversedMessages = messages.asReversed()
    val reversedIds = orderedIds.asReversed()
    val timestampIds =
        messages
            .filterIndexed { index, message ->
                index == 0 ||
                    Math.floorDiv(messages[index - 1].at, 60_000) !=
                        Math.floorDiv(message.at, 60_000)
            }
            .map { it.id }
            .toSet()
    val dateStarts =
        messages
            .filterIndexed { index, message ->
                index == 0 ||
                    java.time.Instant.ofEpochMilli(messages[index - 1].at)
                        .atZone(zone)
                        .toLocalDate() !=
                        java.time.Instant.ofEpochMilli(message.at).atZone(zone).toLocalDate()
            }
            .associate { it.id to java.time.Instant.ofEpochMilli(it.at).atZone(zone).toLocalDate() }
    // Reverse layout puts each date row AFTER its first message in list order, above it onscreen.
    val rows = buildList {
        reversedMessages.forEach { message ->
            add(TimelineRow(message.id, message = message))
            dateStarts[message.id]?.let { add(TimelineRow("date/${message.id}", date = it)) }
        }
    }
    private val positions =
        rows.mapIndexedNotNull { index, row -> row.message?.let { it.id to index } }.toMap()

    fun position(id: String): Int? = positions[id]
}

internal data class TimelineRow(
    val key: String,
    val message: ChatMessage? = null,
    val date: java.time.LocalDate? = null,
)
