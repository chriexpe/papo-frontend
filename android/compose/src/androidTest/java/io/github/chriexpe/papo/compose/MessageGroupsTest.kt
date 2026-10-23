package io.github.chriexpe.papo.compose

import io.github.chriexpe.papo.core.ExternalAuthorView
import io.github.chriexpe.papo.core.NativeClient
import java.time.Instant
import java.time.ZoneOffset
import org.junit.Assert.*
import org.junit.Test

class MessageGroupsTest {
    @Test
    fun groupingRespectsIdentityTimeDayAndUnreadBoundaries() {
        val native = NativeClient.demo()
        try {
            val source = native.poll().messages.first()
            val first =
                source.copy(id = "first", at = Instant.parse("2026-10-21T12:00:00Z").toEpochMilli())
            val second = first.copy(id = "second", at = first.at + 1000)
            val rows = listOf(first, second)
            assertEquals(
                setOf("second"),
                messageContinuations(rows, emptySet(), emptySet(), ZoneOffset.UTC),
            )
            assertTrue(messageContinuations(rows, setOf("second"), emptySet()).isEmpty())
            assertTrue(messageContinuations(rows, emptySet(), setOf("first")).isEmpty())
            assertTrue(
                messageContinuations(
                        listOf(first, second.copy(authorId = "someone-else")),
                        emptySet(),
                        emptySet(),
                    )
                    .isEmpty()
            )
            assertTrue(
                messageContinuations(
                        listOf(first, second.copy(at = first.at + 300000)),
                        emptySet(),
                        emptySet(),
                    )
                    .isEmpty()
            )
            val midnight = Instant.parse("2026-10-22T00:00:00Z").toEpochMilli()
            assertTrue(
                messageContinuations(
                        listOf(first.copy(at = midnight - 1000), second.copy(at = midnight)),
                        emptySet(),
                        emptySet(),
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
                        ),
                        emptySet(),
                        emptySet(),
                    )
                    .isEmpty()
            )
            assertEquals(
                setOf("second"),
                messageContinuations(
                    listOf(
                        first.copy(externalAuthor = external),
                        second.copy(externalAuthor = external),
                    ),
                    emptySet(),
                    emptySet(),
                ),
            )
        } finally {
            native.shutdown()
            native.close()
        }
    }
}
