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
