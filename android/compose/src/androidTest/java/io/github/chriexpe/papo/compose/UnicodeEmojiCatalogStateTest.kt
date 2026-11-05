package io.github.chriexpe.papo.compose

import android.content.Context
import android.os.Looper
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class UnicodeEmojiCatalogStateTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun disabledPeopleSuggestionsNeverStartTheCatalogLoader() {
        val calls = AtomicInteger()
        val query = mutableStateOf("@a")
        val load: (Context) -> List<UnicodeEmoji> = {
            calls.incrementAndGet()
            listOf(UnicodeEmoji("👍", "thumbs up"))
        }
        compose.setContent {
            val catalog by rememberUnicodeEmojiCatalog(enabled = false, load = load)
            Text("${query.value}/${catalog.size}")
        }
        compose.onNodeWithText("@a/0").assertExists()
        compose.runOnIdle { query.value = "@ana" }
        compose.onNodeWithText("@ana/0").assertExists()
        compose.runOnIdle { assertEquals(0, calls.get()) }
    }

    @Test
    fun workerCompletionRefreshesTheCurrentQueryWithoutRestartingTheLoad() {
        val started = CountDownLatch(1)
        val release = CountDownLatch(1)
        val calls = AtomicInteger()
        val ranOnMain = AtomicBoolean(true)
        val applicationContext = AtomicBoolean(false)
        val query = mutableStateOf("no-match")
        val load: (Context) -> List<UnicodeEmoji> = { context ->
            calls.incrementAndGet()
            ranOnMain.set(Looper.myLooper() == Looper.getMainLooper())
            applicationContext.set(context === context.applicationContext)
            started.countDown()
            check(release.await(10, TimeUnit.SECONDS)) { "Test catalog was never released" }
            listOf(UnicodeEmoji("👍", "thumbs up"))
        }
        try {
            compose.setContent {
                val catalog by rememberUnicodeEmojiCatalog(load = load)
                Text("${query.value}/${matchingUnicodeEmoji(catalog, query.value).size}")
            }
            assertTrue(started.await(5, TimeUnit.SECONDS))
            compose.onNodeWithText("no-match/0").assertExists()
            compose.runOnIdle { query.value = "thumbs_up" }
            compose.onNodeWithText("thumbs_up/0").assertExists()
            release.countDown()
            compose.waitUntil(5_000) {
                compose
                    .onAllNodes(androidx.compose.ui.test.hasText("thumbs_up/1"))
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            compose.onNodeWithText("thumbs_up/1").assertTextEquals("thumbs_up/1")
            assertEquals(1, calls.get())
            assertFalse("Asset parsing must not run on Main", ranOnMain.get())
            assertTrue("Loader must not retain an Activity", applicationContext.get())
        } finally {
            release.countDown()
        }
    }

    @Test
    fun disablingWhileLoadingPreventsCancelledPublicationAndCanRestart() {
        val started = CountDownLatch(1)
        val release = CountDownLatch(1)
        val returned = CountDownLatch(1)
        val calls = AtomicInteger()
        val enabled = mutableStateOf(true)
        val load: (Context) -> List<UnicodeEmoji> = {
            calls.incrementAndGet()
            started.countDown()
            check(release.await(10, TimeUnit.SECONDS))
            returned.countDown()
            listOf(UnicodeEmoji("👍", "thumbs up"))
        }
        try {
            compose.setContent {
                val catalog by rememberUnicodeEmojiCatalog(enabled.value, load)
                Text("${enabled.value}/${catalog.size}")
            }
            assertTrue(started.await(5, TimeUnit.SECONDS))
            compose.runOnIdle { enabled.value = false }
            compose.onNodeWithText("false/0").assertExists()
            release.countDown()
            assertTrue(returned.await(5, TimeUnit.SECONDS))
            compose.waitForIdle()
            compose.onNodeWithText("false/0").assertExists()
            compose.runOnIdle { enabled.value = true }
            compose.waitUntil(5_000) {
                compose
                    .onAllNodes(androidx.compose.ui.test.hasText("true/1"))
                    .fetchSemanticsNodes()
                    .isNotEmpty()
            }
            assertEquals(2, calls.get())
        } finally {
            release.countDown()
        }
    }
}
