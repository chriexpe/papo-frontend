package io.github.chriexpe.papo.compose

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.NativeClient
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AttachmentDraftTest {
    @Test fun interruptedUploadRestoresFilesWithAnUncertainOutcomeAndNeverResendsItself() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val preferences = context.getSharedPreferences("compose-attachments", 0)
        val saved = preferences.all.toMap()
        preferences.edit().clear().commit()
        val file = AttachmentFiles.create(context, "txt").apply { writeText("private attachment draft") }
        val firstScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val restoredScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val native = NativeClient.demo()
        try {
            val started = CompletableDeferred<Unit>()
            val waiting = CompletableDeferred<UploadReceipt>()
            val first = AttachmentDrafts(context, firstScope) { _, _, _, _ -> started.complete(Unit); waiting.await() }
            first.captured("fixture/server/account/channel", file, "draft.txt", "text/plain")
            withTimeout(5000) { first.drafts.first { it.values.singleOrNull()?.files?.size == 1 } }
            first.send("fixture/server/account/channel", native.poll(), "caption", null) {}
            withTimeout(5000) { started.await() }
            assertTrue(org.json.JSONObject(preferences.getString("fixture/server/account/channel", null)!!).getBoolean("sending"))
            firstScope.cancel()
            var sends = 0
            val restored = AttachmentDrafts(context, restoredScope) { _, _, _, _ -> sends++; UploadReceipt(true) }
            val draft = withTimeout(5000) { restored.drafts.first { it.isNotEmpty() }.values.single() }
            assertTrue(draft.uncertain)
            assertFalse(draft.sending)
            assertTrue(file.isFile)
            assertEquals("draft.txt", draft.files.single().name)
            assertEquals(0, sends)
        } finally {
            firstScope.cancel(); restoredScope.cancel(); native.shutdown(); native.close(); file.delete()
            preferences.edit().clear().apply { saved.forEach { (key, value) -> putString(key, value as String) } }.commit()
        }
    }
}
