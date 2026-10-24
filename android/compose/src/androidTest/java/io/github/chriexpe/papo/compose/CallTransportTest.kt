package io.github.chriexpe.papo.compose

import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.CallVideoFrame
import io.github.chriexpe.papo.core.ClientSnapshot
import io.github.chriexpe.papo.core.Credentials
import io.github.chriexpe.papo.core.NativeClient
import java.net.URL
import java.util.concurrent.ConcurrentHashMap
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Uses tests/call-sfu on the host, with real SDP/ICE/DTLS/SRTP and synthetic camera/video. */
@RunWith(AndroidJUnit4::class)
class CallTransportTest {
    private val endpoint = "http://10.0.2.2:18765"

    @Test
    fun realMediaLifecycleRejectsOldSessionsAndDisconnectsWithoutAutoRejoining() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val values = ConcurrentHashMap<String, String>()
        val credentials =
            object : Credentials {
                override fun load(server: String, secret: String): String? =
                    values["$server/$secret"]

                override fun store(server: String, secret: String, value: String) {
                    values["$server/$secret"] = value
                }

                override fun remove(server: String, secret: String) {
                    values.remove("$server/$secret")
                }
            }
        val database = context.cacheDir.resolve("call-transport-${System.nanoTime()}.db")
        ActivityScenario.launch(MainActivity::class.java).use {
            val handle =
                NativeClient.open(endpoint, database.absolutePath, credentials, CoreWake {})
            try {
                handle.setCallHost(AndroidCallRuntime.host {})
                handle.dispatch(Action.Login("tester", "fixture-only"))
                val account =
                    await(handle, "login") {
                        it.screen == "chat" &&
                            it.connection == "online" &&
                            it.channels.any { channel -> channel.kind == "voice" }
                    }
                assertTrue(account.channels.any { it.kind == "voice" })
                val joinsBefore = JSONObject(URL("$endpoint/stats").readText()).getLong("joins")
                URL("$endpoint/control/ice-delay?ms=200").readText()
                handle.dispatch(Action.JoinVoice("voice"))
                val cancelled = handle.poll().call.attempt
                handle.dispatch(Action.LeaveVoice(cancelled))
                Thread.sleep(400)
                assertEquals("off", handle.poll().call.phase)
                assertEquals(
                    joinsBefore,
                    JSONObject(URL("$endpoint/stats").readText()).getLong("joins"),
                )
                URL("$endpoint/control/ice-delay?ms=0").readText()
                handle.dispatch(Action.JoinVoice("voice"))
                val first =
                    await(handle, "connected with audio routes") {
                        it.call.connected && it.call.playbackAvailable
                    }
                assertTrue(first.call.muted)
                assertFalse(first.call.camera)
                assertTrue(
                    first.call.rooms.any { room ->
                        room.members.any { m -> m.userId == "ana" && m.camera }
                    }
                )
                val remote = frame(handle, first, "ana", "video")
                assertEquals(320, remote.width.toInt())
                assertEquals(180, remote.height.toInt())
                assertTrue(remote.rgba.any { it.toInt() != 0 })
                val screen = frame(handle, first, "ana", "screen")
                assertEquals(320, screen.width.toInt())
                assertEquals(180, screen.height.toInt())
                assertNull(
                    handle.callVideoFrame("wrong-owner", first.call.attempt, "ana", "video", 0u)
                )
                handle.dispatch(Action.VoicePlayback(first.call.attempt, "ana", 175u, false))
                handle.dispatch(Action.VoicePlayback(first.call.attempt, "ana", 175u, true))
                val muted = handle.poll().call.playback.single { it.userId == "ana" }
                assertEquals(175, muted.volumePercent.toInt())
                assertTrue(muted.muted)
                handle.dispatch(Action.MuteVoice(first.call.attempt, false))
                assertFalse(handle.poll().call.muted)
                awaitStats("outgoing microphone") { it.optLong("audio") > 0 }
                handle.dispatch(Action.CameraVoice(first.call.attempt, true))
                val camera = await(handle, "camera") { it.call.camera }
                val local = frame(handle, camera, camera.me, "video")
                assertEquals(local.width.toInt() * local.height.toInt() * 4, local.rgba.size)
                awaitStats("outgoing camera") { it.optLong("video") > 0 }
                handle.dispatch(Action.CameraFacing(first.call.attempt, false))
                await(handle, "back camera") { !it.call.frontCamera && it.call.camera }
                handle.dispatch(Action.CameraVoice(first.call.attempt, false))
                await(handle, "camera off") { !it.call.camera }
                handle.dispatch(Action.LeaveVoice(first.call.attempt))
                await(handle, "left") { it.call.phase == "off" }
                assertTrue(
                    handle.poll().call.rooms.any { room -> room.members.any { it.userId == "ana" } }
                )
                // Fixture deliberately delays leave: rejoin immediately, before its
                // acknowledgement.
                handle.dispatch(Action.JoinVoice("voice"))
                val second =
                    await(handle, "rejoined") {
                        it.call.connected && it.call.attempt != first.call.attempt
                    }
                handle.dispatch(Action.LeaveVoice(first.call.attempt))
                handle.dispatch(Action.MuteVoice(first.call.attempt, false))
                val unchanged = handle.poll()
                assertEquals(second.call.attempt, unchanged.call.attempt)
                assertTrue(unchanged.call.muted)
                assertEquals("in", unchanged.call.phase)
                assertNull(handle.callVideoFrame(second.me, first.call.attempt, "ana", "video", 0u))
                URL("$endpoint/control/kick").readText()
                await(handle, "server ends room membership") { it.call.phase == "off" }
                handle.dispatch(Action.JoinVoice("voice"))
                await(handle, "join after server leave") {
                    it.call.connected && it.call.attempt != second.call.attempt
                }
                URL("$endpoint/control/disconnect").readText()
                await(handle, "socket loss ends media") { it.call.phase == "off" }
                val restored = await(handle, "socket reconnect") { it.connection == "online" }
                assertEquals("off", restored.call.phase)
            } finally {
                handle.shutdown()
                handle.close()
                database.delete()
                context.cacheDir.resolve(database.name + "-wal").delete()
                context.cacheDir.resolve(database.name + "-shm").delete()
            }
        }
    }

    private fun await(
        handle: NativeClient,
        label: String,
        predicate: (ClientSnapshot) -> Boolean,
    ): ClientSnapshot {
        val end = System.currentTimeMillis() + 20_000
        var value = handle.poll()
        while (!predicate(value) && System.currentTimeMillis() < end) {
            Thread.sleep(50)
            value = handle.poll()
        }
        assertTrue(
            "$label: screen=${value.screen}, coreError=${value.error}, connection=${value.connection}, phase=${value.call.phase}, error=${value.call.error}, warning=${value.call.warning}",
            predicate(value),
        )
        return value
    }

    private fun frame(
        handle: NativeClient,
        state: ClientSnapshot,
        user: String,
        kind: String,
    ): CallVideoFrame {
        val end = System.currentTimeMillis() + 15_000
        while (System.currentTimeMillis() < end) {
            handle.poll()
            handle.callVideoFrame(state.me, state.call.attempt, user, kind, 0u)?.let {
                return it
            }
            Thread.sleep(50)
        }
        error("No decoded $kind frame from $user; call=${handle.poll().call}")
    }

    private fun awaitStats(label: String, predicate: (JSONObject) -> Boolean) {
        val end = System.currentTimeMillis() + 10_000
        var value = JSONObject(URL("$endpoint/stats").readText())
        while (!predicate(value) && System.currentTimeMillis() < end) {
            Thread.sleep(50)
            value = JSONObject(URL("$endpoint/stats").readText())
        }
        assertTrue("$label: $value", predicate(value))
    }
}
