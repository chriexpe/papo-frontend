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
import androidx.compose.ui.graphics.asAndroidBitmap
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

    @Test fun chatProfilesShowImagesAndHeldAuthorsInsertMentions() {
        chat()
        compose.onNodeWithTag("author-general-9",useUnmergedTree=true).performClick()
        compose.onNodeWithTag("user-profile").assertIsDisplayed()
        compose.onNodeWithText("@ana",substring=false).assertIsDisplayed()
        compose.onNodeWithText("Designers").assertIsDisplayed()
        compose.onNodeWithText("Desenhando ideias").assertIsDisplayed()
        compose.onNodeWithText("Design, café e boas conversas.",substring=true).assertIsDisplayed()
        compose.waitUntil(10_000) {compose.onAllNodes(hasTestTag("avatar-ana") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Avatar loaded"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()?.let {image ->
            InstrumentationRegistry.getInstrumentation().targetContext.cacheDir.resolve("profile-preview.png").outputStream().use {image.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
        }
        compose.onNodeWithText("Copy username").performClick()
        compose.onNodeWithText("Copied username").assertIsDisplayed()
        val clipboard=InstrumentationRegistry.getInstrumentation().targetContext.getSystemService(android.content.ClipboardManager::class.java)
        assertEquals("ana",clipboard.primaryClip?.getItemAt(0)?.text.toString())
        compose.onNodeWithContentDescription("Close profile").performClick()
        compose.onNodeWithTag("author-general-9",useUnmergedTree=true).performTouchInput {longClick()}
        compose.onNodeWithTag("user-profile").assertDoesNotExist()
        compose.onNode(hasSetTextAction()).assert(SemanticsMatcher.expectValue(SemanticsProperties.InputText,AnnotatedString("@Ana ")))
        androidx.test.espresso.Espresso.pressBack()
        compose.onNode(hasTestTag("avatar-ana") and hasAnyAncestor(hasTestTag("message-general-9")),useUnmergedTree=true).performTouchInput {longClick()}
        compose.onNode(hasSetTextAction()).assert(SemanticsMatcher.expectValue(SemanticsProperties.InputText,AnnotatedString("@Ana @Ana ")))
        androidx.test.espresso.Espresso.pressBack()
        compose.onNode(hasTestTag("avatar-ana") and hasAnyAncestor(hasTestTag("message-general-9")),useUnmergedTree=true).performClick()
        compose.onNodeWithTag("user-profile").assertIsDisplayed()
    }

    @Test fun peopleOpenProfilesBeforeStartingDirectMessages() {
        demo()
        compose.onNodeWithText("Direct",substring=false).performClick()
        compose.onNodeWithTag("avatar-ana",useUnmergedTree=true).performClick()
        compose.onNodeWithTag("user-profile").assertIsDisplayed()
        compose.onNodeWithContentDescription("Close profile").performClick()
        compose.onNodeWithText("People",substring=false).performClick()
        compose.onNodeWithText("Ana",substring=false).performClick()
        compose.onNodeWithTag("user-profile").assertIsDisplayed()
        compose.onNodeWithText("Message @ana").performClick()
        compose.waitUntil(10_000) {compose.onAllNodes(hasTestTag("channel-header") and hasText("Ana")).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithTag("user-profile").assertDoesNotExist()
    }

    @Test fun wormholeAuthorsUseExternalAvatarsAndAppProfiles() {
        demo()
        compose.onNodeWithText("design",substring=false).performClick()
        compose.onNodeWithTag("author-design-0",useUnmergedTree=true).assertTextEquals("Ari")
        compose.onNodeWithText("APP",substring=false).assertIsDisplayed()
        compose.waitUntil(10_000) {compose.onAllNodes(hasTestTag("external-avatar-Wormhole-remote-ari") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Avatar loaded"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithTag("author-design-0",useUnmergedTree=true).performClick()
        compose.onNodeWithTag("external-profile").assertIsDisplayed()
        compose.onNodeWithTag("user-profile").assertDoesNotExist()
        compose.onNodeWithText("Via Wormhole").assertIsDisplayed()
        compose.onNodeWithText("Open external profile").assertIsDisplayed()
        compose.onNodeWithText("Copy username").assertDoesNotExist()
        compose.onNodeWithText("Copy name").performClick()
        compose.onNodeWithText("Copied name").assertIsDisplayed()
        compose.onNodeWithContentDescription("Close profile").performClick()
        compose.onNodeWithTag("author-design-0",useUnmergedTree=true).performTouchInput {longClick()}
        compose.onNodeWithTag("external-profile").assertIsDisplayed()
        compose.onNode(hasSetTextAction()).assert(SemanticsMatcher.expectValue(SemanticsProperties.InputText,AnnotatedString("")))
    }

    @Test fun serverStickersInsertAnimateAndAppearInSharedReactionPicker() {
        chat()
        compose.onNodeWithContentDescription("Emoji and stickers").performClick()
        compose.onNodeWithTag("emoji-picker").assertIsDisplayed()
        compose.onNodeWithContentDescription("Sticker :papo_wave:").performClick()
        androidx.test.espresso.Espresso.pressBack()
        compose.onNode(hasSetTextAction()).assert(SemanticsMatcher.expectValue(SemanticsProperties.InputText,AnnotatedString(":papo_wave: ")))
        compose.onNodeWithContentDescription("Send message").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription(":papo_wave:",useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        compose.onAllNodesWithTag("message-sticker-demo-wave").onFirst().performClick()
        compose.onNodeWithTag("sticker-details").assertIsDisplayed()
        compose.onNodeWithText("Copy name").performClick()
        compose.onNodeWithText("Copied").assertExists()
        val clipboard=InstrumentationRegistry.getInstrumentation().targetContext.getSystemService(android.content.ClipboardManager::class.java)
        assertEquals("papo_wave",clipboard.primaryClip?.getItemAt(0)?.text.toString())
        compose.onNodeWithText("Favorite",substring=false).performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithText("Favorited").fetchSemanticsNodes().isNotEmpty()}
        androidx.test.espresso.Espresso.pressBack()
        compose.onAllNodesWithContentDescription("Message actions").onFirst().performClick()
        compose.onNodeWithContentDescription("React with :papo_wave:").assertIsDisplayed()
        compose.onNodeWithContentDescription("More reactions").performClick()
        compose.onNodeWithTag("emoji-picker").assertIsDisplayed()
        compose.onNodeWithContentDescription("Sticker :papo_wave:").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription("Reaction papo_wave").fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithContentDescription("Reaction papo_wave").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription("Reaction papo_wave").fetchSemanticsNodes().isEmpty()}
        compose.onNodeWithTag("reaction-people").assertDoesNotExist()
        compose.onAllNodesWithContentDescription("Message actions").onFirst().performClick()
        compose.onNodeWithContentDescription("More reactions").performClick()
        compose.onNodeWithContentDescription("Sticker :papo_wave:").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription("Reaction papo_wave").fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithContentDescription("Reaction papo_wave").performTouchInput {longClick()}
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
        compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription("Remove animation.gif").fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithContentDescription("Send message").assertIsEnabled()
        compose.onNodeWithContentDescription("Remove animation.gif").performClick()
    }

    @Test fun mentionAndStickerSuggestionsScrollInsertAndRenderInsideTheComposer() {
        chat()
        compose.onNodeWithTag("composer-input").performTextInput("@")
        compose.onNodeWithTag("composer-suggestions").assertIsDisplayed().performScrollToNode(hasText("Dora"))
        compose.onNodeWithTag("composer-suggestions").performScrollToNode(hasText("Ana"))
        compose.onNode(hasText("Ana") and hasAnyAncestor(hasTestTag("composer-suggestions"))).performClick()
        compose.onNodeWithTag("composer-input").assert(SemanticsMatcher.expectValue(SemanticsProperties.InputText,AnnotatedString("@Ana ")))
        compose.onNodeWithTag("composer-suggestions").assertDoesNotExist()
        compose.onNodeWithTag("composer-input").performTextInput(":pap")
        compose.onNode(hasText(":papo_wave:") and hasAnyAncestor(hasTestTag("composer-suggestions"))).performClick()
        compose.waitUntil(10_000) {compose.onAllNodes(hasTestTag("composer-input") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Contains stickers: papo_wave")).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithTag("composer-input").assert(SemanticsMatcher.expectValue(SemanticsProperties.InputText,AnnotatedString("@Ana :papo_wave: ")))
        compose.onNodeWithTag("composer-suggestions").assertDoesNotExist()
        val bitmap=compose.onRoot().captureToImage()
        val context=InstrumentationRegistry.getInstrumentation().targetContext
        context.cacheDir.resolve("composer-suggestions-verified.png").outputStream().use { bitmap.asAndroidBitmap().compress(android.graphics.Bitmap.CompressFormat.PNG,100,it) }
        compose.onNodeWithContentDescription("Back to conversations").performClick()
        compose.onNodeWithText("geral").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithTag("composer-input").fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithTag("composer-input").assert(SemanticsMatcher.expectValue(SemanticsProperties.InputText,AnnotatedString("@Ana :papo_wave: ")))
        compose.onNodeWithContentDescription("Send message").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithText("@Ana",substring=true).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithText("@mention(<@ana>)",substring=true).assertDoesNotExist()
    }

    @Test fun replyPreviewsJumpHighlightAndKeepTheComposerDraft() {
        chat()
        compose.onNodeWithTag("composer-input").performTextInput("Keep this draft @An")
        compose.onNodeWithTag("message-list").performScrollToKey("general-6")
        compose.onNodeWithTag("reply-preview-general-6",useUnmergedTree=true).performClick()
        compose.waitUntil(10_000) {compose.onAllNodes(hasTestTag("message-general-5") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Original message"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithTag("message-general-5",useUnmergedTree=true).assertIsDisplayed()
        compose.onNodeWithTag("composer-input").assertTextEquals("Keep this draft @An")
        compose.onNodeWithTag("composer-suggestions").assertDoesNotExist()
        val preview=compose.onRoot().captureToImage().asAndroidBitmap()
        InstrumentationRegistry.getInstrumentation().targetContext.cacheDir.resolve("reply-jump-verified.png").outputStream().use {preview.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
        compose.waitUntil(4_000) {compose.onAllNodes(hasTestTag("message-general-5") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Original message"),useUnmergedTree=true).fetchSemanticsNodes().isEmpty()}
        compose.onNode(hasContentDescription("Message actions") and hasAnyAncestor(hasTestTag("message-general-5")),useUnmergedTree=true).performClick()
        compose.onNodeWithText("Reply",substring=false).performClick()
        compose.onNodeWithTag("composer-reply").assertIsDisplayed().performClick()
        compose.waitUntil(10_000) {compose.onAllNodes(hasTestTag("message-general-5") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Original message"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithContentDescription("Cancel reply").performClick()
        compose.onNodeWithTag("composer-reply").assertDoesNotExist()
        compose.onNodeWithTag("composer-input").assertTextEquals("Keep this draft @An")
    }

    @Test fun replyingToAnAlreadyVisibleMessageHighlightsWithoutMovingIt() {
        chat()
        compose.onNode(hasContentDescription("Message actions") and hasAnyAncestor(hasTestTag("message-general-8")),useUnmergedTree=true).performClick()
        compose.onNodeWithText("Reply",substring=false).performClick()
        compose.waitForIdle()
        val original=compose.onNodeWithTag("message-general-8",useUnmergedTree=true)
        val before=original.fetchSemanticsNode().boundsInRoot
        val header=compose.onNodeWithTag("channel-header").fetchSemanticsNode().boundsInRoot
        val composer=compose.onNodeWithTag("composer-reply").fetchSemanticsNode().boundsInRoot
        assertTrue("The original must be fully visible before tapping the reply",before.top>=header.bottom && before.bottom<=composer.top)
        compose.onNodeWithTag("composer-reply").performClick()
        compose.waitUntil(10_000) {compose.onAllNodes(hasTestTag("message-general-8") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Original message"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        assertEquals("Visible replies should not recenter",before.top,original.fetchSemanticsNode().boundsInRoot.top,1f)
    }

    @Test fun offscreenReplyStartsMovingTowardTheOriginalWithoutAnInstantJump() {
        chat()
        compose.onNodeWithTag("message-list").performScrollToNode(hasTestTag("message-general-0"))
        compose.onNode(hasContentDescription("Message actions") and hasAnyAncestor(hasTestTag("message-general-0")),useUnmergedTree=true).performClick()
        compose.onNodeWithText("Reply",substring=false).performClick()
        compose.onNodeWithText("Latest messages").performClick()
        compose.waitForIdle()
        fun scrollPosition()=compose.onNodeWithTag("message-list").fetchSemanticsNode().config[SemanticsProperties.VerticalScrollAxisRange].value()
        val before=scrollPosition()
        assertTrue("Start at the latest messages",before<1f)
        val newest=compose.onNodeWithTag("message-general-9",useUnmergedTree=true)
        val initialTop=newest.fetchSemanticsNode().boundsInRoot.top
        val viewportHeight=compose.onNodeWithTag("message-list").fetchSemanticsNode().boundsInRoot.height
        compose.mainClock.autoAdvance=false
        try {
            compose.onNodeWithTag("composer-reply").performClick()
            compose.waitUntil(10_000) {
                compose.mainClock.advanceTimeByFrame()
                scrollPosition()>before+0.01f
            }
            newest.assertIsDisplayed()
            val movement=newest.fetchSemanticsNode().boundsInRoot.top-initialTop
            assertTrue("Older replies should start with a small downward movement of the visible content; got $movement px",movement>0f && movement<viewportHeight/4)
        } finally {compose.mainClock.autoAdvance=true}
        compose.waitUntil(10_000) {compose.onAllNodes(hasTestTag("message-general-0") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Original message"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithTag("message-general-0",useUnmergedTree=true).assertIsDisplayed()
    }

    @Test fun oldReplyLoadsItsContextAndReturnsToLatestWithoutLosingDrafts() {
        chat()
        compose.onNodeWithTag("composer-input").performTextInput("Still here")
        compose.onNodeWithTag("message-list").performScrollToNode(hasTestTag("reply-preview-general-0"))
        compose.onNodeWithTag("reply-preview-general-0",useUnmergedTree=true).performClick()
        compose.waitUntil(10_000) {compose.onAllNodes(hasTestTag("message-demo-archived-original") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Original message"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithText("An older hello from this conversation.").assertIsDisplayed()
        compose.onNodeWithTag("composer-input").assertTextEquals("Still here")
        compose.onNodeWithText("Latest messages").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithText("o que vocês estão construindo hoje?").fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithText("o que vocês estão construindo hoje?").assertIsDisplayed()
        compose.onNodeWithTag("composer-input").assertTextEquals("Still here")
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
        assertEquals(sendCenter,compose.onNodeWithContentDescription("Emoji and stickers").fetchSemanticsNode().boundsInRoot.center.y,2f)
        assertEquals(sendCenter,compose.onNodeWithContentDescription("GIF picker").fetchSemanticsNode().boundsInRoot.center.y,2f)
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
        compose.onNodeWithTag("message-list").performScrollToKey("media-4")
        val frame = compose.onNodeWithTag("audio-attachment-demo-audio", useUnmergedTree = true).fetchSemanticsNode().size
        compose.onNodeWithTag("attachment-demo-audio", useUnmergedTree = true).assertDoesNotExist()
        compose.waitUntil(15_000) { compose.onAllNodes(hasTestTag("waveform-demo://papo/me/attachment/demo-audio") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Waveform ready"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithContentDescription("Play voice-note.m4a").performClick()
        compose.waitUntil(15_000) { compose.onAllNodes(hasTestTag("player-demo://papo/me/attachment/demo-audio") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("audio-controls", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithContentDescription("Audio position").assertExists()
        val loaded = compose.onNodeWithTag("audio-attachment-demo-audio", useUnmergedTree = true).fetchSemanticsNode().size
        assertEquals(frame.height, loaded.height)
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
        var previous=android.content.pm.ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
        activity.onActivity {previous=it.requestedOrientation}
        compose.onNodeWithTag("message-list").performScrollToNode(hasContentDescription("Play clip.mp4"))
        val frame = compose.onNodeWithTag("attachment-demo-video", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        compose.onNodeWithContentDescription("Play clip.mp4").performClick()
        val player = "player-demo://papo/me/attachment/demo-video"
        compose.waitUntil(15_000) { compose.onAllNodes(hasTestTag(player) and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        val playing = compose.onNodeWithTag("attachment-demo-video", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        assertEquals(frame.height, playing.height, 1f)
        compose.onNodeWithTag("media-viewer").assertDoesNotExist()
        activity.onActivity {assertEquals("Inline playback respects the normal rotation policy",previous,it.requestedOrientation)}
        var originalPlayer:androidx.media3.common.Player?=null
        var originalView:androidx.media3.ui.PlayerView?=null
        androidx.test.espresso.Espresso.onView(androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(androidx.media3.ui.PlayerView::class.java)).check {view,_ ->
            val native=view as androidx.media3.ui.PlayerView
            originalView=native
            originalPlayer=native.player
            originalPlayer!!.pause();originalPlayer!!.seekTo(300)
            native.showController()
        }
        val fullscreenLabel=InstrumentationRegistry.getInstrumentation().targetContext.getString(androidx.media3.ui.R.string.exo_controls_fullscreen_enter_description)
        androidx.test.espresso.Espresso.onView(org.hamcrest.Matchers.allOf(androidx.test.espresso.matcher.ViewMatchers.withContentDescription(fullscreenLabel),androidx.test.espresso.matcher.ViewMatchers.isDisplayed())).perform(androidx.test.espresso.action.ViewActions.click())
        try {
            compose.waitUntil(10_000) {runCatching {compose.onNodeWithTag("media-viewer").assertIsDisplayed();true}.getOrDefault(false)}
        } catch(e:Exception) {
            android.util.Log.e("PapoViewerTest",compose.onAllNodesWithTag("media-viewer").fetchSemanticsNodes().joinToString {"size=${it.size}, bounds=${it.boundsInRoot}, ${it.config}"})
            InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()?.let {bitmap -> InstrumentationRegistry.getInstrumentation().targetContext.cacheDir.resolve("video-viewer-failure.png").outputStream().use {bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}}
            throw e
        }
        compose.onNodeWithTag("media-viewer").assertIsDisplayed()
        compose.onNodeWithContentDescription("Download media").assertIsDisplayed()
        activity.onActivity {assertEquals(android.content.pm.ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR,it.requestedOrientation)}
        try {
            activity.onActivity {it.requestedOrientation=android.content.pm.ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE}
            compose.waitUntil(10_000) {compose.onAllNodesWithTag("media-viewer").fetchSemanticsNodes().any {it.boundsInRoot.width>it.boundsInRoot.height}}
            // Window dimensions update before Android's rotation animation
            // finishes. Native touch coordinates are usable once it settles.
            InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(750,5000)
            compose.onNode(hasTestTag(player) and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Ready"),useUnmergedTree=true).assertIsDisplayed()
            androidx.test.espresso.Espresso.onView(androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(androidx.media3.ui.PlayerView::class.java)).check {view,_ ->
                assertTrue("Fullscreen moves the original PlayerView and its controls",view===originalView)
                assertTrue("Fullscreen keeps the existing player",(view as androidx.media3.ui.PlayerView).player===originalPlayer)
                assertEquals(300L,originalPlayer!!.currentPosition)
                assertTrue("Fullscreen keeps the paused state",!originalPlayer!!.playWhenReady)
            }
            pinchViewer()
            val exitLabel=InstrumentationRegistry.getInstrumentation().targetContext.getString(androidx.media3.ui.R.string.exo_controls_fullscreen_exit_description)
            androidx.test.espresso.Espresso.onView(org.hamcrest.Matchers.allOf(androidx.test.espresso.matcher.ViewMatchers.withContentDescription(exitLabel),androidx.test.espresso.matcher.ViewMatchers.isDisplayed())).perform(androidx.test.espresso.action.ViewActions.click())
            compose.waitUntil(10_000) {compose.onAllNodesWithTag("media-viewer").fetchSemanticsNodes().isEmpty()}
            compose.onNodeWithTag("media-viewer").assertDoesNotExist()
            activity.onActivity {assertEquals("Closing restores the normal orientation policy",previous,it.requestedOrientation)}
            androidx.test.espresso.Espresso.onView(androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(androidx.media3.ui.PlayerView::class.java)).check {view,_ ->
                assertTrue("Returning inline restores the original PlayerView and controls",view===originalView)
                assertTrue("Returning inline keeps the existing player",(view as androidx.media3.ui.PlayerView).player===originalPlayer)
            }
        } finally {activity.onActivity {it.requestedOrientation=previous};InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(750,5000)}
    }

    @Test fun videoPausesOffscreenReleasesFarAwayAndRestoresItsPosition() {
        media()
        compose.onNodeWithTag("message-list").performScrollToKey("media-3")
        compose.onNodeWithContentDescription("Play clip.mp4").performClick()
        val tag="player-demo://papo/me/attachment/demo-video"
        compose.waitUntil(15_000) {compose.onAllNodes(hasTestTag(tag) and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Ready"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        var original:androidx.media3.common.Player?=null
        androidx.test.espresso.Espresso.onView(androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(androidx.media3.ui.PlayerView::class.java)).check {view,_ ->
            original=(view as androidx.media3.ui.PlayerView).player
            original!!.seekTo(300);original!!.play()
        }
        compose.onNodeWithTag("message-list").performScrollToKey("media-6")
        fun onMain(condition:()->Boolean):Boolean {var result=false;InstrumentationRegistry.getInstrumentation().runOnMainSync {result=condition()};return result}
        compose.waitUntil(5000) {onMain {!original!!.playWhenReady}}
        assertTrue("Nearby offscreen playback stays allocated",onMain {original!!.playbackState!=androidx.media3.common.Player.STATE_IDLE})
        var position=0L
        InstrumentationRegistry.getInstrumentation().runOnMainSync {position=original!!.currentPosition}
        compose.onNodeWithTag("message-list").performScrollToKey("media-11")
        compose.waitUntil(10_000) {onMain {original!!.playbackState==androidx.media3.common.Player.STATE_IDLE}}
        compose.onNodeWithTag("message-list").performScrollToKey("media-3")
        compose.waitUntil(15_000) {compose.onAllNodes(hasTestTag(tag) and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Ready"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        androidx.test.espresso.Espresso.onView(androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(androidx.media3.ui.PlayerView::class.java)).check {view,_ ->
            val restored=(view as androidx.media3.ui.PlayerView).player!!
            assertTrue("Far-away media recreates its released player",restored!==original)
            assertTrue("The recreated player retains its position",restored.currentPosition>=position)
        }
    }

    @Test fun animatedStickersAndGifsStopOffscreenWithoutStoppingOtherOccurrences() {
        chat()
        fun send(text:String) {
            compose.onNodeWithTag("composer-input").performTextInput(text);compose.onNodeWithContentDescription("Send message").performClick()
            compose.waitUntil(10_000) {compose.onNodeWithTag("composer-input").fetchSemanticsNode().config[SemanticsProperties.InputText].text.isEmpty()}
        }
        send(":papo_wave:")
        fun probes()=compose.onAllNodes(SemanticsMatcher.keyIsDefined(MediaAnimationRunning) and hasAnyAncestor(hasTestTag("message-sticker-demo-wave")),useUnmergedTree=true).fetchSemanticsNodes().map {it.config[MediaAnimationRunning]}
        compose.waitUntil(10_000) {probes().any {it()}}
        val first=probes().single()
        send((1..23).joinToString("\n") {"A little space $it"})
        send(":papo_wave:")
        compose.waitUntil(10_000) {probes().any {it()}}
        compose.waitUntil(5000) {!first()}
        assertTrue("The visible copy of the same animated sticker keeps running",probes().any {it()})
        compose.onNodeWithContentDescription("Back to conversations").performClick()
        compose.onNodeWithText("mídia").performClick()
        compose.waitUntil(10_000) {compose.onAllNodesWithTag("message-list").fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithTag("message-list").performScrollToKey("media-2")
        fun gifProbe()=compose.onAllNodes(SemanticsMatcher.keyIsDefined(MediaAnimationRunning) and hasAnyAncestor(hasTestTag("attachment-demo-gif")),useUnmergedTree=true).fetchSemanticsNodes().firstOrNull()?.config?.get(MediaAnimationRunning)
        compose.waitUntil(10_000) {gifProbe()?.invoke()==true}
        val gif=gifProbe()!!
        compose.onNodeWithTag("message-list").performScrollToKey("media-7")
        compose.waitUntil(5000) {!gif()}
    }

    private fun pinchViewer() {
        compose.onNodeWithTag("viewer-content",useUnmergedTree=true).performTouchInput {
            down(0,center.copy(x=center.x-30f));down(1,center.copy(x=center.x+30f))
            moveTo(0,center.copy(x=center.x-90f));moveTo(1,center.copy(x=center.x+90f))
            up(0);up(1)
        }
        compose.waitUntil(5000) {compose.onAllNodesWithTag("viewer-content",useUnmergedTree=true).fetchSemanticsNodes().any {it.config[SemanticsProperties.StateDescription]!="Zoom 1.00x"}}
    }

    @Test fun directMediaHidesItsUrlAndHoldingItCopiesEachLinkSeparately() {
        media()
        compose.onNodeWithTag("message-list",useUnmergedTree=true).performScrollToNode(hasTestTag("embed-embed-portrait"))
        val image=compose.onNodeWithContentDescription("Portrait preview")
        compose.waitUntil(10_000) {compose.onAllNodes(hasContentDescription("Portrait preview") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Loaded")).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithText("As dimensões chegam antes dos pixels.",substring=true).assertIsDisplayed()
        compose.onNodeWithText("https://cdn.example.test/portrait.png?sig=fixture",substring=true).assertDoesNotExist()
        image.performTouchInput {longClick()}
        val clipboard=InstrumentationRegistry.getInstrumentation().targetContext.getSystemService(android.content.ClipboardManager::class.java)
        for(url in listOf("https://cdn.example.test/portrait.png?sig=fixture","https://example.test/more")) {
            compose.onNodeWithContentDescription("Copy URL: $url").performScrollTo().performClick()
            assertEquals(url,clipboard.primaryClip?.getItemAt(0)?.text.toString())
        }
        compose.onNodeWithText("Copy text",substring=false).performScrollTo().performClick()
        assertTrue(clipboard.primaryClip?.getItemAt(0)?.text.toString().contains("As dimensões"))
        assertTrue(clipboard.primaryClip?.getItemAt(0)?.text.toString().contains("https://cdn.example.test/portrait.png?sig=fixture"))
    }

    @Test fun directUrlRendersAndOpensBeforeBackendEmbedMetadataArrives() {
        val context=InstrumentationRegistry.getInstrumentation().targetContext
        val bytes=context.assets.open("demo/portrait.png").use {it.readBytes()}
        val socket=java.net.ServerSocket(0)
        val headers=java.util.concurrent.ConcurrentLinkedQueue<String>()
        val executor=java.util.concurrent.Executors.newSingleThreadExecutor()
        executor.submit {
            while(!socket.isClosed) try {
                socket.accept().use {connection ->
                    val reader=connection.getInputStream().bufferedReader()
                    while(true) {val line=reader.readLine() ?: break;if(line.isEmpty()) break;headers.add(line)}
                    connection.getOutputStream().apply {
                        write("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: ${bytes.size}\r\nConnection: close\r\n\r\n".toByteArray());write(bytes);flush()
                    }
                }
            } catch(_:java.io.IOException) {if(socket.isClosed) break}
        }
        try {
            chat()
            val url="http://127.0.0.1:${socket.localPort}/fixture.PNG?sig=exact%2Bvalue"
            compose.onNodeWithTag("composer-input").performTextInput("Keep this caption\n$url")
            compose.onNodeWithContentDescription("Send message").performClick()
            compose.waitUntil(10_000) {compose.onAllNodes(hasContentDescription("fixture.PNG") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Loaded")).fetchSemanticsNodes().isNotEmpty()}
            compose.onNodeWithText("Keep this caption").assertIsDisplayed()
            compose.onNodeWithText(url,substring=true).assertDoesNotExist()
            compose.onNodeWithContentDescription("fixture.PNG").performClick()
            compose.waitUntil(10_000) {compose.onAllNodes(hasTestTag("viewer-image") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Loaded"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
            assertTrue(headers.any {it.contains("sig=exact%2Bvalue")})
            assertTrue(headers.none {it.startsWith("Cookie:",ignoreCase=true)})
            compose.onNodeWithContentDescription("Close image").performClick()
        } finally {socket.close();executor.shutdownNow()}
    }

    @Test fun sharedImageViewerZoomsDownloadsAndDismissesOnlyOutsideItsContent() {
        media()
        compose.onNodeWithTag("message-list",useUnmergedTree=true).performScrollToNode(hasTestTag("embed-embed-portrait"))
        compose.waitUntil(10_000) {compose.onAllNodes(hasContentDescription("Portrait preview") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Loaded")).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithContentDescription("Portrait preview").performClick()
        compose.waitUntil(10_000) {compose.onAllNodes(hasTestTag("viewer-image") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Loaded"),useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        compose.onNodeWithTag("viewer-content").performClick()
        compose.onNodeWithTag("media-viewer").assertIsDisplayed()
        pinchViewer()
        val reset=compose.onNodeWithTag("viewer-content").fetchSemanticsNode().config[androidx.compose.ui.semantics.SemanticsActions.CustomActions].single {it.label=="Reset zoom"}
        compose.runOnIdle {assertTrue(reset.action())}
        val preview=compose.onNodeWithTag("media-viewer").captureToImage().asAndroidBitmap()
        InstrumentationRegistry.getInstrumentation().targetContext.cacheDir.resolve("media-viewer-verified.png").outputStream().use {preview.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
        val context=InstrumentationRegistry.getInstrumentation().targetContext
        val downloads=android.provider.MediaStore.Downloads.EXTERNAL_CONTENT_URI
        fun ids():Set<Long> = context.contentResolver.query(downloads,arrayOf(android.provider.MediaStore.MediaColumns._ID),"${android.provider.MediaStore.MediaColumns.DISPLAY_NAME} LIKE ?",arrayOf("Portrait preview%"),null)!!.use {cursor ->buildSet {while(cursor.moveToNext()) add(cursor.getLong(0))}}
        val before=ids()
        try {
            compose.onNodeWithContentDescription("Download media").performClick()
            compose.waitUntil(10_000) {compose.onAllNodesWithContentDescription("Downloaded").fetchSemanticsNodes().isNotEmpty()}
            assertTrue(ids().size>before.size)
        } finally {(ids()-before).forEach {id ->context.contentResolver.delete(android.content.ContentUris.withAppendedId(downloads,id),null,null)}}
        val content=compose.onNodeWithTag("viewer-content").fetchSemanticsNode().boundsInRoot
        val backdrop=compose.onNodeWithTag("viewer-backdrop").fetchSemanticsNode().boundsInRoot
        compose.onNodeWithTag("viewer-backdrop").performTouchInput {click(androidx.compose.ui.geometry.Offset(8f,content.top-backdrop.top-12f))}
        compose.onNodeWithTag("media-viewer").assertDoesNotExist()
    }

}
