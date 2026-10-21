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
import org.junit.runner.RunWith
import androidx.test.ext.junit.runners.AndroidJUnit4

/** Runs the real Activity, JNI library and shared Rust reducer on a device. */
@RunWith(AndroidJUnit4::class)
class PapoUiTest {
    @get:Rule val compose = createEmptyComposeRule()
    private lateinit var activity: ActivityScenario<MainActivity>

    @Before fun launch() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        context.getSharedPreferences("compose-client", 0).edit().clear().commit()
        context.getSharedPreferences("compose-appearance", 0).edit().clear().commit()
        activity = ActivityScenario.launch(MainActivity::class.java)
    }
    @After fun close() { activity.close() }

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
}
