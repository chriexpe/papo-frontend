package io.github.chriexpe.papo.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.structuralEqualityPolicy
import io.github.chriexpe.papo.core.ReadingView
import io.github.chriexpe.papo.core.ViewportSnapshot

/** Only control changes invalidate the conversation; moving boundary IDs do not. */
internal data class ReadingControls(
    val liveIds: List<String>,
    val mentions: List<String>,
    val hasCatchUp: Boolean,
    val seekingCatchUp: Boolean,
)

@Composable
internal fun rememberReadingControls(reading: State<ReadingView>): State<ReadingControls> =
    remember(reading) {
        derivedStateOf(structuralEqualityPolicy()) {
            val value = reading.value
            ReadingControls(value.liveIds, value.mentions, value.hasCatchUp, value.seekingCatchUp)
        }
    }

/** Patch read state while retaining the existing message and resource objects. */
internal fun ServerState.withViewport(update: ViewportSnapshot): ServerState {
    if (
        me != update.ownerUserId ||
            selected != update.channelId ||
            reading.liveIds != update.reading.liveIds
    )
        return this
    fun List<Conversation>.updated(): List<Conversation> {
        if (
            none {
                it.id == update.channelId &&
                    (it.unread != update.channelUnread || it.mentions != update.channelMentions)
            }
        )
            return this
        return map {
            if (it.id == update.channelId)
                it.copy(unread = update.channelUnread, mentions = update.channelMentions)
            else it
        }
    }
    val nextChannels = channels.updated()
    val nextDms = dms.updated()
    val nextReading =
        if (reading == update.reading) reading else update.reading.copy(liveIds = reading.liveIds)
    if (
        nextChannels === channels &&
            nextDms === dms &&
            nextReading === reading &&
            notificationCount == update.notificationCount
    )
        return this
    return copy(
        channels = nextChannels,
        dms = nextDms,
        reading = nextReading,
        notificationCount = update.notificationCount,
    )
}
