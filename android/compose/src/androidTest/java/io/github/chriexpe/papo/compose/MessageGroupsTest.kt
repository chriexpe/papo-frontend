package io.github.chriexpe.papo.compose

import io.github.chriexpe.papo.core.ExternalAuthorView
import io.github.chriexpe.papo.core.NativeClient
import java.time.Instant
import java.time.ZoneOffset
import org.junit.Assert.*
import org.junit.Test

class MessageGroupsTest {
    @Test
    fun dateRowsUseLocalDaysAndKeepMessagePositionsSeparateFromHeaders() {
        val client = NativeClient.demo()
        try {
            val first =
                client
                    .poll()
                    .messages
                    .first()
                    .copy(id = "first", at = Instant.parse("2026-10-24T12:00:00Z").toEpochMilli())
            val messages =
                listOf(
                    first,
                    first.copy(id = "second", at = first.at + 1000),
                    first.copy(id = "third", at = first.at + 60000),
                    first.copy(id = "next-day", at = first.at + 86400000),
                )
            val index = MessageTimelineIndex(messages, ZoneOffset.UTC)
            assertEquals(setOf("first", "third", "next-day"), index.timestampIds)
            assertEquals(setOf("first", "next-day"), index.dateStarts.keys)
            assertEquals(0, index.position("next-day"))
            assertEquals(2, index.position("third"))
            assertEquals(4, index.position("first"))
            assertNull(index.position("date/first"))
            assertEquals(
                listOf("next-day", "date/next-day", "third", "second", "first", "date/first"),
                index.rows.map { it.key },
            )
            val midnight = Instant.parse("2026-10-25T00:00:00Z").toEpochMilli()
            val aroundMidnight =
                listOf(
                    first.copy(at = midnight - 1000),
                    first.copy(id = "after", at = midnight + 1000),
                )
            assertEquals(2, MessageTimelineIndex(aroundMidnight, ZoneOffset.UTC).dateStarts.size)
            assertEquals(
                1,
                MessageTimelineIndex(aroundMidnight, java.time.ZoneId.of("America/Sao_Paulo"))
                    .dateStarts
                    .size,
            )
        } finally {
            client.close()
        }
    }

    @Test
    fun groupingDependsOnMessageContentRatherThanReadingState() {
        val native = NativeClient.demo()
        try {
            val source = native.poll().messages.first()
            val first =
                source.copy(id = "first", at = Instant.parse("2026-10-21T12:00:00Z").toEpochMilli())
            val second = first.copy(id = "second", at = first.at + 1000)
            val rows = listOf(first, second)
            assertEquals(
                setOf("second"),
                messageContinuations(rows, ZoneOffset.UTC),
            )
            assertTrue(
                messageContinuations(listOf(first, second.copy(authorId = "someone-else")))
                    .isEmpty()
            )
            assertTrue(
                messageContinuations(listOf(first, second.copy(at = first.at + 300000))).isEmpty()
            )
            val midnight = Instant.parse("2026-10-22T00:00:00Z").toEpochMilli()
            assertTrue(
                messageContinuations(
                        listOf(first.copy(at = midnight - 1000), second.copy(at = midnight)),
                        ZoneOffset.UTC,
                    )
                    .isEmpty()
            )
            val external = ExternalAuthorView("wormhole", "one", "External", "", "")
            assertTrue(
                messageContinuations(
                        listOf(
                            first.copy(externalAuthor = external),
                            second.copy(externalAuthor = external.copy(id = "two")),
                        )
                    )
                    .isEmpty()
            )
            assertEquals(
                setOf("second"),
                messageContinuations(
                    listOf(
                        first.copy(externalAuthor = external),
                        second.copy(externalAuthor = external),
                    )
                ),
            )
        } finally {
            native.shutdown()
            native.close()
        }
    }
}
