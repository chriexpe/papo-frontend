package io.github.chriexpe.papo.compose

import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit
import org.junit.runner.RunWith
import androidx.test.ext.junit.runners.AndroidJUnit4

@RunWith(AndroidJUnit4::class)
class CoreBridgeTest {
    @Test fun keystoreSeparatesServersAndNativeCoreWakesWithoutPollingTimers() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val secrets = AndroidSecrets(context)
        val server = "instrumentation-${System.nanoTime()}"
        secrets.store(server, "token", "test-session")
        secrets.store(server, "server", "test-password")
        assertEquals("test-session", secrets.load(server, "token"))
        assertEquals("test-password", secrets.load(server, "server"))
        assertNull(secrets.load("other-$server", "token"))
        val encoded = context.getSharedPreferences("core-secrets", 0).getString("$server/token", "")!!
        assertFalse(encoded.contains("test-session"))
        secrets.remove(server, "token")
        secrets.remove(server, "server")
        assertNull(secrets.load(server, "token"))

        val wake = Semaphore(0)
        val db = context.cacheDir.resolve("bridge-${System.nanoTime()}.db")
        val handle = NativeCore.open("http://127.0.0.1:9", db.absolutePath, secrets, CoreWake { wake.release() })
        try {
            assertTrue("Native network worker must signal the actor", wake.tryAcquire(10, TimeUnit.SECONDS))
            val state = JSONObject(NativeCore.poll(handle))
            assertEquals(1, state.getInt("version"))
            assertEquals("http://127.0.0.1:9", state.getString("url"))
            assertEquals(0, state.getJSONArray("messages").length())
        } finally { NativeCore.close(handle) }
    }
}
