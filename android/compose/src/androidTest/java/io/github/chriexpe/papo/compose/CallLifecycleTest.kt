package io.github.chriexpe.papo.compose

import android.accessibilityservice.AccessibilityService
import android.app.NotificationManager
import android.content.Intent
import android.media.AudioManager
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.Action
import java.net.URL
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/** Process-owned call + Android foreground service/PiP, using only the local synthetic SFU. */
@RunWith(AndroidJUnit4::class)
class CallLifecycleTest {
    @Test
    fun callSurvivesBrowsingRecreationAndPipWithScopedNotificationControls() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val model = (context.applicationContext as PapoApplication).model
        val endpoint = "http://10.0.2.2:18765"
        val previousServer = model.state.value.activeUrl
        assumeTrue(model.state.value.servers.none { it.url == endpoint })
        assumeTrue(model.voiceCalls.state.value.session?.demo != false)
        lateinit var scenario: ActivityScenario<MainActivity>
        scenario = ActivityScenario.launch(MainActivity::class.java)
        try {
            main { model.voiceCalls.state.value.session?.let(model.voiceCalls::leave) }
            runBlocking { model.addServer(endpoint).join() }
            runBlocking { model.command(Action.Login("tester", "fixture-only"), endpoint).join() }
            waitFor("local login") {
                model.state.value.servers.any {
                    it.url == endpoint &&
                        it.screen == "chat" &&
                        it.channels.any { channel -> channel.id == "voice" }
                }
            }
            val server = model.state.value.servers.first { it.url == endpoint }
            val route = CallRoute(server.url, server.me, "voice")
            main { model.voiceCalls.join(route, model.state.value.servers) }
            waitFor("live call") { model.voiceCalls.state.value.session?.connected == true }
            val session = model.voiceCalls.state.value.session!!
            assertFalse(session.demo)
            assertNotNull(VoiceCallService.active)
            assertTrue(
                context.getSystemService(NotificationManager::class.java).activeNotifications.any {
                    it.id == 125
                }
            )
            assertEquals(
                AudioManager.MODE_IN_COMMUNICATION,
                context.getSystemService(AudioManager::class.java).mode,
            )
            main { model.voiceCalls.minimize() }
            runBlocking { model.exploreDemo().join() }
            waitFor("browse another server") { model.state.value.activeUrl == "demo://papo" }
            assertEquals(session.sessionId, model.voiceCalls.state.value.session!!.sessionId)
            scenario.recreate()
            assertEquals(session.sessionId, model.voiceCalls.state.value.session!!.sessionId)
            assertTrue(model.voiceCalls.state.value.session!!.connected)
            main { model.voiceCalls.toggleCamera(model.voiceCalls.state.value.session!!) }
            waitFor("foreground camera") { model.voiceCalls.state.value.session?.camera == true }
            instrumentation.uiAutomation.performGlobalAction(
                AccessibilityService.GLOBAL_ACTION_HOME
            )
            var inPip = false
            waitFor("system PiP") {
                scenario.onActivity { inPip = it.isInPictureInPictureMode }
                inPip
            }
            assertTrue(model.voiceCalls.state.value.session!!.connected)
            main {
                VoiceCallService.active!!.control(model.voiceCalls.state.value.session!!, false)
                    .send()
            }
            waitFor("notification unmutes") { model.voiceCalls.state.value.session?.muted == false }
            val oldLeave =
                VoiceCallService.active!!.control(model.voiceCalls.state.value.session!!, true)
            context.startActivity(
                Intent(context, MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
                    .putExtra("openCall", true)
            )
            waitFor("notification opens call") { model.voiceCalls.state.value.screen == route }
            main { oldLeave.send() }
            waitFor("leave stops foreground service") {
                model.voiceCalls.state.value.session == null && VoiceCallService.active == null
            }
            assertEquals(
                AudioManager.MODE_NORMAL,
                context.getSystemService(AudioManager::class.java).mode,
            )
            main { model.voiceCalls.join(route, model.state.value.servers) }
            waitFor("replacement call") { model.voiceCalls.state.value.session?.connected == true }
            val replacement = model.voiceCalls.state.value.session!!
            assertNotEquals(session.sessionId, replacement.sessionId)
            main { oldLeave.send() }
            Thread.sleep(300)
            assertEquals(replacement.sessionId, model.voiceCalls.state.value.session!!.sessionId)
            assertTrue(model.voiceCalls.state.value.session!!.connected)
            main { model.voiceCalls.leave(model.voiceCalls.state.value.session!!) }
            waitFor("leave before slow join") {
                model.voiceCalls.state.value.session == null && VoiceCallService.active == null
            }
            URL("$endpoint/control/ice-delay?ms=1000").readText()
            main { model.voiceCalls.join(route, model.state.value.servers) }
            waitFor("connecting notification") {
                model.voiceCalls.state.value.session != null && VoiceCallService.active != null
            }
            val connecting = model.voiceCalls.state.value.session!!
            main { oldLeave.send() }
            Thread.sleep(100)
            assertEquals(connecting.sessionId, model.voiceCalls.state.value.session!!.sessionId)
            main { VoiceCallService.active!!.control(connecting, true).send() }
            waitFor("notification cancels ICE lookup") {
                model.voiceCalls.state.value.session == null && VoiceCallService.active == null
            }
            Thread.sleep(1200)
            assertNull(model.voiceCalls.state.value.session)
        } finally {
            URL("$endpoint/control/ice-delay?ms=0").readText()
            main {
                model.voiceCalls.state.value.session
                    ?.takeIf { it.route.serverUrl == endpoint }
                    ?.let(model.voiceCalls::leave)
            }
            waitFor("local call teardown") {
                model.voiceCalls.state.value.session == null && VoiceCallService.active == null
            }
            model.state.value.servers
                .firstOrNull { it.url == endpoint }
                ?.let { runBlocking { model.forgetServer(it) } }
            if (previousServer.isNotBlank()) runBlocking { model.activate(previousServer).join() }
            scenario.close()
        }
    }

    private fun main(block: () -> Unit) =
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)

    private fun waitFor(label: String, condition: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 20_000
        while (!condition() && System.currentTimeMillis() < deadline) Thread.sleep(50)
        assertTrue(label, condition())
    }
}
