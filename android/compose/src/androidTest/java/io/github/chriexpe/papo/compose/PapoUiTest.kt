package io.github.chriexpe.papo.compose

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.text.AnnotatedString
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.stateDescription
import android.content.SharedPreferences
import org.junit.runner.RunWith
import androidx.test.ext.junit.runners.AndroidJUnit4

/** Runs the real Activity, UniFFI library and shared Rust reducer on a device. */
@RunWith(AndroidJUnit4::class)
class PapoUiTest {
    @get:Rule val compose = createEmptyComposeRule()
    private lateinit var activity: ActivityScenario<MainActivity>

    private lateinit var savedClient: Map<String, *>
    private lateinit var savedAppearance: Map<String, *>

    @Before fun launch() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        savedClient = context.getSharedPreferences("compose-client", 0).all.toMap()
        savedAppearance = context.getSharedPreferences("compose-appearance", 0).all.toMap()
        context.getSharedPreferences("compose-client", 0).edit().clear().commit()
        context.getSharedPreferences("compose-appearance", 0).edit().clear().commit()
        activity = ActivityScenario.launch(MainActivity::class.java)
    }
    @After fun close() {
        activity.close()
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        fun restore(name: String, values: Map<String, *>) {
            val editor = context.getSharedPreferences(name, 0).edit().clear()
            values.forEach { (key, value) -> when (value) { is String -> editor.putString(key, value); is Boolean -> editor.putBoolean(key, value) } }
            editor.commit()
        }
        restore("compose-client", savedClient)
        restore("compose-appearance", savedAppearance)
    }

    private fun demo() {
        compose.onNodeWithText("Explore demo").performClick()
        compose.waitUntil(10_000) { compose.onAllNodesWithText("DEMO SPACE").fetchSemanticsNodes().isNotEmpty() }
    }
    private fun chat() {
        demo()
        compose.onNodeWithText("geral").performClick()
        compose.waitUntil(10_000) { compose.onAllNodesWithText("Papo · Demo").fetchSemanticsNodes().isNotEmpty() }
    }

    @Test fun firstLaunchHasNoPrefilledServerAndValidatesAddressesInline() {
        compose.onNodeWithText("Add server").assertIsNotEnabled()
        compose.onNode(hasSetTextAction()).assert(SemanticsMatcher.expectValue(SemanticsProperties.EditableText, AnnotatedString("")))
        compose.onNode(hasSetTextAction()).performTextInput("file:///tmp/papo")
        compose.onNodeWithText("Add server").performScrollTo().assertIsEnabled().performClick()
        compose.waitUntil(10_000) { compose.onAllNodesWithText("Use an http(s) server address", substring = true).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithText("Explore demo").assertExists()
    }

    @Test fun nativeDemoSendsUnicodeAndKeepsDraftsAcrossConversationNavigation() {
        chat()
        val draft = "rascunho 🌱\nsegunda linha"
        compose.onNode(hasSetTextAction()).performTextInput(draft)
        compose.onNodeWithContentDescription("Back to conversations").performClick()
        compose.onNodeWithText("geral").performClick()
        compose.onNode(hasSetTextAction()).assertTextEquals(draft)
        compose.onNodeWithContentDescription("Send message").performClick()
        compose.waitUntil(10_000) { compose.onAllNodesWithText(draft).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithContentDescription("Send message").assertIsNotEnabled()
    }

    @Test fun appearanceControlsAndDemoExitWork() {
        demo()
        compose.onNodeWithText("Settings").performClick()
        compose.onNodeWithText("Dark").performClick()
        compose.onNodeWithContentDescription("Frosted controls").performClick()
        compose.onNodeWithText("Leave demo").performScrollTo().performClick()
        compose.onNodeWithText("Explore demo").assertExists()
        compose.onNodeWithText("Add server").assertIsNotEnabled()
    }
    private fun media() {
        demo()
        compose.onNodeWithText("mídia").performClick()
        compose.waitUntil(10_000) { compose.onAllNodesWithText("Papo · Demo").fetchSemanticsNodes().isNotEmpty() }
    }

    @Test fun portraitEmbedReservesServerDimensionsAndOpensViewer() {
        media()
        compose.onNodeWithTag("message-list", useUnmergedTree = true).performScrollToNode(hasTestTag("embed-embed-portrait"))
        val frame = compose.onNodeWithTag("embed-embed-portrait", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        assertEquals(540f / 960f, frame.width / frame.height, 0.01f)
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasContentDescription("Portrait preview") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Loaded")).fetchSemanticsNodes().isNotEmpty()
        }
        val loaded = compose.onNodeWithTag("embed-embed-portrait", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        assertEquals(frame.height, loaded.height, 1f)
        compose.onNodeWithContentDescription("Portrait preview").performClick()
        compose.onNodeWithContentDescription("Close image").assertExists().performClick()
    }

    @Test fun videoAttachmentDecodesAndKeepsItsReservedFrame() {
        media()
        compose.onNodeWithTag("message-list").performScrollToNode(hasContentDescription("Play clip.mp4"))
        val frame = compose.onNodeWithTag("attachment-demo-video", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        compose.onNodeWithContentDescription("Play clip.mp4").performClick()
        val player = "player-demo://papo/me/attachment/demo-video"
        compose.waitUntil(15_000) { compose.onAllNodes(hasTestTag(player) and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready")).fetchSemanticsNodes().isNotEmpty() }
        val playing = compose.onNodeWithTag("attachment-demo-video", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        assertEquals(frame.height, playing.height, 1f)
    }

}
