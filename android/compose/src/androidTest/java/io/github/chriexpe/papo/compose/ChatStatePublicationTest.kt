package io.github.chriexpe.papo.compose

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import io.github.chriexpe.papo.core.NativeClient
import io.github.chriexpe.papo.core.ReadingView
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class ChatStatePublicationTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun boundaryAdvancesOnlyRecomposeTheMarkerConsumer() {
        val client = NativeClient.demo()
        try {
            val server = client.poll().let { it.copy(reading = it.reading.copy(hasCatchUp = true)) }
            val publication = ChatStatePublication()
            publication.publish(PapoState(listOf(server), server.url))
            val roots = AtomicInteger()
            val markers = AtomicInteger()
            compose.setContent {
                MaterialTheme { PublicationProbe(publication, roots, markers) }
            }
            compose.waitForIdle()
            val original = publication.state.value
            assertEquals(1, roots.get())
            assertEquals(1, markers.get())
            repeat(5) { n ->
                compose.runOnIdle {
                    val next = server.copy(reading = server.reading.copy(gapEnd = "boundary-$n"))
                    assertFalse(publication.publish(PapoState(listOf(next), server.url)))
                }
                compose.onNodeWithTag("publication-marker").assertTextEquals("boundary-$n")
            }
            assertSame(
                "Read progress must not emit application/timeline state",
                original,
                publication.state.value,
            )
            assertEquals("Moving IDs must not recompose the conversation", 1, roots.get())
            assertEquals(6, markers.get())
            compose.runOnIdle {
                val next = server.copy(reading = server.reading.copy(hasCatchUp = false))
                assertFalse(publication.publish(PapoState(listOf(next), server.url)))
            }
            compose.onNodeWithTag("publication-control").assertTextEquals("false")
            assertEquals("Finishing a block must update the hourglass controls", 2, roots.get())
        } finally {
            client.shutdown()
            client.close()
        }
    }

    @Test
    fun nativeReadProgressStaysCurrentWhileContentBadgesAndAccountsStillPublish() {
        val client = NativeClient.demo()
        try {
            var server = client.poll()
            val publication = ChatStatePublication()
            publication.publish(PapoState(listOf(server), server.url))
            val read = publication.reading(server)
            val messages = server.messages
            for ((ids, latest) in
                listOf(
                    listOf(messages.first().id) to false,
                    listOf(messages.last().id) to true,
                    listOf(messages[messages.lastIndex - 1].id) to false,
                )) {
                client.dispatch(io.github.chriexpe.papo.core.Action.BreakReadSpan(server.selected))
                val delta =
                    requireNotNull(client.observeViewport(server.selected, ids, latest, server.me))
                server = server.withViewport(delta)
                publication.publish(PapoState(listOf(server), server.url))
                assertEquals(client.poll().reading, read.value)
                assertEquals(server.channels, publication.state.value.active!!.channels)
                assertEquals(
                    server.notificationCount,
                    publication.state.value.active!!.notificationCount,
                )
            }
            val advanced = read.value
            publication.clearError()
            assertEquals(
                "Clearing an error must not restore the snapshot's older reading seed",
                advanced,
                read.value,
            )
            val edited =
                server.copy(
                    messages =
                        server.messages.mapIndexed { i, m ->
                            if (i == 0) m.copy(content = "edited") else m
                        }
                )
            assertTrue(publication.publish(PapoState(listOf(edited), server.url)))
            assertEquals("edited", publication.state.value.active!!.messages.first().content)
            val other =
                edited.copy(
                    me = "other-owner",
                    reading = edited.reading.copy(gapEnd = "other-boundary"),
                )
            assertTrue(publication.publish(PapoState(listOf(other), server.url)))
            assertNotSame(read, publication.reading(other))
            assertEquals(
                "Account replacement cannot publish into the previous reader",
                advanced,
                read.value,
            )
            assertEquals("other-boundary", publication.reading(other).value.gapEnd)
            val channel =
                other.copy(
                    selectedChannel = "another-channel",
                    reading = other.reading.copy(gapEnd = "another-boundary"),
                )
            publication.publish(PapoState(listOf(channel), channel.url))
            assertEquals("another-boundary", publication.reading(channel).value.gapEnd)
        } finally {
            client.shutdown()
            client.close()
        }
    }
}

@Composable
private fun PublicationProbe(
    publication: ChatStatePublication,
    roots: AtomicInteger,
    markers: AtomicInteger,
) {
    val application by publication.state.collectAsState()
    val server = requireNotNull(application.active)
    val reading =
        remember(server.url, server.me, server.selected) { publication.reading(server) }
            .collectAsState()
    val controls by rememberReadingControls(reading)
    SideEffect { roots.incrementAndGet() }
    Text(controls.hasCatchUp.toString(), Modifier.testTag("publication-control"))
    ReadingMarkerProbe(reading, markers)
}

@Composable
private fun ReadingMarkerProbe(reading: State<ReadingView>, count: AtomicInteger) {
    val value = reading.value
    SideEffect { count.incrementAndGet() }
    Text(value.gapEnd, Modifier.testTag("publication-marker"))
}
