package io.github.chriexpe.papo.compose

import io.github.chriexpe.papo.core.ViewportSnapshot

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
