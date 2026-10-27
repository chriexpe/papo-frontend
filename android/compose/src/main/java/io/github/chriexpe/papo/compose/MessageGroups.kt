package io.github.chriexpe.papo.compose

import java.time.Instant
import java.time.ZoneId

/**
 * Presentation only: keep each message independently keyed for reading, media retention, replies
 * and gestures, while sharing an author heading.
 */
internal fun messageContinuations(
    messages: List<ChatMessage>,
    zone: ZoneId = ZoneId.systemDefault(),
): Set<String> {
    fun identity(message: ChatMessage): Pair<String, String> =
        message.externalAuthor?.let { "external:${it.source}" to it.id }
            ?: ("papo" to message.authorId)
    return messages
        .zipWithNext()
        .filter { (previous, current) ->
            identity(previous) == identity(current) &&
                current.at >= previous.at &&
                current.at - previous.at < 5 * 60 * 1000 &&
                Instant.ofEpochMilli(previous.at).atZone(zone).toLocalDate() ==
                    Instant.ofEpochMilli(current.at).atZone(zone).toLocalDate()
        }
        .map { (_, current) -> current.id }
        .toSet()
}
