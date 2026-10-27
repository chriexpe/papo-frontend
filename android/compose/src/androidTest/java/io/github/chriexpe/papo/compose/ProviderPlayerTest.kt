package io.github.chriexpe.papo.compose

import android.content.pm.ActivityInfo
import android.graphics.Color
import android.util.Base64
import android.view.View
import androidx.activity.compose.setContent
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import androidx.test.espresso.Espresso.onView
import androidx.test.espresso.action.GeneralClickAction
import androidx.test.espresso.action.GeneralSwipeAction
import androidx.test.espresso.action.Press
import androidx.test.espresso.action.Swipe
import androidx.test.espresso.action.Tap
import androidx.test.espresso.matcher.ViewMatchers.isAssignableFrom
import androidx.test.platform.app.InstrumentationRegistry
import dev.chrisbanes.haze.HazeState
import dev.chrisbanes.haze.hazeSource
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/** Local HTML/video exercise Chromium without relying on YouTube/network availability. */
class ProviderPlayerTest {
    @get:Rule val compose = createEmptyComposeRule()
    private lateinit var activity: ActivityScenario<MainActivity>
    private lateinit var web: ProviderWebView
    private var replies = 0
    private var actions = 0
    private val visible = mutableStateOf(true)

    @OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
    @Before
    fun launch() {
        activity = ActivityScenario.launch(MainActivity::class.java)
        activity.onActivity { host ->
            host.setContent {
                MaterialTheme {
                    Box(
                        Modifier.fillMaxSize()
                            .hazeSource(remember { HazeState() })
                            .padding(top = 80.dp)
                    ) {
                        if (visible.value)
                            SwipeToReply("provider-fixture", true, { replies++ }) {
                                Box(
                                    Modifier.height(300.dp)
                                        .combinedClickable(
                                            onClick = {},
                                            onLongClick = { actions++ },
                                        )
                                ) {
                                    ProviderPlayer("about:blank") {}
                                }
                            }
                    }
                }
            }
        }
        compose.waitForIdle()
        onView(isAssignableFrom(ProviderWebView::class.java)).check { view, _ ->
            web = view as ProviderWebView
        }
        val clip =
            InstrumentationRegistry.getInstrumentation()
                .targetContext
                .assets
                .open("demo/clip.mp4")
                .use { Base64.encodeToString(it.readBytes(), Base64.NO_WRAP) }
        activity.onActivity {
            web.loadDataWithBaseURL(
                "https://provider.example/",
                """
                <!doctype html><meta name="viewport" content="width=device-width, initial-scale=1">
                <style>html,body{margin:0;width:100%;height:100%;background:#000}
                video{position:absolute;top:0;left:0;width:100%;height:75vh;object-fit:fill}
                input{position:absolute;left:0;bottom:0;width:70%;height:24%}
                button{position:absolute;right:0;bottom:0;width:25%;height:24%}</style>
                <video id="video" loop playsinline src="data:video/mp4;base64,$clip"></video>
                <input id="seek" type="range" min="0" max="100" value="0" oninput="video.currentTime=this.value/100*video.duration">
                <button onclick="video.play();video.requestFullscreen()">Fullscreen</button>
                <script>video.onclick=()=>video.paused?video.play():video.pause();</script>
            """
                    .trimIndent(),
                "text/html",
                "UTF-8",
                null,
            )
        }
        compose.waitUntil(10_000) {
            js("document.querySelector('video')?.readyState >= 2") == "true"
        }
    }

    @After
    fun close() {
        activity.close()
    }

    private fun js(expression: String): String {
        val done = CountDownLatch(1)
        var result = ""
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            web.evaluateJavascript(expression) {
                result = it
                done.countDown()
            }
        }
        check(done.await(5, TimeUnit.SECONDS)) { "Provider JavaScript did not respond" }
        return result
    }

    private fun point(x: Float, y: Float): (View) -> FloatArray = { view ->
        val location = IntArray(2)
        view.getLocationOnScreen(location)
        floatArrayOf(location[0] + view.width * x, location[1] + view.height * y)
    }

    private fun tap(x: Float, y: Float) {
        onView(isAssignableFrom(ProviderWebView::class.java))
            .perform(GeneralClickAction(Tap.SINGLE, point(x, y), Press.FINGER))
    }

    @Test
    fun videoRendersInTimelineAndSeekGestureDoesNotReplyOrOpenActions() {
        assertEquals("true", js("video.getBoundingClientRect().height > innerHeight * .7"))
        tap(.5f, .35f)
        compose.waitUntil(10_000) {
            js("video.webkitDecodedFrameCount > 2 && !video.paused") == "true"
        }
        val location = IntArray(2)
        activity.onActivity { web.getLocationOnScreen(location) }
        val screenshot = InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
        var colored = 0
        for (y in 1..8) for (x in 1..8) {
            val pixel =
                screenshot.getPixel(
                    location[0] + web.width * x / 10,
                    location[1] + web.height * y / 12,
                )
            if (
                maxOf(Color.red(pixel), Color.green(pixel), Color.blue(pixel)) -
                    minOf(Color.red(pixel), Color.green(pixel), Color.blue(pixel)) > 40
            )
                colored++
        }
        screenshot.recycle()
        assertTrue("Decoded video must be visible, not a blank black/white frame", colored > 10)
        tap(.5f, .35f)
        onView(isAssignableFrom(ProviderWebView::class.java))
            .perform(
                GeneralSwipeAction(Swipe.SLOW, point(.60f, .88f), point(.10f, .88f), Press.FINGER)
            )
        assertTrue(js("Number(document.querySelector('input').value)").toDouble() < 40)
        assertEquals(0, replies)
        assertEquals(0, actions)
    }

    @Test
    fun fullscreenBackAndDisposalRestoreTheInlineSessionAndOrientation() {
        var previous = 0
        activity.onActivity { previous = it.requestedOrientation }
        tap(.87f, .88f)
        compose.waitUntil(10_000) { js("!!document.fullscreenElement") == "true" }
        lateinit var chrome: ProviderFullscreen
        activity.onActivity {
            chrome = web.webChromeClient as ProviderFullscreen
            assertTrue(chrome.showing)
            assertEquals(ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR, it.requestedOrientation)
            it.onBackPressedDispatcher.onBackPressed()
        }
        compose.waitUntil(5_000) { js("!document.fullscreenElement") == "true" }
        activity.onActivity {
            assertFalse(chrome.showing)
            assertEquals(previous, it.requestedOrientation)
            assertTrue(web.isAttachedToWindow)
        }
        assertEquals("true", js("!video.paused && video.webkitDecodedFrameCount > 0"))
        tap(.87f, .88f)
        compose.waitUntil(5_000) { js("!!document.fullscreenElement") == "true" }
        compose.runOnIdle { visible.value = false }
        compose.waitForIdle()
        activity.onActivity {
            assertFalse(chrome.showing)
            assertEquals(previous, it.requestedOrientation)
        }
    }
}
