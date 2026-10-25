package io.github.chriexpe.papo.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.core.app.ApplicationProvider
import io.github.chriexpe.papo.core.NativeClient
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class MessagePresentationTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun readingAndOffscreenArrivalsDoNotRecomposeContentButEditsAndResourcesDo() {
        val client = NativeClient.demo()
        try {
            val initial = client.poll()
            val state = mutableStateOf(initial)
            val renders = AtomicInteger()
            compose.setContent {
                MaterialTheme {
                    val current = state.value
                    PresentationProbe(
                        current.messages[0],
                        current.messages[1],
                        rememberMessagePresentation(current),
                        renders,
                    )
                }
            }
            compose.waitForIdle()
            assertEquals(1, renders.get())
            repeat(5) { n ->
                compose.runOnIdle {
                    state.value =
                        state.value.copy(
                            notificationCount = n.toUInt(),
                            reading = state.value.reading.copy(gapStart = "unread-$n"),
                        )
                }
            }
            compose.waitForIdle()
            assertEquals("Cursor updates must not recompose message content", 1, renders.get())
            compose.runOnIdle {
                val previous = state.value
                state.value =
                    previous
                        .copy(
                            messages =
                                previous.messages + previous.messages.last().copy(id = "offscreen")
                        )
                        .retainPresentation(previous)
            }
            compose.waitForIdle()
            assertEquals(
                "An offscreen arrival must not invalidate the visible body",
                1,
                renders.get(),
            )
            compose.runOnIdle {
                state.value =
                    state.value.copy(
                        messages =
                            state.value.messages.mapIndexed { i, m ->
                                if (i == 0) m.copy(content = "Edited visible message") else m
                            }
                    )
            }
            compose.onNodeWithTag("probe-body").assertTextEquals("Edited visible message")
            assertEquals(2, renders.get())
            compose.runOnIdle {
                state.value =
                    state.value.copy(
                        messages =
                            state.value.messages.mapIndexed { i, m ->
                                if (i == 1) m.copy(content = "Edited reply preview") else m
                            }
                    )
            }
            compose.onNodeWithTag("probe-reply").assertTextEquals("Edited reply preview")
            assertEquals(3, renders.get())
            compose.runOnIdle {
                state.value =
                    state.value.copy(
                        members =
                            state.value.members.mapIndexed { i, person ->
                                if (i == 0) person.copy(name = "Updated author") else person
                            }
                    )
            }
            compose.onNodeWithTag("probe-author").assertTextEquals("Updated author")
            assertEquals(4, renders.get())
        } finally {
            client.shutdown()
            client.close()
        }
    }

    @Test
    fun inheritedProfilesDoNotInvalidateContentAndActionsUseTheLatestHandler() {
        val client = NativeClient.demo()
        try {
            val state = mutableStateOf(client.poll())
            val model = ApplicationProvider.getApplicationContext<PapoApplication>().model
            val renders = AtomicInteger()
            var invoked: String? = null
            compose.setContent {
                MaterialTheme {
                    val current = state.value
                    CompositionLocalProvider(
                        LocalProfileContext provides ProfileContext(current, model),
                        LocalOpenProfile provides
                            { id ->
                                invoked = "$id/${current.notificationCount}"
                            },
                    ) {
                        MessagePresentationScope(rememberMessagePresentation(current), model) {
                            ProfilePresentationProbe(renders)
                        }
                    }
                }
            }
            compose.waitForIdle()
            assertEquals(1, renders.get())
            repeat(5) { n ->
                compose.runOnIdle {
                    state.value = state.value.copy(notificationCount = (n + 1).toUInt())
                }
            }
            compose.waitForIdle()
            assertEquals("Profile locals must not bypass rendering isolation", 1, renders.get())
            compose.onNodeWithTag("profile-probe").performClick()
            compose.runOnIdle { assertEquals("author/5", invoked) }
            compose.runOnIdle {
                state.value =
                    state.value.copy(
                        members =
                            state.value.members.mapIndexed { i, person ->
                                if (i == 0) person.copy(name = "Changed profile") else person
                            }
                    )
            }
            compose.onNodeWithTag("profile-probe").assertTextEquals("Changed profile")
            assertEquals(2, renders.get())
        } finally {
            client.shutdown()
            client.close()
        }
    }

    @Test
    fun fullSnapshotsRetainUnchangedObjectsWithoutHidingEditsOrCrossingAccounts() {
        val client = NativeClient.demo()
        try {
            val old = client.poll()
            val fresh =
                old.copy(
                    messages = old.messages.map { it.copy() },
                    members = old.members.map { it.copy() },
                    emojis = old.emojis.map { it.copy() },
                )
            val retained = fresh.retainPresentation(old)
            assertSame(old.messages, retained.messages)
            assertSame(old.members, retained.members)
            assertSame(old.emojis, retained.emojis)
            val changed =
                fresh
                    .copy(
                        messages =
                            fresh.messages.mapIndexed { i, m ->
                                if (i == 1) m.copy(content = "Changed body") else m
                            }
                    )
                    .retainPresentation(old)
            assertSame(old.messages[0], changed.messages[0])
            assertEquals("Changed body", changed.messages[1].content)
            assertNotSame(old.messages[1], changed.messages[1])
            val other = fresh.copy(me = "another-owner")
            assertSame(other, other.retainPresentation(old))
        } finally {
            client.shutdown()
            client.close()
        }
    }
}

@Composable
private fun ProfilePresentationProbe(renders: AtomicInteger) {
    val profile = LocalProfileContext.current!!
    val open = LocalOpenProfile.current
    SideEffect { renders.incrementAndGet() }
    Text(
        profile.server.members.first().name,
        Modifier.testTag("profile-probe").clickable { open("author") },
    )
}

@Composable
private fun PresentationProbe(
    message: ChatMessage,
    reply: ChatMessage,
    presentation: ServerState,
    renders: AtomicInteger,
) {
    SideEffect { renders.incrementAndGet() }
    Column {
        Text(message.content, Modifier.testTag("probe-body"))
        Text(reply.content, Modifier.testTag("probe-reply"))
        Text(presentation.members.first().name, Modifier.testTag("probe-author"))
    }
}
