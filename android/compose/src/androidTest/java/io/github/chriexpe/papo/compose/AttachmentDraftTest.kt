package io.github.chriexpe.papo.compose

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.NativeClient
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AttachmentDraftTest {
    @Before
    fun initializePresentationResources() {
        // These model tests do not launch an Activity, so initialize the same
        // resource context that MainActivity normally provides for callbacks.
        AppLanguage.update(InstrumentationRegistry.getInstrumentation().targetContext)
    }

    @Test
    fun attachmentOptionsRestoreAndMetadataRemovalCanBeReversedWithoutLosingTheSource() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val preferences = context.getSharedPreferences("compose-attachments", 0)
            val saved = preferences.all.toMap()
            val key = "test/attachment-options"
            preferences.edit().remove(key).commit()
            val source = AttachmentFiles.create(context, "png")
            val bitmap =
                android.graphics.Bitmap.createBitmap(8, 6, android.graphics.Bitmap.Config.ARGB_8888)
            source.outputStream().use {
                bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
            }
            bitmap.recycle()
            val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
            val restoredScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
            var privacyCopy: java.io.File? = null
            try {
                val drafts =
                    AttachmentDrafts(context, scope) { _, _, _, _ ->
                        error("Editing must never send")
                    }
                drafts.captured(key, source, "photo.png", "image/png").join()
                val original = drafts.drafts.value.getValue(key).files.single()
                withContext(Dispatchers.Main.immediate) {
                    drafts.applyOptions(
                        key,
                        original,
                        "chosen-name",
                        spoiler = true,
                        removeMetadata = true,
                    )
                }
                val applied = drafts.drafts.value.getValue(key).files.single()
                privacyCopy = java.io.File(applied.sanitized!!.path)
                assertTrue(source.isFile)
                assertTrue(privacyCopy!!.isFile)
                assertEquals("SPOILER_chosen-name.png", applied.wireName)
                assertEquals(privacyCopy!!.absolutePath, applied.native().path)
                assertTrue(applied.native().spoiler)
                val restored =
                    AttachmentDrafts(context, restoredScope) { _, _, _, _ ->
                        error("Restoring must never send")
                    }
                val file =
                    withTimeout(5000) {
                        restored.drafts.first { key in it }.getValue(key).files.single()
                    }
                assertEquals(applied, file)
                withContext(Dispatchers.Main.immediate) {
                    restored.applyOptions(
                        key,
                        file,
                        "renamed",
                        spoiler = false,
                        removeMetadata = false,
                    )
                }
                val reverted = restored.drafts.value.getValue(key).files.single()
                assertNull(reverted.sanitized)
                assertFalse(reverted.spoiler)
                assertEquals(source.absolutePath, reverted.native().path)
                assertEquals("renamed.png", reverted.name)
                assertTrue(source.isFile)
                assertFalse(privacyCopy!!.exists())
            } finally {
                scope.cancel()
                restoredScope.cancel()
                source.delete()
                privacyCopy?.delete()
                preferences
                    .edit()
                    .clear()
                    .apply { saved.forEach { (k, v) -> putString(k, v as String) } }
                    .commit()
            }
        }

    @Test
    fun failedMetadataRemovalKeepsTheOriginalDraftAndDoesNotEnableItsSpoiler() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val preferences = context.getSharedPreferences("compose-attachments", 0)
        val saved = preferences.all.toMap()
        val key = "test/invalid-privacy-image"
        preferences.edit().remove(key).commit()
        val source =
            AttachmentFiles.create(context, "png").apply { writeText("not a decodable image") }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        try {
            val drafts =
                AttachmentDrafts(context, scope) { _, _, _, _ ->
                    error("A failed edit must never send")
                }
            drafts.captured(key, source, "photo.png", "image/png").join()
            val before = drafts.drafts.value.getValue(key)
            try {
                withContext(Dispatchers.Main.immediate) {
                    drafts.applyOptions(
                        key,
                        before.files.single(),
                        "changed",
                        spoiler = true,
                        removeMetadata = true,
                    )
                }
                fail("Invalid image must fail closed")
            } catch (_: java.io.IOException) {}
            assertEquals(before, drafts.drafts.value.getValue(key))
            assertTrue(source.isFile)
            val savedFile =
                org.json
                    .JSONObject(preferences.getString(key, null)!!)
                    .getJSONArray("files")
                    .getJSONObject(0)
            assertEquals("photo.png", savedFile.getString("name"))
            assertFalse(savedFile.getBoolean("spoiler"))
            assertTrue(savedFile.isNull("sanitized"))
        } finally {
            scope.cancel()
            source.delete()
            preferences
                .edit()
                .clear()
                .apply { saved.forEach { (k, v) -> putString(k, v as String) } }
                .commit()
        }
    }

    @Test
    fun editedImageReplacesThePersistedDraftBeforeRemovingItsOriginal() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val preferences = context.getSharedPreferences("compose-attachments", 0)
        val saved = preferences.all.toMap()
        val key = "test/edited-image"
        preferences.edit().remove(key).commit()
        val original =
            AttachmentFiles.create(context, "png").apply { writeText("original private draft") }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        var replacement: java.io.File? = null
        try {
            val drafts =
                AttachmentDrafts(context, scope) { _, _, _, _ -> error("Editing must never send") }
            drafts.captured(key, original, "photo.png", "image/png").join()
            val first = drafts.drafts.value.getValue(key).files.single()
            val bytes = "encoded edited image".toByteArray()
            val image =
                io.github.chriexpe.papo.core.PreparedMediaImage(
                    android.util.Base64.encodeToString(bytes, android.util.Base64.NO_WRAP),
                    "WEBP",
                    20u,
                    10u,
                    bytes.size.toULong(),
                    false,
                )
            withContext(Dispatchers.Main.immediate) { drafts.replaceEdited(key, first, image) }
            val edited = drafts.drafts.value.getValue(key).files.single()
            replacement = java.io.File(edited.path)
            assertFalse(original.exists())
            assertArrayEquals(bytes, replacement!!.readBytes())
            assertEquals("photo.webp", edited.name)
            assertEquals("image/webp", edited.mime)
            assertEquals(
                edited.path,
                org.json
                    .JSONObject(preferences.getString(key, null)!!)
                    .getJSONArray("files")
                    .getJSONObject(0)
                    .getString("path"),
            )
            try {
                withContext(Dispatchers.Main.immediate) { drafts.replaceEdited(key, first, image) }
                fail("Stale original must be rejected")
            } catch (_: IllegalStateException) {}
            assertTrue(replacement!!.exists())
        } finally {
            scope.cancel()
            original.delete()
            replacement?.delete()
            preferences
                .edit()
                .clear()
                .apply { saved.forEach { (k, v) -> putString(k, v as String) } }
                .commit()
        }
    }

    @Test
    fun interruptedUploadRestoresFilesWithAnUncertainOutcomeAndNeverResendsItself() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val preferences = context.getSharedPreferences("compose-attachments", 0)
        val saved = preferences.all.toMap()
        preferences.edit().clear().commit()
        val file =
            AttachmentFiles.create(context, "txt").apply { writeText("private attachment draft") }
        val firstScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val restoredScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val native = NativeClient.demo()
        try {
            val started = CompletableDeferred<Unit>()
            val waiting = CompletableDeferred<UploadReceipt>()
            val first =
                AttachmentDrafts(context, firstScope) { _, _, _, _ ->
                    started.complete(Unit)
                    waiting.await()
                }
            first.captured("fixture/server/account/channel", file, "draft.txt", "text/plain")
            withTimeout(5000) { first.drafts.first { it.values.singleOrNull()?.files?.size == 1 } }
            first.send("fixture/server/account/channel", native.poll(), "caption", null) {}
            withTimeout(5000) { started.await() }
            assertTrue(
                org.json
                    .JSONObject(preferences.getString("fixture/server/account/channel", null)!!)
                    .getBoolean("sending")
            )
            firstScope.cancel()
            var sends = 0
            val restored =
                AttachmentDrafts(context, restoredScope) { _, _, _, _ ->
                    sends++
                    UploadReceipt(true)
                }
            val draft =
                withTimeout(5000) { restored.drafts.first { it.isNotEmpty() }.values.single() }
            assertTrue(draft.uncertain)
            assertFalse(draft.sending)
            assertTrue(file.isFile)
            assertEquals("draft.txt", draft.files.single().name)
            assertEquals(0, sends)
        } finally {
            firstScope.cancel()
            restoredScope.cancel()
            native.shutdown()
            native.close()
            file.delete()
            preferences
                .edit()
                .clear()
                .apply { saved.forEach { (key, value) -> putString(key, value as String) } }
                .commit()
        }
    }
}
