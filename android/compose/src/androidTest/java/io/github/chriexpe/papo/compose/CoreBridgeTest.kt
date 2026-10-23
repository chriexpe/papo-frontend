package io.github.chriexpe.papo.compose

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.NativeClient
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class CoreBridgeTest {
    @Test
    fun webhookReceiptCrossesUniFfiOnceWithoutEnteringSnapshots() {
        val handle = NativeClient.demo()
        try {
            val initial = handle.poll()
            val receipt = handle.createWebhook(initial.me, "general", "Integration")
            val id = receipt.webhook().id
            assertTrue(receipt.takeToken().isNotBlank())
            assertEquals("", receipt.takeToken())
            receipt.close()
            assertEquals("Integration", handle.webhooks(initial.me).single().name)
            handle.revokeWebhook(initial.me, id)
            assertTrue(handle.webhooks(initial.me).isEmpty())
            assertTrue(
                initial.capabilities
                    .copy(
                        manageServer = false,
                        manageChannels = false,
                        manageRoles = false,
                        banMembers = false,
                        manageWebhooks = true,
                    )
                    .canAdmin
            )
        } finally {
            handle.shutdown()
            handle.close()
        }
    }

    @Test
    fun coldBackgroundReconciliationWithoutASessionIsABoundedNoop() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val task = io.github.chriexpe.papo.core.NotificationReconcile()
        val listener =
            object : io.github.chriexpe.papo.core.NotificationListener {
                override fun available() = true

                override fun isForeground() = false

                override fun show(alert: io.github.chriexpe.papo.core.MessageAlert) {
                    fail("No session may publish a notification")
                }
            }
        val before = android.os.SystemClock.elapsedRealtime()
        try {
            assertFalse(
                task.run(
                    "http://127.0.0.1:9",
                    context.cacheDir.resolve("background-${System.nanoTime()}.db").absolutePath,
                    AndroidSecrets(context),
                    listener,
                )
            )
            assertTrue(android.os.SystemClock.elapsedRealtime() - before < 5000)
        } finally {
            task.destroy()
        }
    }

    @Test
    fun readViewportCrossesUniFfiWithoutCouplingBadgeToCatchUp() {
        val handle = NativeClient.demo()
        try {
            val start = handle.poll()
            val messages = start.messages
            handle.dispatch(
                io.github.chriexpe.papo.core.Action.Visible(
                    start.selectedChannel,
                    listOf(messages.first().id),
                    false,
                    start.me,
                )
            )
            handle.dispatch(
                io.github.chriexpe.papo.core.Action.BreakReadSpan(start.selectedChannel)
            )
            handle.dispatch(
                io.github.chriexpe.papo.core.Action.Visible(
                    start.selectedChannel,
                    listOf(messages.last().id),
                    true,
                    start.me,
                )
            )
            val atNewest = handle.poll()
            assertFalse(atNewest.channels.first { it.id == start.selectedChannel }.unread)
            assertTrue(atNewest.reading.hourglassTarget.isNotBlank())
            assertFalse(atNewest.reading.syncPending)
            handle.dispatch(
                io.github.chriexpe.papo.core.Action.ClearCatchUp(start.selectedChannel, start.me)
            )
            assertTrue(handle.poll().reading.hourglassTarget.isBlank())
        } finally {
            handle.shutdown()
            handle.close()
        }
    }

    @Test
    fun keystoreSeparatesServersAndNativeCoreWakesWithoutPollingTimers() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val secrets = AndroidSecrets(context)
        val server = "instrumentation-${System.nanoTime()}"
        secrets.store(server, "token", "test-session")
        secrets.store(server, "server", "test-password")
        assertEquals("test-session", secrets.load(server, "token"))
        assertEquals("test-password", secrets.load(server, "server"))
        assertNull(secrets.load("other-$server", "token"))
        val encoded =
            context.getSharedPreferences("core-secrets", 0).getString("$server/token", "")!!
        assertFalse(encoded.contains("test-session"))
        secrets.remove(server, "token")
        secrets.remove(server, "server")
        assertNull(secrets.load(server, "token"))

        val wake = Semaphore(0)
        val db = context.cacheDir.resolve("bridge-${System.nanoTime()}.db")
        val handle =
            NativeClient.open(
                "http://127.0.0.1:9",
                db.absolutePath,
                secrets,
                CoreWake { wake.release() },
            )
        try {
            assertTrue(
                "Native network worker must signal the actor",
                wake.tryAcquire(10, TimeUnit.SECONDS),
            )
            val state = handle.poll()
            assertEquals(1, state.version)
            assertEquals("http://127.0.0.1:9", state.url)
            assertEquals(0, state.messages.size)
        } finally {
            handle.shutdown()
            handle.close()
        }
    }
}
