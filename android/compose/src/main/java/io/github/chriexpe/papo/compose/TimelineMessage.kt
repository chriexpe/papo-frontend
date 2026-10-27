package io.github.chriexpe.papo.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Stable
import androidx.compose.runtime.State
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.geometry.Rect
import io.github.chriexpe.papo.core.ExternalAuthorView

internal sealed interface TimelineInteraction {
    data class Reply(val id: String) : TimelineInteraction

    data class Mention(val person: Person) : TimelineInteraction

    data class Jump(val id: String) : TimelineInteraction

    data class Select(val id: String, val embed: String? = null) : TimelineInteraction

    data class React(val id: String, val reaction: Reaction) : TimelineInteraction

    data class People(val id: String, val reaction: Reaction) : TimelineInteraction

    data class Player(val id: String?) : TimelineInteraction
}

/** Stable callbacks, with live routing/drafts looked up only when an action is performed. */
@Stable
internal class TimelineInteractions(private val current: State<(TimelineInteraction) -> Unit>) {
    fun send(action: TimelineInteraction) = current.value(action)
}

/** Inherited profile locals must not bypass the stable message parameters. */
@Composable
internal fun MessagePresentationScope(
    presentation: ServerState,
    model: PapoViewModel,
    content: @Composable () -> Unit,
) {
    val profile = remember(presentation, model) { ProfileContext(presentation, model) }
    val openProfile = rememberUpdatedState(LocalOpenProfile.current)
    val openExternal = rememberUpdatedState(LocalOpenExternalProfile.current)
    val profileAction = remember { { id: String -> openProfile.value(id) } }
    val externalAction = remember { { author: ExternalAuthorView -> openExternal.value(author) } }
    CompositionLocalProvider(
        LocalProfileContext provides profile,
        LocalOpenProfile provides profileAction,
        LocalOpenExternalProfile provides externalAction,
        content = content,
    )
}

/** Reading/navigation changes never enter the message-content composition boundary. */
@Composable
internal fun TimelineMessage(
    message: ChatMessage,
    presentation: ServerState,
    replyMessage: ChatMessage?,
    model: PapoViewModel,
    grouped: Boolean,
    canReply: Boolean,
    highlighted: Boolean,
    viewport: Rect,
    mediaVisible: Boolean,
    activePlayer: String?,
    interactions: TimelineInteractions,
    showCompactTime: Boolean = true,
) {
    MessagePresentationScope(presentation, model) {
        CompositionLocalProvider(
            LocalMediaViewport provides viewport,
            LocalTimelineMediaVisible provides mediaVisible,
            LocalMediaRowId provides message.id,
        ) {
            MessageRow(
                message,
                presentation,
                model,
                grouped = grouped,
                canReply = canReply,
                onReply = { interactions.send(TimelineInteraction.Reply(message.id)) },
                highlight = highlighted,
                onMention = { interactions.send(TimelineInteraction.Mention(it)) },
                onJump = { interactions.send(TimelineInteraction.Jump(it)) },
                onActions = { interactions.send(TimelineInteraction.Select(message.id)) },
                onReaction = { interactions.send(TimelineInteraction.React(message.id, it)) },
                onReactionPeople = {
                    interactions.send(TimelineInteraction.People(message.id, it))
                },
                media = {
                    MessageMedia(
                        message,
                        presentation,
                        model,
                        activePlayer,
                        { interactions.send(TimelineInteraction.Player(it)) },
                        { interactions.send(TimelineInteraction.Select(message.id)) },
                        onEmbedActions = {
                            interactions.send(TimelineInteraction.Select(message.id, it.id))
                        },
                    )
                },
                replyMessage = replyMessage,
                showCompactTime = showCompactTime,
            )
        }
    }
}
