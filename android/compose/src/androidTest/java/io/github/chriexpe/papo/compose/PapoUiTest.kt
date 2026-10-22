package io.github.chriexpe.papo.compose

import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.launch
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Runs the real Activity, UniFFI library and shared Rust reducer on a device. */
@RunWith(AndroidJUnit4::class)
class PapoUiTest {
    @get:Rule val compose = createEmptyComposeRule()
    private lateinit var activity: ActivityScenario<MainActivity>

    private lateinit var savedClient: Map<String, *>
    private lateinit var savedAppearance: Map<String, *>
    private lateinit var savedExpressions: Map<String, *>
    private lateinit var savedAttachments: Map<String, *>

    private val preferenceBackup
        get() =
            android.util.AtomicFile(
                InstrumentationRegistry.getInstrumentation()
                    .targetContext
                    .filesDir
                    .resolve("ui-test-preferences.json")
            )

    private fun restore(name: String, values: Map<String, *>) {
        val editor =
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .getSharedPreferences(name, 0)
                .edit()
                .clear()
        values.forEach { (key, value) ->
            when (value) {
                is String -> editor.putString(key, value)
                is Boolean -> editor.putBoolean(key, value)
            }
        }
        check(editor.commit())
    }

    private fun recoverPreferences() {
        val bytes =
            try {
                preferenceBackup.readFully()
            } catch (_: java.io.FileNotFoundException) {
                return
            }
        val snapshot = org.json.JSONObject(bytes.toString(Charsets.UTF_8))
        snapshot.keys().forEach { name ->
            val values = snapshot.getJSONObject(name)
            restore(name, values.keys().asSequence().associateWith { values.get(it) })
        }
        preferenceBackup.delete()
    }

    @Before
    fun launch() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        // A killed instrumentation process cannot run @After. Recover its
        // durable snapshot before taking another backup or clearing anything.
        recoverPreferences()
        savedExpressions = context.getSharedPreferences("compose-expressions", 0).all.toMap()
        savedAttachments = context.getSharedPreferences("compose-attachments", 0).all.toMap()
        savedClient = context.getSharedPreferences("compose-client", 0).all.toMap()
        savedAppearance = context.getSharedPreferences("compose-appearance", 0).all.toMap()
        val snapshot =
            org.json
                .JSONObject()
                .put("compose-client", org.json.JSONObject(savedClient))
                .put("compose-appearance", org.json.JSONObject(savedAppearance))
                .put("compose-expressions", org.json.JSONObject(savedExpressions))
                .put("compose-attachments", org.json.JSONObject(savedAttachments))
        val stream = preferenceBackup.startWrite()
        try {
            stream.write(snapshot.toString().toByteArray())
            preferenceBackup.finishWrite(stream)
        } catch (error: Throwable) {
            preferenceBackup.failWrite(stream)
            throw error
        }
        listOf("compose-client", "compose-appearance", "compose-expressions", "compose-attachments")
            .forEach { restore(it, emptyMap<String, Any>()) }
        activity = ActivityScenario.launch(MainActivity::class.java)
    }

    @After
    fun close() {
        activity.close()
        restore("compose-expressions", savedExpressions)
        restore("compose-attachments", savedAttachments)
        restore("compose-client", savedClient)
        restore("compose-appearance", savedAppearance)
        preferenceBackup.delete()
    }

    private fun demo() {
        compose.onNodeWithText("Explore demo").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("server-header").fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun chat() {
        demo()
        compose.onNodeWithText("geral").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(hasTestTag("channel-header") and hasText("geral"))
                .fetchSemanticsNodes()
                .isNotEmpty() &&
                compose.onAllNodesWithTag("message-list").fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun systemBack(gesture: Boolean = false) {
        compose.waitForIdle()
        InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(500, 5000)
        val metrics =
            InstrumentationRegistry.getInstrumentation().targetContext.resources.displayMetrics
        val command =
            if (gesture)
                "input swipe 1 ${metrics.heightPixels/2} ${metrics.widthPixels/3} ${metrics.heightPixels/2} 350"
            else "input keyevent KEYCODE_BACK"
        val descriptor =
            InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand(command)
        android.os.ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { it.readBytes() }
        InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(500, 5000)
        compose.waitForIdle()
    }

    @Test
    fun systemBackClosesTheViewerThenTheConversation() {
        media()
        compose.onNodeWithTag("message-list").performScrollToKey("media-0")
        compose
            .onNodeWithContentDescription("Photo: landscape.png", useUnmergedTree = true)
            .performClick()
        compose.onNodeWithTag("media-viewer").assertIsDisplayed()
        systemBack()
        compose.onNodeWithTag("media-viewer").assertDoesNotExist()
        compose.onNodeWithTag("message-list").assertIsDisplayed()
        systemBack()
        compose.onNodeWithTag("server-header").assertIsDisplayed()
    }

    @Test
    fun backGestureClosesTheViewerThenTheConversation() {
        media()
        compose.onNodeWithTag("message-list").performScrollToKey("media-0")
        compose
            .onNodeWithContentDescription("Photo: landscape.png", useUnmergedTree = true)
            .performClick()
        compose.onNodeWithTag("media-viewer").assertIsDisplayed()
        systemBack(gesture = true)
        compose.onNodeWithTag("media-viewer").assertDoesNotExist()
        compose.onNodeWithTag("message-list").assertIsDisplayed()
        systemBack(gesture = true)
        compose.onNodeWithTag("server-header").assertIsDisplayed()
    }

    @Test
    fun activityBackDispatchClosesOnlyTheActiveViewer() {
        media()
        compose.onNodeWithTag("message-list").performScrollToKey("media-0")
        compose
            .onNodeWithContentDescription("Photo: landscape.png", useUnmergedTree = true)
            .performClick()
        compose.onNodeWithTag("media-viewer").assertIsDisplayed()
        activity.onActivity { it.onBackPressedDispatcher.onBackPressed() }
        compose.onNodeWithTag("media-viewer").assertDoesNotExist()
        compose.onNodeWithTag("message-list").assertIsDisplayed()
    }

    @Test
    fun systemBackHidesKeyboardBeforeLeavingAndKeepsTheDraft() {
        chat()
        compose.onNodeWithTag("composer-input").performClick().performTextInput("Keep this draft")
        fun keyboardVisible(): Boolean {
            var visible = false
            activity.onActivity {
                visible =
                    androidx.core.view.ViewCompat.getRootWindowInsets(it.window.decorView)
                        ?.isVisible(androidx.core.view.WindowInsetsCompat.Type.ime()) == true
            }
            return visible
        }
        compose.waitUntil(5000) { keyboardVisible() }
        systemBack()
        compose.waitUntil(5000) { !keyboardVisible() }
        compose.onNodeWithTag("message-list").assertIsDisplayed()
        compose.onNodeWithTag("composer-input").assertTextEquals("Keep this draft")
        systemBack()
        compose.onNodeWithTag("server-header").assertIsDisplayed()
        compose.onNodeWithText("geral").performClick()
        compose.onNodeWithTag("composer-input").assertTextEquals("Keep this draft")
    }

    @Test
    fun systemBackStopsRecordingIntoAnUnsentDraft() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.grantRuntimePermission(
            instrumentation.targetContext.packageName,
            android.Manifest.permission.RECORD_AUDIO,
        )
        chat()
        compose.onNodeWithContentDescription("Open gallery").performTouchInput { longClick() }
        compose.onNodeWithText("Record audio").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("Send recording")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Send recording").assertIsEnabled()
        android.os.SystemClock.sleep(1200)
        systemBack()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("Remove Voice note ", substring = true)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("message-list").assertIsDisplayed()
        compose
            .onNodeWithContentDescription("Play Voice note ", substring = true)
            .assertDoesNotExist()
        compose.onNodeWithContentDescription("Remove Voice note ", substring = true).performClick()
    }

    @Test
    fun systemBackReturnsFullscreenVideoToInlineWithoutLeavingChat() {
        media()
        compose.onNodeWithTag("message-list").performScrollToKey("media-3")
        // Keep native controls clear of the floating Latest messages button.
        val pixels =
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .resources
                .displayMetrics
                .density
        compose.onNodeWithTag("message-list").performTouchInput {
            swipe(center, center.copy(y = center.y - 120 * pixels), 300)
        }
        compose.onNodeWithContentDescription("Play clip.mp4").performClick()
        val tag = "player-demo://papo/me/attachment/demo-video"
        compose.waitUntil(15_000) {
            compose
                .onAllNodes(
                    hasTestTag(tag) and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready"),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        val label =
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .getString(androidx.media3.ui.R.string.exo_controls_fullscreen_enter_description)
        androidx.test.espresso.Espresso.onView(
                org.hamcrest.Matchers.allOf(
                    androidx.test.espresso.matcher.ViewMatchers.withContentDescription(label),
                    androidx.test.espresso.matcher.ViewMatchers.isDisplayed(),
                )
            )
            .perform(androidx.test.espresso.action.ViewActions.click())
        compose.onNodeWithTag("media-viewer").assertIsDisplayed()
        systemBack()
        compose.onNodeWithTag("media-viewer").assertDoesNotExist()
        compose.onNodeWithTag("message-list").assertIsDisplayed()
        systemBack()
        compose.onNodeWithTag("server-header").assertIsDisplayed()
    }

    @Test
    fun systemBackLeavesSettingsForChatsBeforeExiting() {
        demo()
        compose.onNodeWithContentDescription("Client settings").performClick()
        systemBack()
        compose.onNodeWithTag("server-header").assertIsDisplayed()
    }

    @Test
    fun systemBackClosesProfilesAndPickersBeforeLeavingChat() {
        chat()
        compose.onNodeWithTag("author-general-9", useUnmergedTree = true).performClick()
        systemBack()
        compose.onNodeWithTag("user-profile").assertDoesNotExist()
        compose.onNodeWithTag("message-list").assertIsDisplayed()
        compose.onNodeWithContentDescription("Emoji and stickers").performClick()
        systemBack()
        compose.onNodeWithTag("emoji-picker").assertDoesNotExist()
        compose.onNodeWithTag("message-list").assertIsDisplayed()
        systemBack()
        compose.onNodeWithTag("server-header").assertIsDisplayed()
    }

    @Test
    fun navigationGroupsChatsAndDirectAndKeepsServersSeparate() {
        demo()
        compose
            .onNode(hasText("Chats") and hasAnyAncestor(hasTestTag("chat-navigation")))
            .assertIsDisplayed()
        compose
            .onNode(hasText("Direct") and hasAnyAncestor(hasTestTag("chat-navigation")))
            .assertIsDisplayed()
        compose
            .onNode(hasText("Servers") and hasAnyAncestor(hasTestTag("servers-navigation")))
            .assertIsDisplayed()
        compose.onNodeWithText("Channels", substring = false).assertDoesNotExist()
        compose.onNodeWithText("People", substring = false).assertDoesNotExist()
        compose.onNodeWithText("Settings", substring = false).assertDoesNotExist()
        compose.onNodeWithText("Direct", substring = false).performClick()
        compose.onNodeWithText("Direct messages").assertIsDisplayed()
        compose.onNodeWithText("Ana", substring = false).performClick()
        compose.onNodeWithTag("message-list").assertIsDisplayed()
        systemBack()
        compose.onNodeWithText("Direct messages").assertIsDisplayed()
        compose.onNodeWithText("Servers", substring = false).performClick()
        compose.onNodeWithText("Your servers").assertIsDisplayed()
        systemBack()
        compose.onNodeWithText("geral", substring = false).assertIsDisplayed()
    }

    @Test
    fun serversOpensGlobalDirectMessagesAndReturnsAfterChatAndProfile() {
        demo()
        compose.onNodeWithText("Servers", substring = false).performClick()
        compose
            .onNode(hasText("Direct messages") and hasAnyAncestor(hasTestTag("global-actions")))
            .assertIsDisplayed()
            .performClick()
        compose.onNodeWithTag("global-direct-messages").assertIsDisplayed()
        compose.onNodeWithText("Ana", substring = false).assertIsDisplayed()
        compose
            .onNode(hasText("Papo") and hasAnyAncestor(hasTestTag("global-direct-list")))
            .assertIsDisplayed()
        compose.onNodeWithTag("avatar-ana", useUnmergedTree = true).performClick()
        compose.onNodeWithTag("user-profile").assertIsDisplayed()
        compose.onNodeWithText("@ana", substring = false).assertIsDisplayed()
        systemBack()
        compose.onNodeWithText("Ana", substring = false).performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("message-list").fetchSemanticsNodes().isNotEmpty()
        }
        systemBack()
        compose.onNodeWithTag("global-direct-messages").assertIsDisplayed()
        systemBack()
        compose.onNodeWithTag("global-actions").assertIsDisplayed()
    }

    @Test
    fun globalNotificationsOpenTheSourceMessageAndConfirmReadThroughCore() {
        demo()
        compose.onNodeWithText("Servers", substring = false).performClick()
        compose
            .onNode(hasText("Notifications") and hasAnyAncestor(hasTestTag("global-actions")))
            .performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithTag("notification-demo-demo-notification")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose
            .onNodeWithTag("notification-demo-demo-notification")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Unread"))
        compose.onNodeWithContentDescription("Mark notification as read").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("notification-demo-demo-notification") and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Read")
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("notification-demo-demo-notification").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("message-list").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("channel-header").assert(hasText("geral"))
        systemBack()
        compose.onNodeWithTag("global-notifications").assertIsDisplayed()
        systemBack()
        compose.onNodeWithTag("global-actions").assertIsDisplayed()
    }

    @Test
    fun globalNotificationsKeepHealthyResultsWhenAnotherAccountFails() {
        demo()
        activity.onActivity { host ->
            val model = androidx.lifecycle.ViewModelProvider(host)[PapoViewModel::class.java]
            val source = model.state.value.active!!
            val unavailable =
                source.copy(
                    url = "https://unavailable.example.test",
                    key = "unavailable",
                    serverName = "Unavailable account",
                )
            host.setContent {
                androidx.compose.material3.MaterialTheme {
                    androidx.compose.runtime.CompositionLocalProvider(
                        LocalHaze provides dev.chrisbanes.haze.HazeState(),
                        LocalGlass provides false,
                    ) {
                        GlobalNotifications(
                            PapoState(listOf(source, unavailable), source.url),
                            model,
                            onBack = {},
                            onOpen = { _, _ -> },
                        )
                    }
                }
            }
        }
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithTag("notification-demo-demo-notification")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("notification-demo-demo-notification").assertIsDisplayed()
        compose.onNodeWithText("Unavailable account", substring = false).assertIsDisplayed()
        compose.onNodeWithText("Retry", substring = false).assertIsDisplayed()
    }

    @Test
    fun globalDirectRowsKeepServersSeparateEvenWithTheSameUserAndChannelIds() {
        val client = io.github.chriexpe.papo.core.NativeClient.demo()
        val source =
            try {
                client.poll()
            } finally {
                client.close()
            }
        val first =
            source.copy(
                url = "https://first.example.test",
                key = "first",
                serverName = "First server",
            )
        val second =
            source.copy(
                url = "https://second.example.test",
                key = "second",
                serverName = "Second server",
            )
        val state = PapoState(listOf(first, second), first.url)
        assertEquals(2, globalDirects(state, "Ana").size)
        assertEquals(second.url, globalDirects(state, "second").single().server.url)
        assertEquals(
            1,
            globalDirects(
                    state.copy(servers = listOf(first, second.copy(screen = "auth", me = ""))),
                    "",
                )
                .size,
        )
        var opened = ""
        activity.onActivity { host ->
            val model = androidx.lifecycle.ViewModelProvider(host)[PapoViewModel::class.java]
            host.setContent {
                androidx.compose.material3.MaterialTheme {
                    androidx.compose.runtime.CompositionLocalProvider(
                        LocalHaze provides dev.chrisbanes.haze.HazeState(),
                        LocalGlass provides false,
                    ) {
                        GlobalDirectMessages(
                            state,
                            model,
                            onBack = {},
                            onProfile = { _, _ -> },
                            onOpen = { server, channel -> opened = "${server.url}/${channel.id}" },
                        )
                    }
                }
            }
        }
        compose.onNodeWithTag("dm-second-dm-ana").performClick()
        compose.runOnIdle { assertEquals("${second.url}/dm-ana", opened) }
    }

    @Test
    fun avatarOpensClientSettingsAndReturnsToItsOriginalDestination() {
        demo()
        compose.onNodeWithText("Direct", substring = false).performClick()
        compose.onNodeWithContentDescription("Client settings").performClick()
        compose.onNodeWithText("Appearance", substring = false).assertIsDisplayed()
        systemBack()
        compose.onNodeWithText("Direct messages").assertIsDisplayed()
        compose.onNodeWithText("Chats", substring = false).performClick()
        compose.onNodeWithText("geral", substring = false).assertIsDisplayed()
    }

    @Test
    fun serverHeaderShowsIconAndPersonalNotificationsWithoutCreatorName() {
        demo()
        compose.onNodeWithText("YOUR SPACE").assertDoesNotExist()
        compose.onNodeWithText("DEMO SPACE").assertDoesNotExist()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("server-icon") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Icon loaded",
                        ),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("server-header").performClick()
        compose.onNodeWithTag("server-info").assertIsDisplayed()
        compose.onNodeWithText("ADDRESS").assertIsDisplayed()
        compose.onNodeWithText("Christian", substring = true).assertDoesNotExist()
        compose.onNodeWithTag("channel-notifications-general").performClick()
        compose.onNodeWithText("Off", substring = false).performClick()
        compose.onNodeWithTag("channel-notifications-general").assert(hasText("Off"))
        systemBack()
        compose.onNodeWithTag("server-info").assertDoesNotExist()
        compose.onNodeWithTag("server-header").assertIsDisplayed()
    }

    @Test
    fun serverGearOpensPermissionScopedSettingsAndSavesTheServer() {
        demo()
        compose.onNodeWithContentDescription("Server settings").performClick()
        compose.onNodeWithTag("server-settings").assertIsDisplayed()
        compose.onNodeWithText("General").assertIsDisplayed()
        compose
            .onNode(
                hasText("Channels", substring = false) and
                    hasAnyAncestor(hasTestTag("server-settings"))
            )
            .assertIsDisplayed()
        compose.onNodeWithText("Roles").assertIsDisplayed()
        compose.onNodeWithText("General").performClick()
        compose.onNodeWithTag("server-name-field").performTextReplacement("Papo Compose")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.onNodeWithText("Save server").performClick()
        compose.waitUntil(5000) {
            compose.onAllNodesWithText("Server saved").fetchSemanticsNodes().isNotEmpty()
        }
        systemBack()
        compose.onNodeWithTag("server-header").assert(hasText("Papo Compose"))
    }

    @Test
    fun serverEditorsSaveChannelAndRoleChanges() {
        demo()
        compose.onNodeWithContentDescription("Server settings").performClick()
        compose
            .onNode(
                hasText("Channels", substring = false) and
                    hasAnyAncestor(hasTestTag("server-settings"))
            )
            .performClick()
        compose
            .onNode(
                hasText("geral", substring = false) and
                    hasAnyAncestor(hasTestTag("server-settings"))
            )
            .performClick()
        compose
            .onNode(hasText("Channel name") and hasSetTextAction())
            .performTextReplacement("geral-updated")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.onNodeWithText("Save channel").performClick()
        compose.waitUntil(5000) {
            compose.onAllNodesWithText("Save channel").fetchSemanticsNodes().isEmpty()
        }
        systemBack()
        compose
            .onNode(
                hasText("Roles", substring = false) and
                    hasAnyAncestor(hasTestTag("server-settings"))
            )
            .performClick()
        compose.onNodeWithText("Designers", substring = false).performClick()
        compose
            .onNode(hasText("Role name") and hasSetTextAction())
            .performTextReplacement("Creators")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.onNodeWithText("Save role").performScrollTo().performClick()
        compose.waitUntil(5000) {
            compose.onAllNodesWithText("Save role").fetchSemanticsNodes().isEmpty()
        }
        compose.onNodeWithText("Creators", substring = false).assertIsDisplayed()
        systemBack()
        systemBack()
        compose.onNodeWithText("geral-updated", substring = false).assertIsDisplayed()
    }

    @Test
    fun administrationSearchFindsIndividualSettingsAndProtectsUnsavedChanges() {
        demo()
        compose.onNodeWithContentDescription("Server settings").performClick()
        compose.onNodeWithTag("server-settings-search").performTextInput("server name")
        compose.onNodeWithText("Server name", substring = false).performClick()
        compose.onNodeWithTag("server-name-field").performTextReplacement("Unsaved server")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        systemBack()
        compose.onNodeWithText("Discard changes?").assertIsDisplayed()
        compose.onNodeWithText("Keep editing").performClick()
        compose
            .onNodeWithTag("server-name-field")
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.InputText,
                    AnnotatedString("Unsaved server"),
                )
            )
        systemBack()
        compose.onNodeWithText("Discard", substring = false).performClick()
        compose.onNodeWithTag("server-settings-search").assertTextEquals("server name")
    }

    @Test
    fun administrationSearchScrollsToTheExactRolePermission() {
        demo()
        compose.onNodeWithContentDescription("Server settings").performClick()
        compose
            .onNodeWithTag("server-settings-search")
            .performTextInput("Designers send attachments")
        compose.onNodeWithText("Designers: Send attachments", substring = false).performClick()
        compose
            .onNodeWithContentDescription("Send attachments", useUnmergedTree = true)
            .assertIsDisplayed()
        compose.onNodeWithTag("admin-role-editor").captureToImage().asAndroidBitmap().let { bitmap
            ->
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .cacheDir
                .resolve("administration-role-editor.png")
                .outputStream()
                .use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
            bitmap.recycle()
        }
    }

    @Test
    fun administrationCreatesChannelsAndAuditFiltersTheSavedAction() {
        demo()
        compose.onNodeWithContentDescription("Server settings").performClick()
        compose.onNodeWithText("Channels", substring = false).performClick()
        compose.onNodeWithText("Create channel", substring = false).performClick()
        compose.onNodeWithTag("admin-channel-name").performTextReplacement("release-room")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.onNodeWithTag("save-channel").performScrollTo().performClick()
        compose.waitUntil(5000) {
            compose.onAllNodesWithTag("admin-channels").fetchSemanticsNodes().isNotEmpty()
        }
        compose
            .onNodeWithTag("admin-channels")
            .performScrollToNode(hasText("release-room", substring = false))
        compose
            .onNode(
                hasText("release-room", substring = false) and
                    hasAnyAncestor(hasTestTag("admin-channels"))
            )
            .assertIsDisplayed()
        systemBack()
        compose
            .onNodeWithTag("server-settings-overview")
            .performScrollToNode(hasText("Audit", substring = false))
        compose.onNodeWithText("Audit", substring = false).performClick()
        compose.waitUntil(5000) {
            compose
                .onAllNodes(
                    hasText("release-room", substring = true) and
                        hasAnyAncestor(hasTestTag("admin-audit"))
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("audit-person-search").performTextInput("no-such-person")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.waitUntil(5000) {
            compose
                .onAllNodesWithText("No matching activity in these entries.")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Clear person filter").performClick()
        compose.waitUntil(5000) {
            compose
                .onAllNodes(
                    hasText("release-room", substring = true) and
                        hasAnyAncestor(hasTestTag("admin-audit"))
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
    }

    @Test
    fun sharedMediaEditorPreviewsTheCropAndKeepsItAfterARejectedSave() {
        demo()
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val file = java.io.File.createTempFile("editor-ui-", ".png", context.cacheDir)
        val bitmap =
            android.graphics.Bitmap.createBitmap(800, 400, android.graphics.Bitmap.Config.ARGB_8888)
        val canvas = android.graphics.Canvas(bitmap)
        canvas.drawColor(android.graphics.Color.rgb(29, 114, 202))
        canvas.drawRect(
            400f,
            0f,
            800f,
            400f,
            android.graphics.Paint().apply { color = android.graphics.Color.rgb(255, 173, 40) },
        )
        file.outputStream().use {
            bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
        }
        bitmap.recycle()
        val attempts = java.util.concurrent.atomic.AtomicInteger()
        val result =
            java.util.concurrent.atomic.AtomicReference<
                io.github.chriexpe.papo.core.PreparedMediaImage
            >()
        val client = io.github.chriexpe.papo.core.NativeClient.demo()
        val server =
            try {
                client.poll()
            } finally {
                client.shutdown()
                client.close()
            }
        try {
            activity.onActivity { host ->
                val model = androidx.lifecycle.ViewModelProvider(host)[PapoViewModel::class.java]
                host.setContent {
                    androidx.compose.material3.MaterialTheme {
                        val visible =
                            androidx.compose.runtime.remember {
                                androidx.compose.runtime.mutableStateOf(true)
                            }
                        if (visible.value)
                            MediaEditor(
                                MediaEditorSource(file.absolutePath, "rocket.png", 800, 400),
                                io.github.chriexpe.papo.core.MediaEditPurpose.STICKER,
                                server,
                                model,
                                onDismiss = { visible.value = false },
                            ) { output, _ ->
                                if (attempts.incrementAndGet() == 1)
                                    error("Upload rejected; try again")
                                result.set(output)
                            }
                    }
                }
            }
            compose.onNodeWithTag("media-crop-area").assertIsDisplayed()
            compose.onNodeWithTag("crop-zoom").performSemanticsAction(
                androidx.compose.ui.semantics.SemanticsActions.SetProgress
            ) {
                it(2f)
            }
            compose.onNodeWithTag("media-editor").captureToImage().asAndroidBitmap().let { preview
                ->
                context.cacheDir.resolve("administration-media-editor.png").outputStream().use {
                    preview.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
                }
                preview.recycle()
            }
            compose.onNodeWithTag("save-media-edit").performClick()
            compose.waitUntil(10000) {
                compose
                    .onAllNodesWithText("Upload rejected; try again")
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            compose.onNodeWithTag("media-editor").assertIsDisplayed()
            compose.onNodeWithTag("save-media-edit").performClick()
            compose.waitUntil(10000) { result.get() != null }
            compose.onNodeWithTag("media-editor").assertDoesNotExist()
            assertEquals(200u, result.get().width)
            assertEquals(200u, result.get().height)
            assertEquals(2, attempts.get())
            assertTrue(file.exists())
        } finally {
            file.delete()
        }
    }

    @Test
    fun normalMemberCanOpenServerInformationWithoutAnAdminGear() {
        val client = io.github.chriexpe.papo.core.NativeClient.demo()
        val server =
            try {
                client.poll().let {
                    it.copy(
                        serverIconRevision = "",
                        capabilities =
                            it.capabilities.copy(
                                manageServer = false,
                                manageChannels = false,
                                manageRoles = false,
                                banMembers = false,
                            ),
                    )
                }
            } finally {
                client.shutdown()
                client.close()
            }
        activity.onActivity { host ->
            val model = androidx.lifecycle.ViewModelProvider(host)[PapoViewModel::class.java]
            host.setContentForMemberHeader(server, model)
        }
        compose.onNodeWithContentDescription("Server settings").assertDoesNotExist()
        compose.onNodeWithTag("server-header").performClick()
        compose.onNodeWithTag("server-info").assertIsDisplayed()
        compose.onNodeWithContentDescription("Server settings").assertDoesNotExist()
    }

    @Test
    fun searchCompletesFiltersFindsMessagesAndReturnsToTheSameQuery() {
        demo()
        compose.onNodeWithTag("global-search").performClick().performTextInput("d")
        compose.waitUntil(5000) {
            compose.onAllNodesWithText("de:  author").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("de:  author").performClick()
        compose.onNodeWithTag("global-search").performTextInput("an")
        compose.waitUntil(5000) {
            compose.onAllNodesWithText("de: @ana  Ana").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("de: @ana  Ana").performClick()
        compose.onNodeWithTag("global-search").assertTextEquals("de:@ana ").performTextInput("em:")
        compose.waitUntil(5000) {
            compose.onAllNodesWithText("em: #geral").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("em: #geral").performClick()
        val query = "de:@ana em:\"#geral\" "
        compose.onNodeWithTag("global-search").assertTextEquals(query)
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("search-hit-general-9").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("search-hit-general-9").performClick()
        compose.onNodeWithTag("message-list").assertIsDisplayed()
        compose.onNodeWithTag("message-general-9").assertIsDisplayed()
        systemBack()
        compose.onNodeWithTag("global-search").assertTextEquals(query)
        systemBack()
        compose
            .onNodeWithTag("global-search")
            .assert(
                SemanticsMatcher.expectValue(SemanticsProperties.InputText, AnnotatedString(""))
            )
    }

    @Test
    fun searchFindsPeopleChannelsAndMessagesOutsideTheCurrentChannel() {
        demo()
        compose.onNodeWithTag("global-search").performTextInput("ana")
        compose.onNodeWithText("@ana", substring = false).assertIsDisplayed()
        compose.onNodeWithTag("global-search").performTextReplacement("design")
        compose
            .onNode(hasText("design", substring = false) and !hasSetTextAction())
            .assertIsDisplayed()
        compose.onNodeWithTag("global-search").performTextReplacement("tem:anexo")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    SemanticsMatcher("Search result") {
                        it.config
                            .getOrNull(SemanticsProperties.TestTag)
                            ?.startsWith("search-hit-") == true
                    }
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose
            .onNodeWithTag("conversation-results")
            .performScrollToNode(hasTestTag("search-hit-media-0"))
        compose
            .onNode(
                hasText("#mídia", substring = false) and
                    hasAnyAncestor(hasTestTag("search-hit-media-0")),
                useUnmergedTree = true,
            )
            .performScrollTo()
            .assertIsDisplayed()
            .performClick()
        compose.waitUntil(5000) {
            compose
                .onAllNodes(hasTestTag("channel-header") and hasText("mídia"))
                .fetchSemanticsNodes()
                .isNotEmpty() ||
                compose.onAllNodesWithTag("user-profile").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("user-profile").assertDoesNotExist()
        compose.onNode(hasTestTag("channel-header") and hasText("mídia")).assertIsDisplayed()
        compose.onNodeWithTag("message-media-0").assertIsDisplayed()
    }

    @Test
    fun chatProfilesShowImagesAndHeldAuthorsInsertMentions() {
        chat()
        compose.onNodeWithTag("author-general-9", useUnmergedTree = true).performClick()
        compose.onNodeWithTag("user-profile").assertIsDisplayed()
        compose.onNodeWithText("@ana", substring = false).assertIsDisplayed()
        compose.onNodeWithText("Designers").assertIsDisplayed()
        compose.onNodeWithText("Desenhando ideias").assertIsDisplayed()
        compose
            .onNodeWithText("Design, café e boas conversas.", substring = true)
            .assertIsDisplayed()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("avatar-ana") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Avatar loaded",
                        ),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()?.let { image ->
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .cacheDir
                .resolve("profile-preview.png")
                .outputStream()
                .use { image.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
        }
        compose.onNodeWithText("Copy username").performClick()
        compose.onNodeWithText("Copied username").assertIsDisplayed()
        val clipboard =
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .getSystemService(android.content.ClipboardManager::class.java)
        assertEquals("ana", clipboard.primaryClip?.getItemAt(0)?.text.toString())
        compose.onNodeWithContentDescription("Close profile").performClick()
        compose.onNodeWithTag("author-general-9", useUnmergedTree = true).performTouchInput {
            longClick()
        }
        compose.onNodeWithTag("user-profile").assertDoesNotExist()
        compose
            .onNode(hasSetTextAction())
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.InputText,
                    AnnotatedString("@Ana "),
                )
            )
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.onNodeWithTag("message-list").performScrollToKey("general-9")
        compose
            .onNode(
                hasTestTag("avatar-ana") and hasAnyAncestor(hasTestTag("message-general-9")),
                useUnmergedTree = true,
            )
            .performTouchInput { longClick() }
        compose
            .onNode(hasSetTextAction())
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.InputText,
                    AnnotatedString("@Ana @Ana "),
                )
            )
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.onNodeWithTag("message-list").performScrollToKey("general-9")
        compose
            .onNode(
                hasTestTag("avatar-ana") and hasAnyAncestor(hasTestTag("message-general-9")),
                useUnmergedTree = true,
            )
            .performClick()
        compose.onNodeWithTag("user-profile").assertIsDisplayed()
    }

    @Test
    fun peopleOpenProfilesBeforeStartingDirectMessages() {
        demo()
        compose.onNodeWithText("Direct", substring = false).performClick()
        compose.onNodeWithTag("avatar-ana", useUnmergedTree = true).performClick()
        compose.onNodeWithTag("user-profile").assertIsDisplayed()
        compose.onNodeWithContentDescription("Close profile").performClick()
        compose.onNodeWithTag("global-search").performTextInput("ana")
        compose.onNode(hasText("Ana", substring = false) and !hasSetTextAction()).performClick()
        compose.onNodeWithTag("user-profile").assertIsDisplayed()
        compose.onNodeWithText("Message @ana").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(hasTestTag("channel-header") and hasText("Ana"))
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("user-profile").assertDoesNotExist()
    }

    @Test
    fun wormholeAuthorsUseExternalAvatarsAndAppProfiles() {
        demo()
        compose.onNodeWithText("design", substring = false).performClick()
        compose.onNodeWithTag("author-design-0", useUnmergedTree = true).assertTextEquals("Ari")
        compose.onNodeWithText("APP", substring = false).assertIsDisplayed()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("external-avatar-Wormhole-remote-ari") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Avatar loaded",
                        ),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("author-design-0", useUnmergedTree = true).performClick()
        compose.onNodeWithTag("external-profile").assertIsDisplayed()
        compose.onNodeWithTag("user-profile").assertDoesNotExist()
        compose.onNodeWithText("Via Wormhole").assertIsDisplayed()
        compose.onNodeWithText("Open external profile").assertIsDisplayed()
        compose.onNodeWithText("Copy username").assertDoesNotExist()
        compose.onNodeWithText("Copy name").performClick()
        compose.onNodeWithText("Copied name").assertIsDisplayed()
        compose.onNodeWithContentDescription("Close profile").performClick()
        compose.onNodeWithTag("author-design-0", useUnmergedTree = true).performTouchInput {
            longClick()
        }
        compose.onNodeWithTag("external-profile").assertIsDisplayed()
        compose
            .onNode(hasSetTextAction())
            .assert(
                SemanticsMatcher.expectValue(SemanticsProperties.InputText, AnnotatedString(""))
            )
    }

    @Test
    fun serverStickersInsertAnimateAndAppearInSharedReactionPicker() {
        chat()
        compose.onNodeWithContentDescription("Emoji and stickers").performClick()
        compose.onNodeWithTag("emoji-picker").assertIsDisplayed()
        compose.onNodeWithContentDescription("Sticker :papo_wave:").performClick()
        androidx.test.espresso.Espresso.pressBack()
        compose
            .onNode(hasSetTextAction())
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.InputText,
                    AnnotatedString(":papo_wave: "),
                )
            )
        compose.onNodeWithContentDescription("Send message").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription(":papo_wave:", useUnmergedTree = true)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onAllNodesWithTag("message-sticker-demo-wave").onFirst().performClick()
        compose.onNodeWithTag("sticker-details").assertIsDisplayed()
        compose.onNodeWithText("Copy name").performClick()
        compose.onNodeWithText("Copied").assertExists()
        val clipboard =
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .getSystemService(android.content.ClipboardManager::class.java)
        assertEquals("papo_wave", clipboard.primaryClip?.getItemAt(0)?.text.toString())
        compose.onNodeWithText("Favorite", substring = false).performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText("Favorited").fetchSemanticsNodes().isNotEmpty()
        }
        androidx.test.espresso.Espresso.pressBack()
        compose.onAllNodesWithContentDescription("Message actions").onFirst().performClick()
        compose.onNodeWithContentDescription("React with :papo_wave:").assertIsDisplayed()
        compose.onNodeWithContentDescription("More reactions").performClick()
        compose.onNodeWithTag("emoji-picker").assertIsDisplayed()
        compose.onNodeWithContentDescription("Sticker :papo_wave:").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("Reaction papo_wave")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Reaction papo_wave").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("Reaction papo_wave")
                .fetchSemanticsNodes()
                .isEmpty()
        }
        compose.onNodeWithTag("reaction-people").assertDoesNotExist()
        compose.onAllNodesWithContentDescription("Message actions").onFirst().performClick()
        compose.onNodeWithContentDescription("More reactions").performClick()
        compose.onNodeWithContentDescription("Sticker :papo_wave:").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("Reaction papo_wave")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Reaction papo_wave").performTouchInput { longClick() }
        compose.onNodeWithTag("reaction-people").assertIsDisplayed()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithText("You", useUnmergedTree = true)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithText("Remove my reaction").assertExists()
    }

    @Test
    fun gifPickerUsesStableProviderMarkerAndTracksRecentOnlyAfterSending() {
        chat()
        compose.onNodeWithContentDescription("GIF picker").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("A little motion")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("A little motion").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodes(hasSetTextAction()).fetchSemanticsNodes().any {
                it.config
                    .getOrElse(SemanticsProperties.EditableText) { AnnotatedString("") }
                    .text == "giphy:demo-motion"
            }
        }
        compose.onNodeWithContentDescription("Send message").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithTag("provider-gif-demo-motion", useUnmergedTree = true)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("GIF picker").performClick()
        compose.onNodeWithText("Recent").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("A little motion")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose
            .onNode(
                hasContentDescription("A little motion") and
                    hasAnyAncestor(hasTestTag("gif-picker"))
            )
            .assertIsDisplayed()
    }

    @Test
    fun uploadedGifFavoriteIsSharedWithFullscreenAndPicker() {
        media()
        compose
            .onNodeWithTag("message-list")
            .performScrollToNode(hasContentDescription("Photo: animation.gif"))
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasContentDescription("Photo: animation.gif") and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Loaded")
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Photo: animation.gif").performClick()
        compose.onAllNodesWithContentDescription("Favorite GIF").onLast().performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("Remove GIF from favorites")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Close image").performClick()
        compose.onNodeWithContentDescription("GIF picker").performClick()
        compose.onNodeWithText("Favorites").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("animation.gif")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("animation.gif").assertIsDisplayed().performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("Remove animation.gif")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Send message").assertIsEnabled()
        compose.onNodeWithContentDescription("Remove animation.gif").performClick()
    }

    @Test
    fun mentionAndStickerSuggestionsScrollInsertAndRenderInsideTheComposer() {
        chat()
        compose.onNodeWithTag("composer-input").performTextInput("@")
        compose
            .onNodeWithTag("composer-suggestions")
            .assertIsDisplayed()
            .performScrollToNode(hasText("Dora"))
        compose.onNodeWithTag("composer-suggestions").performScrollToNode(hasText("Ana"))
        compose
            .onNode(hasText("Ana") and hasAnyAncestor(hasTestTag("composer-suggestions")))
            .performClick()
        compose
            .onNodeWithTag("composer-input")
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.InputText,
                    AnnotatedString("@Ana "),
                )
            )
        compose.onNodeWithTag("composer-suggestions").assertDoesNotExist()
        compose.onNodeWithTag("composer-input").performTextInput(":pap")
        compose
            .onNode(hasText(":papo_wave:") and hasAnyAncestor(hasTestTag("composer-suggestions")))
            .performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("composer-input") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Contains stickers: papo_wave",
                        )
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose
            .onNodeWithTag("composer-input")
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.InputText,
                    AnnotatedString("@Ana :papo_wave: "),
                )
            )
        compose.onNodeWithTag("composer-suggestions").assertDoesNotExist()
        val bitmap = compose.onRoot().captureToImage()
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        context.cacheDir.resolve("composer-suggestions-verified.png").outputStream().use {
            bitmap.asAndroidBitmap().compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
        }
        compose.onNodeWithContentDescription("Back to conversations").performClick()
        compose.onNodeWithText("geral").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("composer-input").fetchSemanticsNodes().isNotEmpty()
        }
        compose
            .onNodeWithTag("composer-input")
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.InputText,
                    AnnotatedString("@Ana :papo_wave: "),
                )
            )
        compose.onNodeWithContentDescription("Send message").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText("@Ana", substring = true).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText("@mention(<@ana>)", substring = true).assertDoesNotExist()
    }

    @Test
    fun pinButtonIsLeftOfPeopleAndPinUnpinJumpsToTheOriginal() {
        chat()
        val pin =
            compose
                .onNodeWithContentDescription("Pinned messages")
                .fetchSemanticsNode()
                .boundsInRoot
        val people =
            compose
                .onNodeWithContentDescription("Community members")
                .fetchSemanticsNode()
                .boundsInRoot
        assertTrue(pin.right <= people.left)
        compose.onNodeWithTag("message-list").performScrollToKey("general-9")
        compose.onNodeWithTag("message-general-9").performTouchInput { longClick() }
        compose.onNodeWithText("Pin message", substring = false).performClick()
        compose.waitUntil(5000) {
            compose
                .onAllNodesWithText("Pin message", substring = false)
                .fetchSemanticsNodes()
                .isEmpty()
        }
        compose.onNodeWithContentDescription("Pinned messages").performClick()
        compose.waitUntil(5000) {
            compose.onAllNodesWithTag("pin-general-9").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithContentDescription("Refresh pins").assertDoesNotExist()
        compose.onNodeWithText("Open conversation", substring = false).assertDoesNotExist()
        compose
            .onNode(
                hasText("Ana", substring = false) and hasAnyAncestor(hasTestTag("pin-general-9")),
                useUnmergedTree = true,
            )
            .performClick()
        compose.onNodeWithTag("user-profile").assertIsDisplayed()
        compose.onNodeWithContentDescription("Close profile").performClick()
        compose.onNodeWithTag("pinned-messages").assertIsDisplayed()
        compose
            .onNode(
                hasText("o que vocês estão construindo hoje?", substring = false) and
                    hasAnyAncestor(hasTestTag("pin-general-9")),
                useUnmergedTree = true,
            )
            .performClick()
        compose.onNodeWithTag("pinned-messages").assertDoesNotExist()
        compose.waitUntil(5000) {
            compose
                .onAllNodes(
                    hasTestTag("message-general-9") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Original message",
                        )
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Pinned messages").performClick()
        compose.onNodeWithTag("pin-general-9").performTouchInput { longClick() }
        compose.onNodeWithText("Unpin message", substring = false).performClick()
        compose.waitUntil(5000) {
            compose
                .onAllNodesWithText("No pinned messages yet.", substring = false)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        systemBack()
        compose.onNodeWithTag("message-list").assertIsDisplayed()
    }

    @Test
    fun pinManagementIsHiddenWithoutPermissionButPinsRemainBrowsable() {
        chat()
        activity.onActivity { host ->
            val model = androidx.lifecycle.ViewModelProvider(host)[PapoViewModel::class.java]
            val server =
                checkNotNull(model.state.value.active).let {
                    it.copy(canPin = false, capabilities = it.capabilities.copy(pinMessage = false))
                }
            host.setContent {
                androidx.compose.material3.MaterialTheme {
                    androidx.compose.runtime.CompositionLocalProvider(
                        LocalHaze provides dev.chrisbanes.haze.HazeState(),
                        LocalGlass provides false,
                    ) {
                        ConversationScreen(server, model, {})
                    }
                }
            }
        }
        compose.onNodeWithContentDescription("Pinned messages").assertIsDisplayed()
        compose.onNodeWithTag("message-list").performScrollToKey("general-9")
        compose.onNodeWithTag("message-general-9").performTouchInput { longClick() }
        compose.onNodeWithText("Reply", substring = false).assertIsDisplayed()
        compose.onNodeWithText("Pin message", substring = false).assertDoesNotExist()
        compose.onNodeWithText("Unpin message", substring = false).assertDoesNotExist()
        systemBack()
        compose.onNodeWithContentDescription("Pinned messages").performClick()
        compose.onNodeWithTag("pinned-messages").assertIsDisplayed()
    }

    @Test
    fun pinsUseTheSharedVideoPlayerAndImageViewerWithoutLeavingTheSheet() {
        media()
        lateinit var model: PapoViewModel
        lateinit var server: ServerState
        activity.onActivity {
            model = androidx.lifecycle.ViewModelProvider(it)[PapoViewModel::class.java]
            server = checkNotNull(model.state.value.active)
        }
        kotlinx.coroutines.runBlocking {
            listOf("media-0", "media-3").forEach {
                model.setPinned(server, server.selected, it, true)
            }
        }
        compose.onNodeWithContentDescription("Pinned messages").performClick()
        compose.waitUntil(5000) {
            compose.onAllNodesWithTag("pin-media-3").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("pinned-message-list").performScrollToKey("pin/media-3")
        compose
            .onNode(
                hasContentDescription("Play clip.mp4") and hasAnyAncestor(hasTestTag("pin-media-3"))
            )
            .performClick()
        val tag = "player-demo://papo/me/attachment/demo-video"
        compose.waitUntil(15_000) {
            compose
                .onAllNodes(
                    hasTestTag(tag) and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready"),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        var player: androidx.media3.common.Player? = null
        androidx.test.espresso.Espresso.onView(
                org.hamcrest.Matchers.allOf(
                    androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(
                        androidx.media3.ui.PlayerView::class.java
                    ),
                    androidx.test.espresso.matcher.ViewMatchers.isDisplayed(),
                )
            )
            .check { view, _ ->
                val native = view as androidx.media3.ui.PlayerView
                player = native.player
                native.controllerShowTimeoutMs = 0
                native.showController()
            }
        val fullscreen =
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .getString(androidx.media3.ui.R.string.exo_controls_fullscreen_enter_description)
        // Media3 animates its controller independently of Compose's idling.
        compose.waitUntil(5000) {
            var shown = false
            runCatching {
                androidx.test.espresso.Espresso.onView(
                        org.hamcrest.Matchers.allOf(
                            androidx.test.espresso.matcher.ViewMatchers.withContentDescription(
                                fullscreen
                            ),
                            androidx.test.espresso.matcher.ViewMatchers.isDisplayed(),
                        )
                    )
                    .check { view, _ ->
                        shown =
                            androidx.test.espresso.matcher.ViewMatchers.isDisplayingAtLeast(90)
                                .matches(view)
                    }
            }
            shown
        }
        androidx.test.espresso.Espresso.onView(
                org.hamcrest.Matchers.allOf(
                    androidx.test.espresso.matcher.ViewMatchers.withContentDescription(fullscreen),
                    androidx.test.espresso.matcher.ViewMatchers.isDisplayed(),
                )
            )
            .perform(androidx.test.espresso.action.ViewActions.click())
        compose.onNodeWithTag("media-viewer").assertIsDisplayed()
        systemBack()
        compose.onNodeWithTag("pinned-messages").assertIsDisplayed()
        androidx.test.espresso.Espresso.onView(
                org.hamcrest.Matchers.allOf(
                    androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(
                        androidx.media3.ui.PlayerView::class.java
                    ),
                    androidx.test.espresso.matcher.ViewMatchers.isDisplayed(),
                )
            )
            .check { view, _ ->
                assertTrue((view as androidx.media3.ui.PlayerView).player === player)
            }
        compose.waitUntil(5000) {
            var resumed = false
            InstrumentationRegistry.getInstrumentation().runOnMainSync {
                resumed = player?.playWhenReady == true
            }
            resumed
        }
        compose.onNodeWithTag("pinned-message-list").performScrollToKey("pin/media-0")
        compose
            .onNode(
                hasContentDescription("Photo: landscape.png") and
                    hasAnyAncestor(hasTestTag("pin-media-0"))
            )
            .performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("viewer-image") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Loaded",
                        ),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        systemBack()
        compose.onNodeWithTag("pinned-messages").assertIsDisplayed()
    }

    @Test
    fun searchResultsPlayVideoAndWebEmbedsAcrossChannels() {
        demo()
        compose.onNodeWithTag("global-search").performTextInput("em:#mídia")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    SemanticsMatcher("Result") {
                        it.config
                            .getOrNull(SemanticsProperties.TestTag)
                            ?.startsWith("search-hit-") == true
                    }
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("conversation-results").performScrollToKey("message/media-3")
        compose.onNodeWithContentDescription("Play clip.mp4").performClick()
        val tag = "player-demo://papo/me/attachment/demo-video"
        compose.waitUntil(15_000) {
            compose
                .onAllNodes(
                    hasTestTag(tag) and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready"),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("message-list").assertDoesNotExist()
        compose.onNodeWithTag("conversation-results").performScrollToKey("message/media-10")
        compose.onNodeWithContentDescription("Play YouTube preview").performClick()
        androidx.test.espresso.Espresso.onView(
                androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(
                    android.webkit.WebView::class.java
                )
            )
            .check { view, _ ->
                assertTrue(view is android.webkit.WebView)
                assertTrue((view as android.webkit.WebView).settings.javaScriptEnabled)
                assertTrue(!view.settings.allowFileAccess)
            }
        systemBack()
        compose
            .onNodeWithTag("global-search")
            .assert(
                SemanticsMatcher.expectValue(SemanticsProperties.InputText, AnnotatedString(""))
            )
    }

    @Test
    fun markdownRendersRichBlocksAndCopiesCode() {
        chat()
        activity.onActivity { host ->
            val model = androidx.lifecycle.ViewModelProvider(host)[PapoViewModel::class.java]
            val server = checkNotNull(model.state.value.active)
            val text =
                "# Rich message\n\n**Bold** and *italic* with :papo_wave: and @mention(<@ana>).\n\n> A quoted thought\n\n- [x] Completed task\n- [ ] Next task\n\n```kotlin\nval answer = 42\n```\n\n| Name | State |\n| :-- | --: |\n| Papo | Ready |"
            val fixture = io.github.chriexpe.papo.core.NativeClient.demo()
            val message =
                try {
                    fixture.dispatch(
                        io.github.chriexpe.papo.core.Action.Send(
                            "markdown-fixture",
                            fixture.poll().selected,
                            "$text",
                            null,
                        )
                    )
                    fixture.poll().messages.single { it.id == "markdown-fixture" }
                } finally {
                    fixture.close()
                }
            host.setContent {
                androidx.compose.material3.MaterialTheme {
                    androidx.compose.material3.Surface {
                        androidx.compose.foundation.layout.Column(
                            androidx.compose.ui.Modifier.testTag("markdown-fixture")
                                .verticalScroll(androidx.compose.foundation.rememberScrollState())
                                .padding(16.dp)
                        ) {
                            MessageText(message, server, model)
                        }
                    }
                }
            }
        }
        compose.onNodeWithText("Rich message", substring = false).assertIsDisplayed()
        compose.onNodeWithText("A quoted thought", substring = false).assertIsDisplayed()
        compose
            .onNodeWithTag("message-sticker-demo-wave", useUnmergedTree = true)
            .assertIsDisplayed()
        compose.onNodeWithText("Completed task", substring = false).assertIsDisplayed()
        compose
            .onNodeWithText("val answer = 42", substring = false)
            .performScrollTo()
            .assertIsDisplayed()
        compose.onNodeWithContentDescription("Copy code").performClick()
        compose.waitForIdle()
        activity.onActivity {
            assertEquals(
                "val answer = 42",
                it.getSystemService(android.content.ClipboardManager::class.java)
                    .primaryClip
                    ?.getItemAt(0)
                    ?.text
                    ?.toString(),
            )
        }
        compose.onNodeWithText("Ready", substring = false).performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("markdown-fixture").captureToImage().asAndroidBitmap().let { bitmap ->
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .cacheDir
                .resolve("markdown-verified.png")
                .outputStream()
                .use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
            bitmap.recycle()
        }
    }

    @Test
    fun composerAutoClosesFormattingAndSendsRenderedText() {
        chat()
        val input = compose.onNodeWithTag("composer-input")
        input.performTextInput("*")
        input.performTextInput("*")
        input.assert(
            SemanticsMatcher.expectValue(SemanticsProperties.InputText, AnnotatedString("****"))
        )
        input.performTextInput("A formatted thought")
        input.performTextInput("*")
        input.performTextInput("*")
        input.assert(
            SemanticsMatcher.expectValue(
                SemanticsProperties.InputText,
                AnnotatedString("**A formatted thought** "),
            )
        )
        compose.onNodeWithContentDescription("Send message").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithText("A formatted thought", substring = false)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithText("**A formatted thought**", substring = true).assertDoesNotExist()
    }

    @Test
    fun typingPillUsesCustomPhraseAndAlignsWithTheTextBox() {
        chat()
        val people = androidx.compose.runtime.mutableStateOf(listOf("Ana"))
        val phrase = androidx.compose.runtime.mutableStateOf("is sketching…")
        activity.onActivity { host ->
            val model = androidx.lifecycle.ViewModelProvider(host)[PapoViewModel::class.java]
            val server = checkNotNull(model.state.value.active)
            host.setContent {
                androidx.compose.material3.MaterialTheme {
                    androidx.compose.runtime.CompositionLocalProvider(
                        LocalHaze provides dev.chrisbanes.haze.HazeState(),
                        LocalGlass provides false,
                    ) {
                        ConversationScreen(
                            server.copy(typing = people.value, typingPhrase = phrase.value),
                            model,
                            {},
                        )
                    }
                }
            }
        }
        compose.onNodeWithText("Ana is sketching…").assertIsDisplayed()
        val pill = compose.onNodeWithTag("typing-indicator").fetchSemanticsNode().boundsInRoot
        val input = compose.onNodeWithTag("composer-input").fetchSemanticsNode().boundsInRoot
        val gallery =
            compose.onNodeWithContentDescription("Open gallery").fetchSemanticsNode().boundsInRoot
        assertTrue(
            "Typing belongs above the text box, beyond the gallery button",
            pill.left > gallery.right && pill.left <= input.left && pill.bottom <= input.top,
        )
        compose.runOnIdle { people.value = listOf("Bruno", "Ana") }
        compose.onNodeWithText("Ana, Bruno are typing…").assertIsDisplayed()
        compose.runOnIdle {
            people.value = listOf("Ana")
            phrase.value = ""
        }
        compose.onNodeWithText("Ana is typing…").assertIsDisplayed()
        compose.runOnIdle { people.value = emptyList() }
        compose.onNodeWithTag("typing-indicator").assertDoesNotExist()
    }

    @Test
    fun consecutiveMessagesShareTheirHeadingAndCompactReducesTheirSpacing() {
        chat()
        lateinit var model: PapoViewModel
        activity.onActivity {
            model = androidx.lifecycle.ViewModelProvider(it)[PapoViewModel::class.java]
        }
        model.command(
            io.github.chriexpe.papo.core.Action.Send(
                "group-first",
                "general",
                "First grouped message",
                null,
            )
        )
        model.command(
            io.github.chriexpe.papo.core.Action.Send(
                "group-second",
                "general",
                "Second grouped message",
                null,
            )
        )
        compose.waitUntil(5000) {
            compose
                .onAllNodesWithTag("message-group-second", useUnmergedTree = true)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("author-group-first", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithTag("author-group-second", useUnmergedTree = true).assertDoesNotExist()
        fun textBaseline(node: SemanticsNodeInteraction): Float {
            val layouts = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
            node.performSemanticsAction(
                androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult
            ) {
                it(layouts)
            }
            return node.fetchSemanticsNode().boundsInRoot.top + layouts.single().firstBaseline
        }
        fun nicknameGap() =
            textBaseline(compose.onNodeWithText("First grouped message", useUnmergedTree = true)) -
                textBaseline(compose.onNodeWithTag("author-group-first", useUnmergedTree = true))
        val normalGap = nicknameGap()
        val normal =
            compose
                .onNodeWithTag("message-group-first", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
                .height
        compose.onNodeWithContentDescription("Back to conversations").performClick()
        compose.onNodeWithContentDescription("Client settings").performClick()
        compose.onNodeWithText("Appearance").performClick()
        compose
            .onNodeWithTag("appearance-settings")
            .performScrollToNode(hasContentDescription("Compact messages"))
        compose.onNodeWithContentDescription("Compact messages").performClick()
        androidx.test.espresso.Espresso.pressBack()
        androidx.test.espresso.Espresso.pressBack()
        compose.onNodeWithText("geral").performClick()
        val compact =
            compose
                .onNodeWithTag("message-group-first", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
                .height
        assertTrue("Compact must actually reduce the message row height", compact < normal)
        assertTrue(
            "Compact must reduce nickname-to-message spacing, not just outer padding",
            nicknameGap() < normalGap,
        )
        assertTrue(
            "Compact must not retain the old 48 dp author-header gap",
            nicknameGap() <=
                34 *
                    InstrumentationRegistry.getInstrumentation()
                        .targetContext
                        .resources
                        .displayMetrics
                        .density,
        )
        compose.onNodeWithTag("author-group-second", useUnmergedTree = true).assertDoesNotExist()
        val bitmap = compose.onRoot().captureToImage().asAndroidBitmap()
        InstrumentationRegistry.getInstrumentation()
            .targetContext
            .cacheDir
            .resolve("grouped-compact-verified.png")
            .outputStream()
            .use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
        compose.onNodeWithTag("message-group-second", useUnmergedTree = true).performTouchInput {
            longClick()
        }
        compose.onNodeWithText("Reply", substring = false).performClick()
        compose.onNodeWithTag("composer-reply").assertIsDisplayed()
    }

    @Test
    fun mainRoleColorIsSharedByChatAndPeople() {
        chat()
        compose.onNodeWithTag("message-list").performScrollToKey("general-0")
        fun textColor(tag: String): androidx.compose.ui.graphics.Color {
            val layouts = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
            compose.onNodeWithTag(tag, useUnmergedTree = true).performSemanticsAction(
                androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult
            ) {
                it(layouts)
            }
            return layouts.single().layoutInput.style.color
        }
        assertEquals(androidx.compose.ui.graphics.Color(0xFF67B989), textColor("author-general-0"))
        compose.onNodeWithContentDescription("Community members").performClick()
        assertEquals(androidx.compose.ui.graphics.Color(0xFF67B989), textColor("member-name-ana"))
        assertTrue(textColor("member-name-bruno") != androidx.compose.ui.graphics.Color(0xFF67B989))
    }

    @Test
    fun nativeAndFirebaseNotificationTapsOpenTheirMessageWithoutLosingDrafts() {
        chat()
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        val permission = android.Manifest.permission.POST_NOTIFICATIONS
        val server = "demo://papo"
        val manager = context.getSystemService(android.app.NotificationManager::class.java)
        automation.grantRuntimePermission(context.packageName, permission)
        try {
            compose.onNodeWithTag("composer-input").performTextInput("Keep notification draft")
            androidx.test.espresso.Espresso.closeSoftKeyboard()
            MessageAlerts.setOwner(server, "me")
            MessageAlerts.listener.show(
                io.github.chriexpe.papo.core.MessageAlert(
                    server,
                    "me",
                    "design",
                    "design-0",
                    "test-notice",
                    "Design",
                    "A notification",
                )
            )
            compose.waitUntil(5000) {
                manager.activeNotifications.any { it.tag == "papo-message:$server:me:design-0" }
            }
            val notice =
                manager.activeNotifications.single { it.tag == "papo-message:$server:me:design-0" }
            notice.notification.contentIntent.send()
            compose.waitUntil(10_000) {
                compose
                    .onAllNodes(hasTestTag("channel-header") and hasText("design"))
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            compose.onNodeWithTag("message-design-0", useUnmergedTree = true).assertIsDisplayed()
            val intent =
                android.content
                    .Intent(context, MainActivity::class.java)
                    .addFlags(
                        android.content.Intent.FLAG_ACTIVITY_NEW_TASK or
                            android.content.Intent.FLAG_ACTIVITY_SINGLE_TOP
                    )
                    .putExtra("google.message_id", "fcm-fixture")
                    .putExtra("channel_id", "general")
                    .putExtra("message_id", "general-0")
            context.startActivity(intent)
            compose.waitUntil(10_000) {
                compose
                    .onAllNodes(hasTestTag("channel-header") and hasText("geral"))
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            compose.onNodeWithTag("composer-input").assertTextEquals("Keep notification draft")
            compose.onNodeWithTag("message-general-0", useUnmergedTree = true).assertIsDisplayed()
            activity.recreate()
            compose.onNodeWithTag("composer-input").assertTextEquals("Keep notification draft")
        } finally {
            MessageAlerts.clear(server, "me", "design")
        }
    }

    @Test
    fun incomingTextAndMediaAtTheLiveHeadNeverResurrectAnOldUnreadBlock() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        compose.waitForIdle()
        IncomingChatFixture(context).use { fixture ->
            val secrets = AndroidSecrets(context)
            lateinit var model: PapoViewModel
            activity.onActivity {
                model = androidx.lifecycle.ViewModelProvider(it)[PapoViewModel::class.java]
            }
            try {
                model.addServer(fixture.url)
                compose.waitUntil(5000) { model.state.value.active?.url == fixture.url }
                model.command(
                    io.github.chriexpe.papo.core.Action.Login("reader", "fixture"),
                    fixture.url,
                )
                compose.waitUntil(30_000) {
                    model.state.value.active?.let {
                        it.url == fixture.url &&
                            it.connection == "online" &&
                            it.timeline == "fresh" &&
                            it.messages.any { m -> m.id == "m30" }
                    } == true
                }
                compose.onNodeWithText("incoming", substring = false).performClick()
                compose.waitUntil(10_000) {
                    compose
                        .onAllNodesWithTag("message-m30", useUnmergedTree = true)
                        .fetchSemanticsNodes()
                        .isNotEmpty() && model.readingPosition(fixture.url + "/me/incoming") != null
                }
                assertTrue(model.state.value.active!!.reading.hourglassTarget.isBlank())
                val unexpected = java.util.concurrent.CopyOnWriteArrayList<String>()
                val observer =
                    kotlinx.coroutines
                        .CoroutineScope(kotlinx.coroutines.Dispatchers.Unconfined)
                        .launch {
                            model.state.collect { state ->
                                state.active
                                    ?.takeIf { it.url == fixture.url }
                                    ?.reading
                                    ?.let { read ->
                                        if (read.hourglassTarget.isNotBlank())
                                            unexpected.add(
                                                "target=" +
                                                    read.hourglassTarget +
                                                    ", gap=" +
                                                    read.gapStart +
                                                    ".." +
                                                    read.gapEnd +
                                                    ", new=" +
                                                    read.unreadStart
                                            )
                                    }
                            }
                        }
                fun settle(id: String) {
                    compose.waitUntil(10_000) {
                        model.state.value.active?.let {
                            it.reading.liveIds.contains(id) &&
                                it.conversation?.unread == false &&
                                !it.reading.syncPending
                        } == true
                    }
                    compose
                        .onNodeWithTag("message-" + id, useUnmergedTree = true)
                        .assertIsDisplayed()
                    compose.onNodeWithTag("hourglass-shortcut").assertDoesNotExist()
                    compose.onNodeWithTag("unread-new", useUnmergedTree = true).assertDoesNotExist()
                    assertTrue(
                        "Following incoming messages must not manufacture catch-up: " + unexpected,
                        unexpected.isEmpty(),
                    )
                }
                try {
                    fixture.incoming("incoming-one", "One incoming message")
                    settle("incoming-one")
                    fixture.incoming("incoming-two-a", "First of two incoming messages")
                    fixture.incoming("incoming-two-b", "Second incoming message")
                    settle("incoming-two-b")
                    fixture.incoming(
                        "incoming-portrait",
                        "Portrait media pushes the viewport",
                        portrait = true,
                    )
                    fixture.incoming("incoming-after-media", "Text after tall media")
                    settle("incoming-after-media")
                    fixture.expandWithPortrait("incoming-after-media")
                    compose.waitUntil(5000) {
                        model.state.value.active
                            ?.messages
                            ?.firstOrNull { it.id == "incoming-after-media" }
                            ?.embeds
                            ?.isNotEmpty() == true
                    }
                    fixture.incoming("incoming-after-reflow", "Text after the embed changes height")
                    settle("incoming-after-reflow")
                    compose.onNodeWithTag("composer-input").performClick()
                    fixture.incoming(
                        "incoming-keyboard",
                        "Incoming message while the keyboard resizes the chat",
                    )
                    settle("incoming-keyboard")
                    systemBack()
                    compose.onNodeWithTag("message-list").performScrollToKey("m28")
                    compose.waitForIdle()
                    compose.waitUntil(5000) {
                        model.readingPosition(fixture.url + "/me/incoming")?.first == "m28"
                    }
                    fixture.incoming(
                        "incoming-behind-one",
                        "First incoming message while slightly behind",
                    )
                    fixture.incoming(
                        "incoming-behind-two",
                        "Second incoming message while slightly behind",
                    )
                    compose.waitUntil(5000) {
                        compose.onAllNodesWithTag("latest-count").fetchSemanticsNodes().isNotEmpty()
                    }
                    compose.onNodeWithContentDescription("Go to first new message").performClick()
                    settle("incoming-behind-two")
                } finally {
                    observer.cancel()
                }
            } catch (error: Throwable) {
                context.cacheDir
                    .resolve("incoming-read-failure.txt")
                    .writeText(
                        "state=" +
                            model.state.value +
                            "\nrequests=" +
                            fixture.requests +
                            "\nfailures=" +
                            fixture.failures +
                            "\n" +
                            compose
                                .onAllNodes(isRoot(), useUnmergedTree = true)
                                .printToString(maxDepth = 20)
                    )
                throw error
            } finally {
                model.state.value.active
                    ?.takeIf { it.url == fixture.url }
                    ?.key
                    ?.let { secrets.remove(it, "token") }
            }
        }
    }

    @Test
    fun arrivalsAtTheLiveHeadStayVisibleAndClearTheChannelBadge() {
        chat()
        lateinit var model: PapoViewModel
        activity.onActivity {
            model = androidx.lifecycle.ViewModelProvider(it)[PapoViewModel::class.java]
        }
        for (i in 0 until 5) model.command(
            io.github.chriexpe.papo.core.Action.Send(
                "head-arrival-$i",
                "general",
                "Live head arrival $i",
                null,
            )
        )
        compose.waitUntil(5000) {
            model.state.value.active?.reading?.liveIds?.contains("head-arrival-4") == true
        }
        compose.waitUntil(5000) {
            compose
                .onAllNodesWithTag("message-head-arrival-4", useUnmergedTree = true)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("message-head-arrival-4", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithTag("latest-count").assertDoesNotExist()
        assertTrue(model.state.value.active?.conversation?.unread == false)
    }

    @Test
    fun countedDownArrowStartsAtFirstArrivalThenReturnsToLatestAndBadgeIsNotClipped() {
        chat()
        compose.onNodeWithTag("message-list").performScrollToKey("general-0")
        lateinit var model: PapoViewModel
        activity.onActivity {
            model = androidx.lifecycle.ViewModelProvider(it)[PapoViewModel::class.java]
        }
        for (i in 0 until 20) model.command(
            io.github.chriexpe.papo.core.Action.Send(
                "read-arrival-%02d".format(i),
                "general",
                "New arrival $i\n" +
                    "A longer message to keep separate reading positions. ".repeat(5),
                null,
            )
        )
        compose.waitUntil(10_000) {
            model.state.value.active?.reading?.liveIds?.contains("read-arrival-19") == true &&
                compose.onAllNodesWithTag("latest-count").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("latest-count").assert(hasAnyDescendant(hasText("20")))
        val badge = compose.onNodeWithTag("latest-count").fetchSemanticsNode().boundsInRoot
        val glass = compose.onNodeWithTag("latest-messages").fetchSemanticsNode().boundsInRoot
        assertTrue(
            "Badge must extend beyond the glass without being clipped",
            badge.top < glass.top && badge.right > glass.right,
        )
        val screenshot = compose.onRoot().captureToImage().asAndroidBitmap()
        val right = screenshot.getPixel((badge.right - 3).toInt(), badge.center.y.toInt())
        val bottom = screenshot.getPixel(badge.center.x.toInt(), (badge.bottom - 3).toInt())
        assertEquals("The right edge of the badge must retain its background", bottom, right)
        InstrumentationRegistry.getInstrumentation()
            .targetContext
            .cacheDir
            .resolve("read-shortcuts-verified.png")
            .outputStream()
            .use { screenshot.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
        compose.onNodeWithContentDescription("Go to first new message").performClick()
        compose.waitUntil(5000) { model.state.value.active?.navigationTarget == "read-arrival-00" }
        compose.onNodeWithTag("message-read-arrival-00", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithContentDescription("Latest messages").performClick()
        compose.waitUntil(5000) {
            compose.onAllNodesWithTag("latest-count").fetchSemanticsNodes().isEmpty()
        }
        compose.onNodeWithTag("message-read-arrival-19", useUnmergedTree = true).assertIsDisplayed()
    }

    @Test
    fun newestClearsChannelBadgeButKeepsCatchUpUntilSecondLatestPress() {
        demo()
        lateinit var model: PapoViewModel
        activity.onActivity {
            model = androidx.lifecycle.ViewModelProvider(it)[PapoViewModel::class.java]
        }
        model.command(
            io.github.chriexpe.papo.core.Action.Visible("general", listOf("general-0"), false, "me")
        )
        compose.waitUntil(5000) { model.state.value.active?.reading?.gapStart?.isBlank() == true }
        model.command(io.github.chriexpe.papo.core.Action.BreakReadSpan("general"))
        model.command(
            io.github.chriexpe.papo.core.Action.Visible("general", listOf("general-9"), true, "me")
        )
        compose.waitUntil(5000) {
            model.state.value.active?.reading?.hourglassTarget?.isNotBlank() == true
        }
        compose.onNodeWithText("geral").performClick()
        compose.waitUntil(5000) {
            compose.onAllNodesWithTag("hourglass-shortcut").fetchSemanticsNodes().isNotEmpty()
        }
        compose.waitUntil(5000) { model.state.value.active?.conversation?.unread == false }
        compose.onNodeWithTag("hourglass-shortcut").assertIsDisplayed()
        compose.onNodeWithTag("latest-messages").performClick()
        compose.onNodeWithContentDescription("Clear catch-up block").assertIsDisplayed()
        compose.onNodeWithContentDescription("Clear catch-up block").performClick()
        compose.waitUntil(5000) {
            model.state.value.active?.reading?.hourglassTarget?.isBlank() == true
        }
        compose.onNodeWithTag("hourglass-shortcut").assertDoesNotExist()
        compose.onNodeWithTag("latest-messages").assertDoesNotExist()
    }

    @Test
    fun catchUpPositionAndRemainingBlockSurviveLeavingAndActivityRecreation() {
        demo()
        lateinit var model: PapoViewModel
        activity.onActivity {
            model = androidx.lifecycle.ViewModelProvider(it)[PapoViewModel::class.java]
        }
        compose.onNodeWithContentDescription("Client settings").performClick()
        compose.onNodeWithText("Appearance").performClick()
        compose
            .onNodeWithTag("appearance-settings")
            .performScrollToNode(hasContentDescription("Open at newest message"))
        compose.onNodeWithContentDescription("Open at newest message").performClick()
        androidx.test.espresso.Espresso.pressBack()
        androidx.test.espresso.Espresso.pressBack()
        model.command(
            io.github.chriexpe.papo.core.Action.Visible("general", listOf("general-0"), false, "me")
        )
        model.command(io.github.chriexpe.papo.core.Action.BreakReadSpan("general"))
        model.command(
            io.github.chriexpe.papo.core.Action.Visible("general", listOf("general-9"), true, "me")
        )
        compose.waitUntil(5000) {
            model.state.value.active?.reading?.hourglassTarget?.isNotBlank() == true
        }
        compose.onNodeWithText("geral").performClick()
        compose.onNodeWithTag("hourglass-shortcut").performClick()
        val key = "demo://papo/me/general"
        compose.waitUntil(5000) {
            model.readingPosition(key)?.first?.let { it != "general-9" } == true
        }
        val anchor = model.readingPosition(key)!!.first
        val remaining = model.state.value.active!!.reading.hourglassTarget
        val beforeBounds =
            compose
                .onNodeWithTag("message-$anchor", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
        assertTrue(
            "There is still catch-up content after reading its beginning",
            remaining.isNotBlank(),
        )
        compose.onNodeWithContentDescription("Back to conversations").performClick()
        compose.onNodeWithText("design", substring = false).performClick()
        compose.onNodeWithContentDescription("Back to conversations").performClick()
        compose.onNodeWithText("geral").performClick()
        compose.onNodeWithTag("message-$anchor", useUnmergedTree = true).assertIsDisplayed()
        assertEquals(remaining, model.state.value.active!!.reading.hourglassTarget)
        val reopenedBounds =
            compose
                .onNodeWithTag("message-$anchor", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
        assertEquals(
            "Restore the offset, not just a nearby message",
            beforeBounds.top,
            reopenedBounds.top,
            2f,
        )
        activity.recreate()
        compose.waitUntil(5000) {
            compose
                .onAllNodesWithTag("message-$anchor", useUnmergedTree = true)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("message-$anchor", useUnmergedTree = true).assertIsDisplayed()
        assertEquals(remaining, model.state.value.active!!.reading.hourglassTarget)
        val recreatedBounds =
            compose
                .onNodeWithTag("message-$anchor", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
        assertEquals(
            "Recreation restores the reading offset",
            beforeBounds.top,
            recreatedBounds.top,
            2f,
        )
    }

    @Test
    fun hourglassSettingHidesItsControlsWithoutClearingReadingPositionOrMentions() {
        demo()
        lateinit var model: PapoViewModel
        activity.onActivity {
            model = androidx.lifecycle.ViewModelProvider(it)[PapoViewModel::class.java]
        }
        model.command(
            io.github.chriexpe.papo.core.Action.Visible("general", listOf("general-0"), false, "me")
        )
        model.command(io.github.chriexpe.papo.core.Action.BreakReadSpan("general"))
        model.command(
            io.github.chriexpe.papo.core.Action.Visible("general", listOf("general-9"), true, "me")
        )
        compose.waitUntil(5000) {
            model.state.value.active?.reading?.hourglassTarget?.isNotBlank() == true
        }
        compose.onNodeWithContentDescription("Client settings").performClick()
        compose.onNodeWithText("Appearance").performClick()
        compose
            .onNodeWithTag("appearance-settings")
            .performScrollToNode(hasContentDescription("Catch-up hourglass"))
        compose.onNodeWithContentDescription("Catch-up hourglass").performClick()
        compose
            .onNodeWithTag("appearance-settings")
            .performScrollToNode(hasContentDescription("Open at newest message"))
        compose.onNodeWithContentDescription("Open at newest message").performClick()
        androidx.test.espresso.Espresso.pressBack()
        androidx.test.espresso.Espresso.pressBack()
        compose.onNodeWithText("geral").performClick()
        compose.onNodeWithTag("hourglass-shortcut").assertDoesNotExist()
        compose.onNodeWithTag("unread-gap-start").assertDoesNotExist()
        compose.onNodeWithTag("unread-gap-end").assertDoesNotExist()
        assertTrue(model.state.value.active!!.reading.hourglassTarget.isNotBlank())
        compose.onNodeWithTag("message-list").performScrollToKey("general-0")
        compose.waitUntil(5000) {
            model.readingPosition("demo://papo/me/general")?.first?.let { it != "general-9" } ==
                true
        }
        compose.onNodeWithContentDescription("Back to conversations").performClick()
        compose.onNodeWithText("geral").performClick()
        compose.onNodeWithTag("message-general-0", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithTag("hourglass-shortcut").assertDoesNotExist()
    }

    @Test
    fun lastPositionRestoresAfterLeavingTheChatAndKeepsTheDraft() {
        demo()
        lateinit var model: PapoViewModel
        activity.onActivity {
            model = androidx.lifecycle.ViewModelProvider(it)[PapoViewModel::class.java]
        }
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        // Load the preference through the Activity's real observable settings source.
        compose.onNodeWithContentDescription("Client settings").performClick()
        compose.onNodeWithText("Appearance").performClick()
        compose
            .onNodeWithTag("appearance-settings")
            .performScrollToNode(hasText("Open at newest message"))
        compose.onNodeWithContentDescription("Open at newest message").performClick()
        androidx.test.espresso.Espresso.pressBack()
        androidx.test.espresso.Espresso.pressBack()
        compose.onNodeWithText("geral").performClick()
        compose.onNodeWithTag("composer-input").performTextInput("Keep reading draft")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.onNodeWithTag("message-list").performScrollToKey("general-0")
        compose.waitUntil(5000) {
            model
                .readingPosition(
                    "${model.state.value.active!!.url}/${model.state.value.active!!.me}/general"
                )
                ?.first
                ?.let { it != "general-9" } == true
        }
        compose.onNodeWithContentDescription("Back to conversations").performClick()
        compose.onNodeWithText("geral").performClick()
        compose.onNodeWithTag("message-general-0", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithTag("composer-input").assertTextEquals("Keep reading draft")
        assertTrue(
            !context.getSharedPreferences("compose-appearance", 0).getBoolean("open-newest", true)
        )
    }

    @Test
    fun swipingLeftRepliesAndOpensTheKeyboardWithoutReplacingTheDraft() {
        chat()
        compose.onNodeWithTag("composer-input").performTextInput("Keep my draft")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.onNodeWithTag("message-list").performScrollToKey("general-9")
        compose.onNodeWithTag("message-general-9", useUnmergedTree = true).performTouchInput {
            swipeLeft()
        }
        compose
            .onNodeWithTag("composer-reply")
            .assertIsDisplayed()
            .assert(hasText("Reply to Ana", substring = true))
        compose.onNodeWithTag("composer-input").assertTextEquals("Keep my draft").assertIsFocused()
        compose.waitUntil(5000) {
            var visible = false
            activity.onActivity {
                visible =
                    androidx.core.view.ViewCompat.getRootWindowInsets(it.window.decorView)
                        ?.isVisible(androidx.core.view.WindowInsetsCompat.Type.ime()) == true
            }
            visible
        }
        compose.onNodeWithTag("composer-input").performTextReplacement("A swiped reply")
        compose.onNodeWithContentDescription("Send message").performClick()
        compose.waitUntil(10_000) {
            var replied = false
            activity.onActivity { host ->
                replied =
                    androidx.lifecycle
                        .ViewModelProvider(host)[PapoViewModel::class.java]
                        .state
                        .value
                        .active
                        ?.messages
                        ?.any { it.content == "A swiped reply" && it.replyTo == "general-9" } ==
                        true
            }
            replied && compose.onAllNodesWithTag("composer-reply").fetchSemanticsNodes().isEmpty()
        }
        compose.onNodeWithText("A swiped reply", substring = false).assertIsDisplayed()
    }

    @Test
    fun shortOrVerticalSwipesDoNotReplyAndOppositeSwipesDoNotLeaveChat() {
        chat()
        val row = compose.onNodeWithTag("message-general-9", useUnmergedTree = true)
        val pixels =
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .resources
                .displayMetrics
                .density
        row.performTouchInput { swipe(center, center.copy(x = center.x - 28 * pixels), 300) }
        compose.onNodeWithTag("composer-reply").assertDoesNotExist()
        row.performTouchInput { swipeRight() }
        compose.onNodeWithTag("composer-reply").assertDoesNotExist()
        compose.onNodeWithTag("message-list").assertIsDisplayed()
        row.performTouchInput { swipeDown() }
        compose.onNodeWithTag("composer-reply").assertDoesNotExist()
        compose.onNodeWithTag("message-list").assertIsDisplayed()
    }

    @Test
    fun latestMessagesFloatsAboveSendWithoutResizingTheComposer() {
        chat()
        val before = compose.onNodeWithTag("chat-composer").fetchSemanticsNode().boundsInRoot
        compose.onNodeWithTag("message-list").performScrollToKey("general-0")
        compose.onNodeWithContentDescription("Latest messages").assertIsDisplayed()
        val after = compose.onNodeWithTag("chat-composer").fetchSemanticsNode().boundsInRoot
        assertEquals(
            "The jump control must not change composer height",
            before.height,
            after.height,
            1f,
        )
        assertEquals("The jump control must not move the composer", before.top, after.top, 1f)
        val jump = compose.onNodeWithTag("latest-messages").fetchSemanticsNode().boundsInRoot
        val send =
            compose.onNodeWithContentDescription("Send message").fetchSemanticsNode().boundsInRoot
        assertTrue("The jump control sits above the composer", jump.bottom <= after.top)
        assertEquals("The jump control lines up with Send", send.center.x, jump.center.x, 1f)
        compose.onNodeWithContentDescription("Latest messages").performClick()
        compose.waitUntil(5000) {
            compose
                .onAllNodesWithContentDescription("Latest messages")
                .fetchSemanticsNodes()
                .isEmpty()
        }
    }

    @Test
    fun replyPreviewsJumpHighlightAndKeepTheComposerDraft() {
        chat()
        compose.onNodeWithTag("composer-input").performTextInput("Keep this draft @An")
        compose.onNodeWithTag("message-list").performScrollToKey("general-6")
        compose.onNodeWithTag("reply-preview-general-6", useUnmergedTree = true).performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("message-general-5") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Original message",
                        ),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("message-general-5", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithTag("composer-input").assertTextEquals("Keep this draft @An")
        compose.onNodeWithTag("composer-suggestions").assertDoesNotExist()
        val preview = compose.onRoot().captureToImage().asAndroidBitmap()
        InstrumentationRegistry.getInstrumentation()
            .targetContext
            .cacheDir
            .resolve("reply-jump-verified.png")
            .outputStream()
            .use { preview.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
        compose.waitUntil(4_000) {
            compose
                .onAllNodes(
                    hasTestTag("message-general-5") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Original message",
                        ),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isEmpty()
        }
        compose
            .onNode(
                hasContentDescription("Message actions") and
                    hasAnyAncestor(hasTestTag("message-general-5")),
                useUnmergedTree = true,
            )
            .performClick()
        compose.onNodeWithText("Reply", substring = false).performClick()
        compose.onNodeWithTag("composer-reply").assertIsDisplayed().performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("message-general-5") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Original message",
                        ),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Cancel reply").performClick()
        compose.onNodeWithTag("composer-reply").assertDoesNotExist()
        compose.onNodeWithTag("composer-input").assertTextEquals("Keep this draft @An")
    }

    @Test
    fun replyingToAnAlreadyVisibleMessageHighlightsWithoutMovingIt() {
        chat()
        compose
            .onNode(
                hasContentDescription("Message actions") and
                    hasAnyAncestor(hasTestTag("message-general-8")),
                useUnmergedTree = true,
            )
            .performClick()
        compose.onNodeWithText("Reply", substring = false).performClick()
        compose.waitForIdle()
        val original = compose.onNodeWithTag("message-general-8", useUnmergedTree = true)
        val before = original.fetchSemanticsNode().boundsInRoot
        val header = compose.onNodeWithTag("channel-header").fetchSemanticsNode().boundsInRoot
        val composer = compose.onNodeWithTag("composer-reply").fetchSemanticsNode().boundsInRoot
        assertTrue(
            "The original must be fully visible before tapping the reply",
            before.top >= header.bottom && before.bottom <= composer.top,
        )
        compose.onNodeWithTag("composer-reply").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("message-general-8") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Original message",
                        ),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        assertEquals(
            "Visible replies should not recenter",
            before.top,
            original.fetchSemanticsNode().boundsInRoot.top,
            1f,
        )
    }

    @Test
    fun offscreenReplyStartsMovingTowardTheOriginalWithoutAnInstantJump() {
        chat()
        compose.onNodeWithTag("message-list").performScrollToNode(hasTestTag("message-general-0"))
        compose
            .onNode(
                hasContentDescription("Message actions") and
                    hasAnyAncestor(hasTestTag("message-general-0")),
                useUnmergedTree = true,
            )
            .performClick()
        compose.onNodeWithText("Reply", substring = false).performClick()
        compose.onNodeWithContentDescription("Latest messages").performClick()
        compose.waitForIdle()
        fun scrollPosition() =
            compose
                .onNodeWithTag("message-list")
                .fetchSemanticsNode()
                .config[SemanticsProperties.VerticalScrollAxisRange]
                .value()
        val before = scrollPosition()
        assertTrue("Start at the latest messages", before < 1f)
        val newest = compose.onNodeWithTag("message-general-9", useUnmergedTree = true)
        val initialTop = newest.fetchSemanticsNode().boundsInRoot.top
        val viewportHeight =
            compose.onNodeWithTag("message-list").fetchSemanticsNode().boundsInRoot.height
        compose.mainClock.autoAdvance = false
        try {
            compose.onNodeWithTag("composer-reply").performClick()
            compose.waitUntil(10_000) {
                compose.mainClock.advanceTimeByFrame()
                scrollPosition() > before + 0.01f
            }
            newest.assertIsDisplayed()
            val movement = newest.fetchSemanticsNode().boundsInRoot.top - initialTop
            assertTrue(
                "Older replies should start with a small downward movement of the visible content; got $movement px",
                movement > 0f && movement < viewportHeight / 4,
            )
        } finally {
            compose.mainClock.autoAdvance = true
        }
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("message-general-0") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Original message",
                        ),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("message-general-0", useUnmergedTree = true).assertIsDisplayed()
    }

    @Test
    fun oldReplyLoadsItsContextAndReturnsToLatestWithoutLosingDrafts() {
        chat()
        compose.onNodeWithTag("composer-input").performTextInput("Still here")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(500, 5000)
        compose.onNodeWithTag("message-list").performScrollToKey("general-0")
        compose.onNodeWithTag("reply-preview-general-0", useUnmergedTree = true).performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("message-demo-archived-original") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Original message",
                        ),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithText("An older hello from this conversation.").assertIsDisplayed()
        compose.onNodeWithTag("composer-input").assertTextEquals("Still here")
        compose.onNodeWithContentDescription("Latest messages").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithText("o que vocês estão construindo hoje?")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithText("o que vocês estão construindo hoje?").assertIsDisplayed()
        compose.onNodeWithTag("composer-input").assertTextEquals("Still here")
    }

    @Test
    fun firstLaunchHasNoPrefilledServerAndValidatesAddressesInline() {
        compose.onNodeWithText("Add server").assertIsNotEnabled()
        compose
            .onNode(hasSetTextAction())
            .assert(
                SemanticsMatcher.expectValue(SemanticsProperties.EditableText, AnnotatedString(""))
            )
        compose.onNode(hasSetTextAction()).performTextInput("file:///tmp/papo")
        compose.onNodeWithText("Add server").performScrollTo().assertIsEnabled().performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithText("Use an http(s) server address", substring = true)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithText("Explore demo").assertExists()
    }

    @Test
    fun nativeDemoSendsUnicodeAndKeepsDraftsAcrossConversationNavigation() {
        chat()
        val draft = "rascunho 🌱\nsegunda linha"
        compose.onNode(hasSetTextAction()).performTextInput(draft)
        InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(500, 5000)
        val sendCenter =
            compose
                .onNodeWithContentDescription("Send message")
                .fetchSemanticsNode()
                .boundsInRoot
                .center
                .y
        assertEquals(
            sendCenter,
            compose
                .onNodeWithContentDescription("Emoji and stickers")
                .fetchSemanticsNode()
                .boundsInRoot
                .center
                .y,
            2f,
        )
        assertEquals(
            sendCenter,
            compose
                .onNodeWithContentDescription("GIF picker")
                .fetchSemanticsNode()
                .boundsInRoot
                .center
                .y,
            2f,
        )
        compose.onNodeWithContentDescription("Back to conversations").performClick()
        compose.onNodeWithText("geral").performClick()
        compose.onNode(hasSetTextAction()).assertTextEquals(draft)
        compose.onNodeWithContentDescription("Send message").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText(draft).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithContentDescription("Send message").assertIsNotEnabled()
    }

    @Test
    fun appearanceControlsAndDemoExitWork() {
        demo()
        compose.onNodeWithContentDescription("Client settings").performClick()
        compose.onNodeWithText("Appearance", substring = false).performClick()
        compose.onNodeWithText("Dark").performClick()
        compose.onNodeWithContentDescription("Frosted controls").performScrollTo().performClick()
        systemBack()
        compose.onNodeWithText("Leave demo").performScrollTo().performClick()
        compose.onNodeWithText("Explore demo").assertExists()
        compose.onNodeWithText("Add server").assertIsNotEnabled()
    }

    @Test
    fun settingsSearchFindsIndividualControlsAndOpensTheirExactLocation() {
        demo()
        compose.onNodeWithContentDescription("Client settings").performClick()
        compose.onNodeWithTag("settings-search").performTextInput("downloads")
        compose.onNodeWithText("Save media to", substring = false).assertIsDisplayed()
        compose.onNodeWithText("Files", substring = false).assertDoesNotExist()
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.onNodeWithText("Save media to", substring = false).performClick()
        compose.onNodeWithText("Downloads/Papo", substring = false).assertIsDisplayed()
        systemBack()
        compose.onNodeWithContentDescription("Clear settings search").performClick()
        compose.onNodeWithTag("settings-search").performTextInput("timestamps")
        compose.onNodeWithText("Show timestamps", substring = false).performClick()
        compose.onNodeWithContentDescription("Show timestamps").assertIsDisplayed().performClick()
        compose.onNodeWithContentDescription("Show timestamps").assertIsOff()
        systemBack()
        compose.onNodeWithTag("settings-search").assertTextContains("timestamps")
        compose.onNodeWithContentDescription("Clear settings search").performClick()
        compose.onNodeWithText("Activity", substring = false).assertDoesNotExist()
        compose.onNodeWithTag("settings-search").performTextInput("background delivery")
        compose
            .onNodeWithText("Background delivery", substring = false)
            .assertIsDisplayed()
            .performClick()
        compose.onNodeWithTag("notification-settings").assertIsDisplayed()
        compose.onNodeWithContentDescription("Message notifications").assertIsDisplayed()
    }

    @Test
    fun accountEditsUseTheCoreAndDiscardingKeepsSavedProfile() {
        demo()
        compose.onNodeWithContentDescription("Client settings").performClick()
        compose.onNodeWithText("Profile, presence and security").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("account-nickname").fetchSemanticsNodes().isNotEmpty()
        }
        compose
            .onNodeWithTag("account-settings")
            .performScrollToNode(hasTestTag("account-nickname"))
        compose.onNodeWithTag("account-nickname").performTextReplacement("Android tester")
        compose.onNodeWithTag("account-settings").performScrollToNode(hasTestTag("account-status"))
        compose.onNodeWithTag("account-status").performTextReplacement("Building Papo")
        compose.onNodeWithTag("account-settings").performScrollToNode(hasTestTag("account-typing"))
        compose.onNodeWithTag("account-typing").performTextReplacement("is making things…")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        compose.onNodeWithTag("save-account").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithText("Profile saved").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("save-account").assertIsNotEnabled()
        compose.onNodeWithTag("account-settings").performScrollToNode(hasTestTag("account-about"))
        compose
            .onNodeWithTag("account-about")
            .assertTextContains("Construindo o Papo para Android.")
        compose
            .onNodeWithTag("account-settings")
            .performScrollToNode(hasTestTag("account-nickname"))
        compose.onNodeWithTag("account-nickname").performTextReplacement("Unsaved name")
        androidx.test.espresso.Espresso.closeSoftKeyboard()
        systemBack()
        compose.onNodeWithText("Discard profile changes?").assertIsDisplayed()
        compose.onNodeWithText("Discard", substring = false).performClick()
        compose.onNodeWithText("Android tester", substring = false).assertIsDisplayed()
        compose.onNodeWithText("Profile, presence and security").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("account-status").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("account-settings").performScrollToNode(hasTestTag("account-status"))
        compose.onNodeWithTag("account-status").assertTextContains("Building Papo")
        compose.onNodeWithTag("account-settings").performScrollToNode(hasTestTag("account-typing"))
        compose.onNodeWithTag("account-typing").assertTextContains("is making things…")
    }

    @Test
    fun conversationAppearanceControlsAffectMessagesAndSurviveRecreation() {
        demo()
        compose.onNodeWithContentDescription("Client settings").performClick()
        compose.onNodeWithText("Appearance", substring = false).performClick()
        compose
            .onNodeWithTag("appearance-settings")
            .performScrollToNode(hasContentDescription("Show avatars"))
        compose.onNodeWithContentDescription("Show avatars").performClick()
        compose
            .onNodeWithTag("appearance-settings")
            .performScrollToNode(hasContentDescription("Show timestamps"))
        compose.onNodeWithContentDescription("Show timestamps").performClick()
        compose
            .onNodeWithTag("appearance-settings")
            .performScrollToNode(hasContentDescription("Animate GIFs and stickers"))
        compose.onNodeWithContentDescription("Animate GIFs and stickers").performClick()
        activity.recreate()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("appearance-settings").fetchSemanticsNodes().isNotEmpty()
        }
        compose
            .onNodeWithTag("appearance-settings")
            .performScrollToNode(hasContentDescription("Show avatars"))
        compose.onNodeWithContentDescription("Show avatars").assertIsOff()
        compose
            .onNodeWithTag("appearance-settings")
            .performScrollToNode(hasContentDescription("Show timestamps"))
        compose.onNodeWithContentDescription("Show timestamps").assertIsOff()
        compose
            .onNodeWithTag("appearance-settings")
            .performScrollToNode(hasContentDescription("Animate GIFs and stickers"))
        compose.onNodeWithContentDescription("Animate GIFs and stickers").assertIsOff()
        systemBack()
        systemBack()
        compose.onNodeWithText("geral", substring = false).performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("message-list").fetchSemanticsNodes().isNotEmpty()
        }
        compose
            .onNode(
                hasTestTag("avatar-ana") and hasAnyAncestor(hasTestTag("message-list")),
                useUnmergedTree = true,
            )
            .assertDoesNotExist()
        compose.onNodeWithTag("author-general-9", useUnmergedTree = true).assertExists()
    }

    private fun media() {
        demo()
        compose.onNodeWithText("mídia").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(hasTestTag("channel-header") and hasText("mídia"))
                .fetchSemanticsNodes()
                .isNotEmpty() &&
                compose.onAllNodesWithTag("message-list").fetchSemanticsNodes().isNotEmpty()
        }
    }

    @Test
    fun attachmentRollupPicksAndSendsAFileThroughTheNativeCore() {
        chat()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val file =
            AttachmentFiles.create(context, "txt").apply { writeText("Compose attachment fixture") }
        val uri =
            androidx.core.content.FileProvider.getUriForFile(
                context,
                "${context.packageName}.files",
                file,
            )
        val filter =
            android.content.IntentFilter(android.content.Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(android.content.Intent.CATEGORY_OPENABLE)
                addDataType("*/*")
            }
        val monitor =
            instrumentation.addMonitor(
                filter,
                android.app.Instrumentation.ActivityResult(
                    android.app.Activity.RESULT_OK,
                    android.content
                        .Intent()
                        .setData(uri)
                        .addFlags(android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION),
                ),
                true,
            )
        try {
            compose.onNodeWithContentDescription("Open gallery").performTouchInput { longClick() }
            compose.onNodeWithText("Record audio").assertIsDisplayed()
            compose.onNodeWithText("Camera").assertIsDisplayed()
            compose.onNodeWithText("Choose files").assertIsDisplayed().performClick()
            compose.waitUntil(10_000) {
                compose
                    .onAllNodesWithContentDescription("Remove ${file.name}")
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            compose.onNodeWithContentDescription("Send message").assertIsEnabled().performClick()
            compose.waitUntil(10_000) {
                compose
                    .onAllNodesWithContentDescription("Open ${file.name}")
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            compose.onNodeWithContentDescription("Remove ${file.name}").assertDoesNotExist()
            compose.onNodeWithContentDescription("Send message").assertIsNotEnabled()
        } finally {
            instrumentation.removeMonitor(monitor)
            file.delete()
        }
    }

    @Test
    fun tappingTheGalleryButtonOpensThePhotoPickerWithoutOpeningTheRollup() {
        chat()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val file = AttachmentFiles.create(context, "jpg")
        val bitmap =
            android.graphics.Bitmap.createBitmap(24, 24, android.graphics.Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(android.graphics.Color.GREEN)
        file.outputStream().use {
            bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 90, it)
        }
        bitmap.recycle()
        val uri =
            androidx.core.content.FileProvider.getUriForFile(
                context,
                "${context.packageName}.files",
                file,
            )
        var galleryOpened = false
        val monitor =
            object : android.app.Instrumentation.ActivityMonitor() {
                override fun onStartActivity(
                    intent: android.content.Intent
                ): android.app.Instrumentation.ActivityResult? {
                    if (
                        intent.action !in
                            listOf(
                                "android.provider.action.PICK_IMAGES",
                                android.content.Intent.ACTION_OPEN_DOCUMENT,
                                android.content.Intent.ACTION_GET_CONTENT,
                            )
                    )
                        return null
                    galleryOpened = true
                    return android.app.Instrumentation.ActivityResult(
                        android.app.Activity.RESULT_OK,
                        android.content.Intent().setData(uri),
                    )
                }
            }
        instrumentation.addMonitor(monitor)
        try {
            compose.onNodeWithContentDescription("Open gallery").performClick()
            compose.waitUntil(10_000) {
                compose
                    .onAllNodesWithContentDescription("Remove ${file.name}")
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            assertTrue(galleryOpened)
            compose.onNodeWithText("Choose files").assertDoesNotExist()
            compose.onNodeWithContentDescription("Remove ${file.name}").performClick()
        } finally {
            instrumentation.removeMonitor(monitor)
            file.delete()
        }
    }

    @Test
    fun cameraCaptureStagesAPhotoWithoutSendingIt() {
        chat()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val monitor =
            object : android.app.Instrumentation.ActivityMonitor() {
                @Suppress("DEPRECATION")
                override fun onStartActivity(
                    intent: android.content.Intent
                ): android.app.Instrumentation.ActivityResult? {
                    if (intent.action != android.provider.MediaStore.ACTION_IMAGE_CAPTURE)
                        return null
                    val uri =
                        intent.getParcelableExtra<android.net.Uri>(
                            android.provider.MediaStore.EXTRA_OUTPUT
                        )!!
                    val bitmap =
                        android.graphics.Bitmap.createBitmap(
                            24,
                            24,
                            android.graphics.Bitmap.Config.ARGB_8888,
                        )
                    bitmap.eraseColor(android.graphics.Color.BLUE)
                    context.contentResolver.openOutputStream(uri)!!.use {
                        bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 90, it)
                    }
                    bitmap.recycle()
                    return android.app.Instrumentation.ActivityResult(
                        android.app.Activity.RESULT_OK,
                        null,
                    )
                }
            }
        instrumentation.addMonitor(monitor)
        try {
            compose.onNodeWithContentDescription("Open gallery").performTouchInput { longClick() }
            compose.onNodeWithText("Camera").performClick()
            compose.waitUntil(10_000) {
                compose
                    .onAllNodesWithContentDescription("Remove Photo ", substring = true)
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            compose.onNodeWithContentDescription("Send message").assertIsEnabled()
            compose.onNodeWithContentDescription("Remove Photo ", substring = true).performClick()
            compose.waitUntil(5000) {
                compose
                    .onAllNodesWithContentDescription("Remove Photo ", substring = true)
                    .fetchSemanticsNodes()
                    .isEmpty()
            }
            compose.onNodeWithContentDescription("Send message").assertIsNotEnabled()
        } finally {
            instrumentation.removeMonitor(monitor)
        }
    }

    @Test
    fun recordsAudioAttachesItAndPlaysTheDemoMessage() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.grantRuntimePermission(
            instrumentation.targetContext.packageName,
            android.Manifest.permission.RECORD_AUDIO,
        )
        chat()
        compose.onNodeWithContentDescription("Open gallery").performTouchInput { longClick() }
        compose.onNodeWithText("Record audio").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("Send recording")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Live microphone waveform").assertIsDisplayed()
        android.os.SystemClock.sleep(1200)
        compose.onNodeWithContentDescription("Send recording").assertIsEnabled().performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("Play Voice note ", substring = true)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Play Voice note ", substring = true).performClick()
        compose.waitUntil(15_000) {
            compose
                .onAllNodes(
                    SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready")
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
    }

    @Test
    fun rotatingDuringRecordingKeepsTheVoiceNoteAndCaptionWithoutSending() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.grantRuntimePermission(
            instrumentation.targetContext.packageName,
            android.Manifest.permission.RECORD_AUDIO,
        )
        chat()
        compose.onNode(hasSetTextAction()).performTextInput("Keep this caption")
        compose.onNodeWithContentDescription("Open gallery").performTouchInput { longClick() }
        compose.onNodeWithText("Record audio").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("Send recording")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        android.os.SystemClock.sleep(1200)
        activity.recreate()
        compose.waitUntil(10_000) {
            compose
                .onAllNodesWithContentDescription("Remove Voice note ", substring = true)
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNode(hasSetTextAction()).assertTextEquals("Keep this caption")
        compose
            .onNodeWithContentDescription("Play Voice note ", substring = true)
            .assertDoesNotExist()
        compose.onNodeWithContentDescription("Remove Voice note ", substring = true).performClick()
    }

    @Test
    fun backendMislabeledM4aUsesAudioControlsAndKeepsItsCompactFrame() {
        media()
        compose.onNodeWithTag("message-list").performScrollToKey("media-4")
        val frame =
            compose
                .onNodeWithTag("audio-attachment-demo-audio", useUnmergedTree = true)
                .fetchSemanticsNode()
                .size
        compose.onNodeWithTag("attachment-demo-audio", useUnmergedTree = true).assertDoesNotExist()
        compose.waitUntil(15_000) {
            compose
                .onAllNodes(
                    hasTestTag("waveform-demo://papo/me/attachment/demo-audio") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Waveform ready",
                        ),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Play voice-note.m4a").performClick()
        compose.waitUntil(15_000) {
            compose
                .onAllNodes(
                    hasTestTag("player-demo://papo/me/attachment/demo-audio") and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready"),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.waitUntil(5000) {
            compose
                .onAllNodesWithContentDescription("Pause voice-note.m4a")
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Pause voice-note.m4a").performClick()
        val waveform =
            compose.onNodeWithTag(
                "waveform-demo://papo/me/attachment/demo-audio",
                useUnmergedTree = true,
            )
        waveform.performTouchInput {
            swipe(
                androidx.compose.ui.geometry.Offset(width * .85f, centerY),
                androidx.compose.ui.geometry.Offset(width * .15f, centerY),
                400,
            )
        }
        compose.onNodeWithTag("composer-reply").assertDoesNotExist()
        compose.waitUntil(5000) {
            waveform
                .fetchSemanticsNode()
                .config[SemanticsProperties.ProgressBarRangeInfo]
                .current in 0.05f..0.3f
        }
        compose.onNodeWithTag("audio-controls", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithContentDescription("Audio position").assertExists()
        val loaded =
            compose
                .onNodeWithTag("audio-attachment-demo-audio", useUnmergedTree = true)
                .fetchSemanticsNode()
                .size
        assertEquals(frame.height, loaded.height)
    }

    @Test
    fun portraitEmbedReservesServerDimensionsAndOpensViewer() {
        media()
        compose
            .onNodeWithTag("message-list", useUnmergedTree = true)
            .performScrollToNode(hasTestTag("embed-embed-portrait"))
        val frame =
            compose
                .onNodeWithTag("embed-embed-portrait", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
        assertEquals(540f / 960f, frame.width / frame.height, 0.01f)
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasContentDescription("Portrait preview") and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Loaded")
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        val loaded =
            compose
                .onNodeWithTag("embed-embed-portrait", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
        assertEquals(frame.height, loaded.height, 1f)
        compose.onNodeWithContentDescription("Portrait preview").performClick()
        compose.onNodeWithContentDescription("Close image").assertExists().performClick()
    }

    @Test
    fun videoAttachmentDecodesAndKeepsItsReservedFrame() {
        media()
        var previous = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
        activity.onActivity { previous = it.requestedOrientation }
        compose.onNodeWithTag("message-list").performScrollToKey("media-3")
        // Use actual geometry, not a fling based on the old message spacing:
        // scrolling the video offscreen correctly deactivates its player.
        val initial =
            compose
                .onNodeWithTag("attachment-demo-video", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
        val header = compose.onNodeWithTag("channel-header").fetchSemanticsNode().boundsInRoot
        val composer = compose.onNodeWithTag("chat-composer").fetchSemanticsNode().boundsInRoot
        compose.onNodeWithTag("message-list").performSemanticsAction(
            androidx.compose.ui.semantics.SemanticsActions.ScrollBy
        ) {
            it(0f, (header.bottom + composer.top) / 2 - initial.center.y)
        }
        compose.waitForIdle()
        val frame =
            compose
                .onNodeWithTag("attachment-demo-video", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
        assertTrue(
            "Keep the video inside the unobscured viewport: initial=$initial moved=$frame usable=${header.bottom}..${composer.top}",
            frame.top >= header.bottom && frame.bottom <= composer.top,
        )
        compose.onNodeWithContentDescription("Play clip.mp4").assertIsDisplayed().performClick()
        val player = "player-demo://papo/me/attachment/demo-video"
        try {
            compose.waitUntil(15_000) {
                compose
                    .onAllNodes(
                        hasTestTag(player) and
                            SemanticsMatcher.expectValue(
                                SemanticsProperties.StateDescription,
                                "Ready",
                            ),
                        useUnmergedTree = true,
                    )
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
        } catch (failure: Throwable) {
            val cache = InstrumentationRegistry.getInstrumentation().targetContext.cacheDir
            cache
                .resolve("video-failure-state.txt")
                .writeText(compose.onRoot(useUnmergedTree = true).printToString())
            val image = compose.onRoot().captureToImage().asAndroidBitmap()
            cache.resolve("video-failure.png").outputStream().use {
                image.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
            }
            throw failure
        }
        val playing =
            compose
                .onNodeWithTag("attachment-demo-video", useUnmergedTree = true)
                .fetchSemanticsNode()
                .boundsInRoot
        assertEquals(frame.height, playing.height, 1f)
        compose.onNodeWithTag("media-viewer").assertDoesNotExist()
        activity.onActivity {
            assertEquals(
                "Inline playback respects the normal rotation policy",
                previous,
                it.requestedOrientation,
            )
        }
        var originalPlayer: androidx.media3.common.Player? = null
        var originalView: androidx.media3.ui.PlayerView? = null
        androidx.test.espresso.Espresso.onView(
                androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(
                    androidx.media3.ui.PlayerView::class.java
                )
            )
            .check { view, _ ->
                val native = view as androidx.media3.ui.PlayerView
                originalView = native
                originalPlayer = native.player
                originalPlayer!!.pause()
                originalPlayer!!.seekTo(300)
                native.showController()
            }
        val fullscreenLabel =
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .getString(androidx.media3.ui.R.string.exo_controls_fullscreen_enter_description)
        androidx.test.espresso.Espresso.onView(
                org.hamcrest.Matchers.allOf(
                    androidx.test.espresso.matcher.ViewMatchers.withContentDescription(
                        fullscreenLabel
                    ),
                    androidx.test.espresso.matcher.ViewMatchers.isDisplayed(),
                )
            )
            .perform(androidx.test.espresso.action.ViewActions.click())
        try {
            compose.waitUntil(10_000) {
                runCatching {
                        compose.onNodeWithTag("media-viewer").assertIsDisplayed()
                        true
                    }
                    .getOrDefault(false)
            }
        } catch (e: Exception) {
            android.util.Log.e(
                "PapoViewerTest",
                compose.onAllNodesWithTag("media-viewer").fetchSemanticsNodes().joinToString {
                    "size=${it.size}, bounds=${it.boundsInRoot}, ${it.config}"
                },
            )
            InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()?.let { bitmap
                ->
                InstrumentationRegistry.getInstrumentation()
                    .targetContext
                    .cacheDir
                    .resolve("video-viewer-failure.png")
                    .outputStream()
                    .use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
            }
            throw e
        }
        compose.onNodeWithTag("media-viewer").assertIsDisplayed()
        compose.onNodeWithContentDescription("Download media").assertIsDisplayed()
        activity.onActivity {
            assertEquals(
                android.content.pm.ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR,
                it.requestedOrientation,
            )
        }
        try {
            activity.onActivity {
                it.requestedOrientation =
                    android.content.pm.ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE
            }
            compose.waitUntil(10_000) {
                compose.onAllNodesWithTag("media-viewer").fetchSemanticsNodes().any {
                    it.boundsInRoot.width > it.boundsInRoot.height
                }
            }
            activity.onActivity {
                assertEquals(
                    "Per-app language must not freeze the Activity's orientation configuration",
                    android.content.res.Configuration.ORIENTATION_LANDSCAPE,
                    it.resources.configuration.orientation,
                )
            }
            // Window dimensions update before Android's rotation animation
            // finishes. Native touch coordinates are usable once it settles.
            InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(750, 5000)
            compose
                .onNode(
                    hasTestTag(player) and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready"),
                    useUnmergedTree = true,
                )
                .assertIsDisplayed()
            androidx.test.espresso.Espresso.onView(
                    androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(
                        androidx.media3.ui.PlayerView::class.java
                    )
                )
                .check { view, _ ->
                    assertTrue(
                        "Fullscreen moves the original PlayerView and its controls",
                        view === originalView,
                    )
                    assertTrue(
                        "Fullscreen keeps the existing player",
                        (view as androidx.media3.ui.PlayerView).player === originalPlayer,
                    )
                    assertEquals(300L, originalPlayer!!.currentPosition)
                    assertTrue("Fullscreen keeps the paused state", !originalPlayer!!.playWhenReady)
                }
            pinchViewer()
            val exitLabel =
                InstrumentationRegistry.getInstrumentation()
                    .targetContext
                    .getString(androidx.media3.ui.R.string.exo_controls_fullscreen_exit_description)
            androidx.test.espresso.Espresso.onView(
                    org.hamcrest.Matchers.allOf(
                        androidx.test.espresso.matcher.ViewMatchers.withContentDescription(
                            exitLabel
                        ),
                        androidx.test.espresso.matcher.ViewMatchers.isDisplayed(),
                    )
                )
                .perform(androidx.test.espresso.action.ViewActions.click())
            compose.waitUntil(10_000) {
                compose.onAllNodesWithTag("media-viewer").fetchSemanticsNodes().isEmpty()
            }
            compose.onNodeWithTag("media-viewer").assertDoesNotExist()
            activity.onActivity {
                assertEquals(
                    "Closing restores the normal orientation policy",
                    previous,
                    it.requestedOrientation,
                )
            }
            androidx.test.espresso.Espresso.onView(
                    androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(
                        androidx.media3.ui.PlayerView::class.java
                    )
                )
                .check { view, _ ->
                    assertTrue(
                        "Returning inline restores the original PlayerView and controls",
                        view === originalView,
                    )
                    assertTrue(
                        "Returning inline keeps the existing player",
                        (view as androidx.media3.ui.PlayerView).player === originalPlayer,
                    )
                }
        } finally {
            activity.onActivity { it.requestedOrientation = previous }
            InstrumentationRegistry.getInstrumentation().uiAutomation.waitForIdle(750, 5000)
        }
    }

    @Test
    fun videoPausesOffscreenReleasesFarAwayAndRestoresItsPosition() {
        media()
        compose.onNodeWithTag("message-list").performScrollToKey("media-3")
        compose.onNodeWithContentDescription("Play clip.mp4").performClick()
        val tag = "player-demo://papo/me/attachment/demo-video"
        compose.waitUntil(15_000) {
            compose
                .onAllNodes(
                    hasTestTag(tag) and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready"),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        var original: androidx.media3.common.Player? = null
        androidx.test.espresso.Espresso.onView(
                androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(
                    androidx.media3.ui.PlayerView::class.java
                )
            )
            .check { view, _ ->
                original = (view as androidx.media3.ui.PlayerView).player
                original!!.seekTo(300)
                original!!.play()
            }
        compose.onNodeWithTag("message-list").performScrollToKey("media-6")
        fun onMain(condition: () -> Boolean): Boolean {
            var result = false
            InstrumentationRegistry.getInstrumentation().runOnMainSync { result = condition() }
            return result
        }
        compose.waitUntil(5000) { onMain { !original!!.playWhenReady } }
        assertTrue(
            "Nearby offscreen playback stays allocated",
            onMain { original!!.playbackState != androidx.media3.common.Player.STATE_IDLE },
        )
        var position = 0L
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            position = original!!.currentPosition
        }
        compose.onNodeWithTag("message-list").performScrollToKey("media-11")
        compose.waitUntil(10_000) {
            onMain { original!!.playbackState == androidx.media3.common.Player.STATE_IDLE }
        }
        compose.onNodeWithTag("message-list").performScrollToKey("media-3")
        compose.waitUntil(15_000) {
            compose
                .onAllNodes(
                    hasTestTag(tag) and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ready"),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        androidx.test.espresso.Espresso.onView(
                androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom(
                    androidx.media3.ui.PlayerView::class.java
                )
            )
            .check { view, _ ->
                val restored = (view as androidx.media3.ui.PlayerView).player!!
                assertTrue("Far-away media recreates its released player", restored !== original)
                assertTrue(
                    "The recreated player retains its position",
                    restored.currentPosition >= position,
                )
            }
    }

    @Test
    fun animatedStickersAndGifsStopOffscreenWithoutStoppingOtherOccurrences() {
        chat()
        fun send(text: String) {
            compose.onNodeWithTag("composer-input").performTextInput(text)
            compose.onNodeWithContentDescription("Send message").performClick()
            compose.waitUntil(10_000) {
                compose
                    .onNodeWithTag("composer-input")
                    .fetchSemanticsNode()
                    .config[SemanticsProperties.InputText]
                    .text
                    .isEmpty()
            }
        }
        send(":papo_wave:")
        fun probes() =
            compose
                .onAllNodes(
                    SemanticsMatcher.keyIsDefined(MediaAnimationRunning) and
                        hasAnyAncestor(hasTestTag("message-sticker-demo-wave")),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .map { it.config[MediaAnimationRunning] }
        compose.waitUntil(10_000) { probes().any { it() } }
        val first = probes().single()
        send((1..23).joinToString("\n") { "A little space $it" })
        send(":papo_wave:")
        compose.waitUntil(10_000) { probes().any { it() } }
        compose.waitUntil(5000) { !first() }
        assertTrue(
            "The visible copy of the same animated sticker keeps running",
            probes().any { it() },
        )
        compose.onNodeWithContentDescription("Back to conversations").performClick()
        compose.onNodeWithText("mídia").performClick()
        compose.waitUntil(10_000) {
            compose.onAllNodesWithTag("message-list").fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithTag("message-list").performScrollToKey("media-2")
        fun gifProbe() =
            compose
                .onAllNodes(
                    SemanticsMatcher.keyIsDefined(MediaAnimationRunning) and
                        hasAnyAncestor(hasTestTag("attachment-demo-gif")),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .firstOrNull()
                ?.config
                ?.get(MediaAnimationRunning)
        compose.waitUntil(10_000) { gifProbe()?.invoke() == true }
        val gif = gifProbe()!!
        compose.onNodeWithTag("message-list").performScrollToKey("media-7")
        compose.waitUntil(5000) { !gif() }
    }

    private fun pinchViewer() {
        compose.onNodeWithTag("viewer-content", useUnmergedTree = true).performTouchInput {
            down(0, center.copy(x = center.x - 30f))
            down(1, center.copy(x = center.x + 30f))
            moveTo(0, center.copy(x = center.x - 90f))
            moveTo(1, center.copy(x = center.x + 90f))
            up(0)
            up(1)
        }
        compose.waitUntil(5000) {
            compose
                .onAllNodesWithTag("viewer-content", useUnmergedTree = true)
                .fetchSemanticsNodes()
                .any { it.config[SemanticsProperties.StateDescription] != "Zoom 1.00x" }
        }
    }

    @Test
    fun directMediaHidesItsUrlAndHoldingItCopiesEachLinkSeparately() {
        media()
        compose
            .onNodeWithTag("message-list", useUnmergedTree = true)
            .performScrollToNode(hasTestTag("embed-embed-portrait"))
        val image = compose.onNodeWithContentDescription("Portrait preview")
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasContentDescription("Portrait preview") and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Loaded")
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose
            .onNodeWithText("As dimensões chegam antes dos pixels.", substring = true)
            .assertIsDisplayed()
        compose
            .onNodeWithText("https://cdn.example.test/portrait.png?sig=fixture", substring = true)
            .assertDoesNotExist()
        image.performTouchInput { longClick() }
        val clipboard =
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .getSystemService(android.content.ClipboardManager::class.java)
        for (url in
            listOf(
                "https://cdn.example.test/portrait.png?sig=fixture",
                "https://example.test/more",
            )) {
            compose.onNodeWithContentDescription("Copy URL: $url").performScrollTo().performClick()
            assertEquals(url, clipboard.primaryClip?.getItemAt(0)?.text.toString())
        }
        compose.onNodeWithText("Copy text", substring = false).performScrollTo().performClick()
        assertTrue(clipboard.primaryClip?.getItemAt(0)?.text.toString().contains("As dimensões"))
        assertTrue(
            clipboard.primaryClip
                ?.getItemAt(0)
                ?.text
                .toString()
                .contains("https://cdn.example.test/portrait.png?sig=fixture")
        )
    }

    @Test
    fun directUrlRendersAndOpensBeforeBackendEmbedMetadataArrives() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val bytes = context.assets.open("demo/portrait.png").use { it.readBytes() }
        val socket = java.net.ServerSocket(0)
        val headers = java.util.concurrent.ConcurrentLinkedQueue<String>()
        val executor = java.util.concurrent.Executors.newSingleThreadExecutor()
        executor.submit {
            while (!socket.isClosed) try {
                socket.accept().use { connection ->
                    val reader = connection.getInputStream().bufferedReader()
                    while (true) {
                        val line = reader.readLine() ?: break
                        if (line.isEmpty()) break
                        headers.add(line)
                    }
                    connection.getOutputStream().apply {
                        write(
                            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: ${bytes.size}\r\nConnection: close\r\n\r\n"
                                .toByteArray()
                        )
                        write(bytes)
                        flush()
                    }
                }
            } catch (_: java.io.IOException) {
                if (socket.isClosed) break
            }
        }
        try {
            chat()
            val url = "http://127.0.0.1:${socket.localPort}/fixture.PNG?sig=exact%2Bvalue"
            compose.onNodeWithTag("composer-input").performTextInput("Keep this caption\n$url")
            compose.onNodeWithContentDescription("Send message").performClick()
            compose.waitUntil(10_000) {
                compose
                    .onAllNodes(
                        hasContentDescription("fixture.PNG") and
                            SemanticsMatcher.expectValue(
                                SemanticsProperties.StateDescription,
                                "Loaded",
                            )
                    )
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            compose.onNodeWithText("Keep this caption").assertIsDisplayed()
            compose.onNodeWithText(url, substring = true).assertDoesNotExist()
            compose.onNodeWithContentDescription("fixture.PNG").performClick()
            compose.waitUntil(10_000) {
                compose
                    .onAllNodes(
                        hasTestTag("viewer-image") and
                            SemanticsMatcher.expectValue(
                                SemanticsProperties.StateDescription,
                                "Loaded",
                            ),
                        useUnmergedTree = true,
                    )
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            assertTrue(headers.any { it.contains("sig=exact%2Bvalue") })
            assertTrue(headers.none { it.startsWith("Cookie:", ignoreCase = true) })
            compose.onNodeWithContentDescription("Close image").performClick()
        } finally {
            socket.close()
            executor.shutdownNow()
        }
    }

    @Test
    fun sharedImageViewerZoomsDownloadsAndDismissesOnlyOutsideItsContent() {
        media()
        compose
            .onNodeWithTag("message-list", useUnmergedTree = true)
            .performScrollToNode(hasTestTag("embed-embed-portrait"))
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasContentDescription("Portrait preview") and
                        SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Loaded")
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithContentDescription("Portrait preview").performClick()
        compose.waitUntil(10_000) {
            compose
                .onAllNodes(
                    hasTestTag("viewer-image") and
                        SemanticsMatcher.expectValue(
                            SemanticsProperties.StateDescription,
                            "Loaded",
                        ),
                    useUnmergedTree = true,
                )
                .fetchSemanticsNodes()
                .isNotEmpty()
        }
        compose.onNodeWithTag("viewer-content").performClick()
        compose.onNodeWithTag("media-viewer").assertIsDisplayed()
        pinchViewer()
        val reset =
            compose
                .onNodeWithTag("viewer-content")
                .fetchSemanticsNode()
                .config[androidx.compose.ui.semantics.SemanticsActions.CustomActions]
                .single { it.label == "Reset zoom" }
        compose.runOnIdle { assertTrue(reset.action()) }
        val preview = compose.onNodeWithTag("media-viewer").captureToImage().asAndroidBitmap()
        InstrumentationRegistry.getInstrumentation()
            .targetContext
            .cacheDir
            .resolve("media-viewer-verified.png")
            .outputStream()
            .use { preview.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val downloads = android.provider.MediaStore.Downloads.EXTERNAL_CONTENT_URI
        fun ids(): Set<Long> =
            context.contentResolver
                .query(
                    downloads,
                    arrayOf(android.provider.MediaStore.MediaColumns._ID),
                    "${android.provider.MediaStore.MediaColumns.DISPLAY_NAME} LIKE ?",
                    arrayOf("Portrait preview%"),
                    null,
                )!!
                .use { cursor -> buildSet { while (cursor.moveToNext()) add(cursor.getLong(0)) } }
        val before = ids()
        try {
            compose.onNodeWithContentDescription("Download media").performClick()
            compose.waitUntil(10_000) {
                compose
                    .onAllNodesWithContentDescription("Downloaded")
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            assertTrue(ids().size > before.size)
        } finally {
            (ids() - before).forEach { id ->
                context.contentResolver.delete(
                    android.content.ContentUris.withAppendedId(downloads, id),
                    null,
                    null,
                )
            }
        }
        val content = compose.onNodeWithTag("viewer-content").fetchSemanticsNode().boundsInRoot
        val backdrop = compose.onNodeWithTag("viewer-backdrop").fetchSemanticsNode().boundsInRoot
        compose.onNodeWithTag("viewer-backdrop").performTouchInput {
            click(androidx.compose.ui.geometry.Offset(8f, content.top - backdrop.top - 12f))
        }
        compose.onNodeWithTag("media-viewer").assertDoesNotExist()
    }

    @Test
    fun languageSelectionRecreatesTheUiAndSearchesTranslatedSettings() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val original = AppLanguage.selected(context)
        try {
            demo()
            compose.onNodeWithContentDescription("Client settings").performClick()
            compose.onNodeWithTag("settings-search").performTextInput("language")
            androidx.test.espresso.Espresso.closeSoftKeyboard()
            compose.onNodeWithText("Language", substring = false).performClick()
            compose.onNodeWithTag("language-pt-BR").performClick()
            compose.waitUntil(10_000) {
                compose
                    .onAllNodesWithText("Idioma", substring = false)
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            compose.onNodeWithTag("language-pt-BR").assertIsSelected()
            systemBack()
            compose.onNodeWithTag("settings-search").performTextReplacement("compactas")
            androidx.test.espresso.Espresso.closeSoftKeyboard()
            compose.onNodeWithText("Mensagens compactas", substring = false).performClick()
            compose.onNodeWithContentDescription("Mensagens compactas").assertIsOff().performClick()
            activity.recreate()
            compose.waitUntil(10_000) {
                compose
                    .onAllNodesWithContentDescription("Mensagens compactas")
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            compose.onNodeWithContentDescription("Mensagens compactas").assertIsOn()
            systemBack()
            systemBack()
            // Channel names remain server content even though the app labels changed language.
            compose.onNodeWithText("geral", substring = false).assertIsDisplayed()
        } finally {
            activity.onActivity { AppLanguage.choose(it, original) }
        }
    }

    @Test
    fun externalLinksAskBeforeOpeningAndTrustOnlyTheSelectedHost() {
        val opened = java.util.concurrent.CopyOnWriteArrayList<String>()
        val prefs = ClientPreferences()
        activity.onActivity { host ->
            host.setContent {
                androidx.compose.material3.MaterialTheme {
                    androidx.compose.runtime.CompositionLocalProvider(
                        LocalClientPreferences provides prefs,
                        androidx.compose.ui.platform.LocalUriHandler provides
                            object : androidx.compose.ui.platform.UriHandler {
                                override fun openUri(uri: String) {
                                    opened.add(uri)
                                }
                            },
                    ) {
                        ExternalLinkHost({}) {
                            val links = androidx.compose.ui.platform.LocalUriHandler.current
                            androidx.compose.foundation.layout.Column {
                                androidx.compose.material3.Button(
                                    onClick = { links.openUri("https://example.com/a?original=1") }
                                ) {
                                    androidx.compose.material3.Text("Visit site")
                                }
                                androidx.compose.material3.Button(
                                    onClick = { links.openUri("https://sub.example.com/a") }
                                ) {
                                    androidx.compose.material3.Text("Visit subdomain")
                                }
                            }
                        }
                    }
                }
            }
        }
        compose.onNodeWithText("Visit site").performClick()
        compose.onNodeWithTag("external-link-confirmation").assertExists()
        assertTrue(opened.isEmpty())
        compose.onNodeWithText("Cancel").performClick()
        assertTrue(opened.isEmpty())
        assertTrue(prefs.trustedDomains.isEmpty())
        compose.onNodeWithText("Visit site").performClick()
        compose.onNodeWithText("Always trust example.com").performClick()
        compose.onNodeWithText("Open link").performClick()
        compose.runOnIdle {
            assertEquals(listOf("https://example.com/a?original=1"), opened.toList())
        }
        compose.onNodeWithText("Visit site").performClick()
        compose.runOnIdle { assertEquals(2, opened.size) }
        compose.onNodeWithTag("external-link-confirmation").assertDoesNotExist()
        compose.onNodeWithText("Visit subdomain").performClick()
        compose.onNodeWithTag("external-link-confirmation").assertExists()
        assertEquals(setOf("example.com"), prefs.trustedDomains)
    }

    @Test
    fun accountRecoveryRequiresMatchingExplicitServerBeforeSendingPassword() {
        activity.onActivity { host ->
            host.setContent {
                androidx.compose.material3.MaterialTheme {
                    androidx.compose.runtime.CompositionLocalProvider(
                        LocalHaze provides dev.chrisbanes.haze.HazeState()
                    ) {
                        AccountRecovery(
                            "papo-reset://passwordchange/test_token?server=https://example.test",
                            "http://127.0.0.1:9",
                            {},
                        )
                    }
                }
            }
        }
        compose.onNodeWithTag("recovery-password").performTextInput("NewPassword!")
        compose.onNodeWithTag("recovery-confirmation").performTextInput("NewPassword!")
        compose.onNodeWithTag("recover-account").performScrollTo().performClick()
        compose.waitUntil(5000) {
            compose.onAllNodesWithTag("recovery-error").fetchSemanticsNodes().isNotEmpty()
        }
        compose
            .onNodeWithTag("recovery-error")
            .assertTextEquals("Select the server this recovery link belongs to")
        compose.onNodeWithTag("account-recovery").assertExists()
    }
}

private fun MainActivity.setContentForMemberHeader(server: ServerState, model: PapoViewModel) {
    setContent {
        androidx.compose.material3.MaterialTheme {
            androidx.compose.runtime.CompositionLocalProvider(
                LocalHaze provides dev.chrisbanes.haze.HazeState(),
                LocalGlass provides false,
            ) {
                ServerHeader(server, model, "")
            }
        }
    }
}
