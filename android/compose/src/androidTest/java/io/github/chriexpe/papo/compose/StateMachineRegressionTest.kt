package io.github.chriexpe.papo.compose

import android.Manifest
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.NativeClient
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class StateMachineRegressionTest {
    @Test
    fun navigationUnavailableOrCancelledReleasesJumpingButLocatedDoesNot() {
        assertTrue(navigationRequiresUnlock("unavailable", 9uL, 8uL))
        assertTrue(navigationRequiresUnlock("failed", 9uL, 8uL))
        assertTrue(navigationRequiresUnlock("cancelled", 9uL, 8uL))
        assertFalse(navigationRequiresUnlock("located", 9uL, 8uL))
        assertFalse(navigationRequiresUnlock("unavailable", 9uL, 9uL))
        assertFalse(navigationRequiresUnlock("", 9uL, 8uL))
    }

    @Test
    fun typingIsScopedToServerAccountAndChannel() {
        val timestamps = mutableMapOf<Triple<String, String, String>, Long>()
        val first = Triple("server", "alice", "channel-a")
        assertTrue(shouldSendTyping(timestamps, first, 10000))
        assertFalse(shouldSendTyping(timestamps, first, 11000))
        assertTrue(shouldSendTyping(timestamps, Triple("server", "alice", "channel-b"), 11000))
        assertTrue(shouldSendTyping(timestamps, Triple("server", "bob", "channel-a"), 11000))
        assertTrue(shouldSendTyping(timestamps, Triple("other", "alice", "channel-a"), 11000))
        assertTrue(shouldSendTyping(timestamps, first, 13001))
        timestamps.keys.removeAll { it.first == "server" }
        assertTrue(shouldSendTyping(timestamps, first, 13002))
    }

    @Test
    fun repeatedJoinDuringPermissionDispatchesOnlyOnce() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val demo = NativeClient.demo()
        try {
            val live = demo.poll().copy(url = "https://fixture.invalid")
            val route = CallRoute(live.url, live.me, "voice-lounge")
            val servers = listOf(live)
            var preparations = 0
            var joins = 0
            val controller =
                VoiceCallController(
                    CoroutineScope(SupervisorJob() + Dispatchers.Main),
                    request = { _, action, success, _ ->
                        if (action is Action.JoinVoice) joins++
                        success()
                    },
                    granted = { it != Manifest.permission.RECORD_AUDIO },
                    prepare = {
                        preparations++
                        true
                    },
                    routeAudio = { true },
                )
            instrumentation.runOnMainSync {
                controller.join(route, servers)
                val request = requireNotNull(controller.state.value.permission)
                controller.join(route, servers)
                assertEquals(request.serial, controller.state.value.permission?.serial)
                controller.permissionResult(request.serial, true)
                controller.join(route, servers)
                assertEquals(1, joins)
                assertEquals(1, preparations)
                assertEquals(route, controller.state.value.joining)
            }
        } finally {
            demo.shutdown()
            demo.close()
        }
    }
}
