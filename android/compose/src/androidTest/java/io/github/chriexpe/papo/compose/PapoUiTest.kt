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
    private lateinit var savedExpressions: Map<String, *>
    private lateinit var savedAttachments: Map<String, *>

    @Before fun launch() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        savedExpressions = context.getSharedPreferences("compose-expressions", 0).all.toMap()
        context.getSharedPreferences("compose-expressions", 0).edit().clear().commit()
        savedAttachments = context.getSharedPreferences("compose-attachments", 0).all.toMap()
        context.getSharedPreferences("compose-attachments", 0).edit().clear().commit()
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
        restore("compose-expressions", savedExpressions)
        restore("compose-attachments", savedAttachments)
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
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasTestTag("channel-header") and hasText("geral")).fetchSemanticsNodes().isNotEmpty() &&
                compose.onAllNodesWithTag("message-list").fetchSemanticsNodes().isNotEmpty()
        }
    }

    @Test fun serverStickersInsertAnimateAndAppearInSharedReactionPicker() {
        chat()
        compose.onNodeWithContentDescription("Emoji and stickers").performClick()
        compose.onNodeWithTag("emoji-picker").assertIsDisplayed()
        compose.onNodeWithContentDescription("Sticker :papo_wave:").performClick()
        androidx.test.espresso.Espresso.pressBack()
        compose.onNode(hasSetTextAction()).assertTextEquals(":papo_wave:")
        compose.onNodeWithContentDescription("Send message").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription(":papo_wave:",useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        compose.onAllNodesWithContentDescription("Message actions").onFirst().performClick()
        compose.onNodeWithContentDescription("More reactions").performClick()
        compose.onNodeWithTag("emoji-picker").assertIsDisplayed()
        compose.onNodeWithContentDescription("Sticker :papo_wave:").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription("See who reacted with papo_wave").fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithContentDescription("See who reacted with papo_wave").performClick()
        compose.onNodeWithTag("reaction-people").assertIsDisplayed()
        compose.waitUntil(10_000) {compose.onAllNodesWithText("You",useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithText("Remove my reaction").assertExists()
    }

    @Test fun gifPickerUsesStableProviderMarkerAndTracksRecentOnlyAfterSending() {
        chat()
        compose.onNodeWithContentDescription("GIF picker").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription("A little motion").fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithContentDescription("A little motion").performClick()
        compose.waitUntil(10_000) {compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().any {it.config.getOrElse(SemanticsProperties.EditableText) {AnnotatedString("")}.text=="giphy:demo-motion"}}
        compose.onNodeWithContentDescription("Send message").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithTag("provider-gif-demo-motion",useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithContentDescription("GIF picker").performClick()
        compose.onNodeWithText("Recent").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription("A little motion").fetchSemanticsNodes().isNotEmpty()}
        compose.onNode(hasContentDescription("A little motion") and hasAnyAncestor(hasTestTag("gif-picker"))).assertIsDisplayed()
    }

    @Test fun uploadedGifFavoriteIsSharedWithFullscreenAndPicker() {
        media()
        compose.onNodeWithTag("message-list").performScrollToNode(hasContentDescription("Photo: animation.gif"))
        compose.waitUntil(10_000) {compose.onAllNodes(hasContentDescription("Photo: animation.gif") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Loaded")).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithContentDescription("Photo: animation.gif").performClick()
        compose.onAllNodesWithContentDescription("Favorite GIF").onLast().performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription("Remove GIF from favorites").fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithContentDescription("Close image").performClick()
        compose.onNodeWithContentDescription("GIF picker").performClick()
        compose.onNodeWithText("Favorites").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription("animation.gif").fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithContentDescription("animation.gif").assertIsDisplayed().performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription("Remove animation.gif.gif").fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithContentDescription("Send message").assertIsEnabled()
        compose.onNodeWithContentDescription("Remove animation.gif.gif").performClick()
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
        val sendCenter=compose.onNodeWithContentDescription("Send message").fetchSemanticsNode().boundsInRoot.center.y
        assertEquals(sendCenter,compose.onNodeWithContentDescription("Emoji and stickers").fetchSemanticsNode().boundsInRoot.center.y,1f)
        assertEquals(sendCenter,compose.onNodeWithContentDescription("GIF picker").fetchSemanticsNode().boundsInRoot.center.y,1f)
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
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasTestTag("channel-header") and hasText("mídia")).fetchSemanticsNodes().isNotEmpty() &&
                compose.onAllNodesWithTag("message-list").fetchSemanticsNodes().isNotEmpty()
        }
    }

    @Test fun attachmentRollupPicksAndSendsAFileThroughTheNativeCore() {
        chat()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val file = AttachmentFiles.create(context, "txt").apply { writeText("Compose attachment fixture") }
        val uri = androidx.core.content.FileProvider.getUriForFile(context, "${context.packageName}.files", file)
        val filter = android.content.IntentFilter(android.content.Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(android.content.Intent.CATEGORY_OPENABLE); addDataType("*/*")
        }
        val monitor = instrumentation.addMonitor(filter, android.app.Instrumentation.ActivityResult(android.app.Activity.RESULT_OK,
            android.content.Intent().setData(uri).addFlags(android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION)), true)
        try {
            compose.onNodeWithContentDescription("Open gallery").performTouchInput { longClick() }
            compose.onNodeWithText("Record audio").assertIsDisplayed()
            compose.onNodeWithText("Camera").assertIsDisplayed()
            compose.onNodeWithText("Choose files").assertIsDisplayed().performClick()
            compose.waitUntil(10_000) { compose.onAllNodesWithContentDescription("Remove ${file.name}").fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithContentDescription("Send message").assertIsEnabled().performClick()
            compose.waitUntil(10_000) { compose.onAllNodesWithContentDescription("Open ${file.name}").fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithContentDescription("Remove ${file.name}").assertDoesNotExist()
            compose.onNodeWithContentDescription("Send message").assertIsNotEnabled()
        } finally { instrumentation.removeMonitor(monitor); file.delete() }
    }

    @Test fun tappingTheGalleryButtonOpensThePhotoPickerWithoutOpeningTheRollup() {
        chat()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val file = AttachmentFiles.create(context, "jpg")
        val bitmap = android.graphics.Bitmap.createBitmap(24, 24, android.graphics.Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(android.graphics.Color.GREEN)
        file.outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 90, it) }; bitmap.recycle()
        val uri = androidx.core.content.FileProvider.getUriForFile(context, "${context.packageName}.files", file)
        var galleryOpened = false
        val monitor = object : android.app.Instrumentation.ActivityMonitor() {
            override fun onStartActivity(intent: android.content.Intent): android.app.Instrumentation.ActivityResult? {
                if (intent.action !in listOf("android.provider.action.PICK_IMAGES", android.content.Intent.ACTION_OPEN_DOCUMENT, android.content.Intent.ACTION_GET_CONTENT)) return null
                galleryOpened = true
                return android.app.Instrumentation.ActivityResult(android.app.Activity.RESULT_OK, android.content.Intent().setData(uri))
            }
        }
        instrumentation.addMonitor(monitor)
        try {
            compose.onNodeWithContentDescription("Open gallery").performClick()
            compose.waitUntil(10_000) { compose.onAllNodesWithContentDescription("Remove ${file.name}").fetchSemanticsNodes().isNotEmpty() }
            assertTrue(galleryOpened)
            compose.onNodeWithText("Choose files").assertDoesNotExist()
            compose.onNodeWithContentDescription("Remove ${file.name}").performClick()
        } finally { instrumentation.removeMonitor(monitor); file.delete() }
    }

    @Test fun cameraCaptureStagesAPhotoWithoutSendingIt() {
        chat()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val monitor = object : android.app.Instrumentation.ActivityMonitor() {
            @Suppress("DEPRECATION")
            override fun onStartActivity(intent: android.content.Intent): android.app.Instrumentation.ActivityResult? {
                if (intent.action != android.provider.MediaStore.ACTION_IMAGE_CAPTURE) return null
                val uri = intent.getParcelableExtra<android.net.Uri>(android.provider.MediaStore.EXTRA_OUTPUT)!!
                val bitmap = android.graphics.Bitmap.createBitmap(24, 24, android.graphics.Bitmap.Config.ARGB_8888)
                bitmap.eraseColor(android.graphics.Color.BLUE)
                context.contentResolver.openOutputStream(uri)!!.use { bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 90, it) }
                bitmap.recycle()
                return android.app.Instrumentation.ActivityResult(android.app.Activity.RESULT_OK, null)
            }
        }
        instrumentation.addMonitor(monitor)
        try {
            compose.onNodeWithContentDescription("Open gallery").performTouchInput { longClick() }
            compose.onNodeWithText("Camera").performClick()
            compose.waitUntil(10_000) { compose.onAllNodesWithContentDescription("Remove Photo ", substring = true).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithContentDescription("Send message").assertIsEnabled()
            compose.onNodeWithContentDescription("Remove Photo ", substring = true).performClick()
            compose.waitUntil(5000) { compose.onAllNodesWithContentDescription("Remove Photo ", substring = true).fetchSemanticsNodes().isEmpty() }
            compose.onNodeWithContentDescription("Send message").assertIsNotEnabled()
        } finally { instrumentation.removeMonitor(monitor) }
    }

    @Test fun recordsAudioAttachesItAndPlaysTheDemoMessage() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.grantRuntimePermission(instrumentation.targetContext.packageName, android.Manifest.permission.RECORD_AUDIO)
        chat()
        compose.onNodeWithContentDescription("Open gallery").performTouchInput { longClick() }
        compose.onNodeWithText("Record audio").performClick()
        compose.waitUntil(10_000) { compose.onAllNodesWithContentDescription("Send recording").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithContentDescription("Live microphone waveform").assertIsDisplayed()
        android.os.SystemClock.sleep(1200)
        compose.onNodeWithContentDescription("Send recording").assertIsEnabled().performClick()
        compose.waitUntil(10_000) { compose.onAllNodesWithContentDescription("Play Voice note ", substring = true).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithContentDescription("Play Voice note ", substring = true).performClick()
        compose.waitUntil(15_000) { compose.onAllNodes(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready")).fetchSemanticsNodes().isNotEmpty() }
    }

    @Test fun rotatingDuringRecordingKeepsTheVoiceNoteAndCaptionWithoutSending() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.grantRuntimePermission(instrumentation.targetContext.packageName, android.Manifest.permission.RECORD_AUDIO)
        chat()
        compose.onNode(hasSetTextAction()).performTextInput("Keep this caption")
        compose.onNodeWithContentDescription("Open gallery").performTouchInput { longClick() }
        compose.onNodeWithText("Record audio").performClick()
        compose.waitUntil(10_000) { compose.onAllNodesWithContentDescription("Send recording").fetchSemanticsNodes().isNotEmpty() }
        android.os.SystemClock.sleep(1200)
        activity.recreate()
        compose.waitUntil(10_000) { compose.onAllNodesWithContentDescription("Remove Voice note ", substring = true).fetchSemanticsNodes().isNotEmpty() }
        compose.onNode(hasSetTextAction()).assertTextEquals("Keep this caption")
        compose.onNodeWithContentDescription("Play Voice note ", substring = true).assertDoesNotExist()
        compose.onNodeWithContentDescription("Remove Voice note ", substring = true).performClick()
    }

    @Test fun backendMislabeledM4aUsesAudioControlsAndKeepsItsCompactFrame() {
        media()
        compose.onNodeWithTag("message-list").performScrollToNode(hasContentDescription("Play voice-note.m4a"))
        val frame = compose.onNodeWithTag("audio-attachment-demo-audio", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        compose.onNodeWithTag("attachment-demo-audio", useUnmergedTree = true).assertDoesNotExist()
        compose.waitUntil(15_000) { compose.onAllNodes(hasTestTag("waveform-demo://papo/me/attachment/demo-audio") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Waveform ready"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithContentDescription("Play voice-note.m4a").performClick()
        compose.waitUntil(15_000) { compose.onAllNodes(hasTestTag("player-demo://papo/me/attachment/demo-audio") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("audio-controls", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithContentDescription("Audio position").assertExists()
        val loaded = compose.onNodeWithTag("audio-attachment-demo-audio", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        assertEquals(frame.height, loaded.height, 1f)
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
        compose.waitUntil(15_000) { compose.onAllNodes(hasTestTag(player) and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        val playing = compose.onNodeWithTag("attachment-demo-video", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        assertEquals(frame.height, playing.height, 1f)
    }

}
