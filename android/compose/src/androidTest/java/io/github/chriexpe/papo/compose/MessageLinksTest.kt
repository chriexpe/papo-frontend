package io.github.chriexpe.papo.compose

import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.NativeClient
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class MessageLinksTest {
    @Test
    fun replyPreviewsResolveMentionsWithoutChangingWireContentOrCodeLiterals() {
        val client = NativeClient.demo()
        try {
            val content = "**Hello** @mention(<@ana>)"
            client.dispatch(
                Action.Send("preview-mention", "general", content, null, true, emptyList())
            )
            val message = client.poll().messages.single { it.id == "preview-mention" }
            assertEquals("Hello @Ana", message.previewText())
            assertEquals(content, message.content)
            // An APP author uses exactly the same resolved presentation tree.
            assertEquals(
                "Hello @Ana",
                message
                    .copy(
                        externalAuthor =
                            client.run {
                                dispatch(Action.SelectChannel("design"))
                                poll().messages.first { it.externalAuthor != null }.externalAuthor
                            }
                    )
                    .previewText(),
            )
            val media = "https://cdn.test/native.gif"
            val mixed = "**Caption** $media\n\n`$media` [named]($media)"
            val literal =
                message.copy(
                    content = mixed,
                    markdown = io.github.chriexpe.papo.core.parseMessageMarkdown(mixed),
                    embeds = emptyList(),
                )
            assertEquals("Caption \n\n$media named", literal.previewText())
        } finally {
            client.close()
        }
    }

    @Test
    fun copiesExactUrlsAndHidesOnlyDirectMediaWhileKeepingCaptionAndOtherLinks() {
        assertEquals(
            listOf("https://cdn.example/image_(1).gif?sig=a%2Bb&x=2", "http://example.test/page"),
            messageLinks(
                    "See (https://cdn.example/image_(1).gif?sig=a%2Bb&x=2), then http://example.test/page."
                )
                .map { it.url },
        )
        val client = NativeClient.demo()
        try {
            client.dispatch(Action.SelectChannel("media"))
            val messages = client.poll().messages
            val portrait = messages.single { it.id == "media-7" }
            assertEquals(
                listOf("https://cdn.example.test/portrait.png?sig=fixture"),
                portrait.nativeLinks().map { it.url },
            )
            assertTrue(portrait.visibleContent().contains("As dimensões"))
            assertTrue(portrait.visibleContent().contains("https://example.test/more"))
            assertFalse(portrait.visibleContent().contains("cdn.example.test"))
            assertTrue(
                messages
                    .single { it.id == "media-6" }
                    .embeds
                    .single()
                    .let { !it.nativeMedia && it.sourceType == "custom" }
            )
            assertFalse(messages.single { it.id == "media-10" }.embeds.single().nativeMedia)
        } finally {
            client.close()
        }
    }

    @Test
    fun publicGifRequestsHaveARealOriginAndDownloadsPublishTheExactBytes() = runBlocking {
        val source = publicGifSource("https://media.example.test/a.gif?sig=secret")
        assertEquals("https://media.example.test", source.origin)
        assertEquals("public/https://media.example.test/a.gif?sig=secret", source.cacheKey)
        assertTrue(source.headers.isEmpty())
        val context = downloadTestContext()
        val file = context.cacheDir.resolve("download-${System.nanoTime()}.gif")
        file.writeBytes(context.assets.open("demo/animation.gif").use { it.readBytes() })
        try {
            val uri = saveMediaDownload(context, file, "Papo download fixture.gif", "image/gif")
            try {
                assertArrayEquals(
                    file.readBytes(),
                    context.contentResolver.openInputStream(uri)!!.use { it.readBytes() },
                )
                context.contentResolver
                    .query(
                        uri,
                        arrayOf(
                            android.provider.MediaStore.MediaColumns.IS_PENDING,
                            android.provider.MediaStore.MediaColumns.MIME_TYPE,
                        ),
                        null,
                        null,
                        null,
                    )!!
                    .use { cursor ->
                        assertTrue(cursor.moveToFirst())
                        assertEquals(0, cursor.getInt(0))
                        assertEquals("image/gif", cursor.getString(1))
                    }
            } finally {
                context.contentResolver.delete(uri, null, null)
            }
        } finally {
            file.delete()
        }
    }

    private fun downloadTestContext(): android.content.Context {
        val base = InstrumentationRegistry.getInstrumentation().targetContext
        return object : android.content.ContextWrapper(base) {
                override fun getSharedPreferences(name: String, mode: Int) =
                    super.getSharedPreferences(
                        if (name == "compose-appearance") "download-test-fixture" else name,
                        mode,
                    )
            }
            .also { it.getSharedPreferences("compose-appearance", 0).edit().clear().commit() }
    }

    @Test
    fun chosenDocumentFolderReceivesExactBytesAndRevokedFoldersFailClearly() = runBlocking {
        val context = downloadTestContext()
        val preferences = context.getSharedPreferences("compose-appearance", 0)
        val tree =
            android.provider.DocumentsContract.buildTreeDocumentUri(
                "io.github.chriexpe.papo.compose.test.documents",
                "root",
            )
        val file =
            context.cacheDir.resolve("document-download-${System.nanoTime()}.gif").apply {
                writeBytes(context.assets.open("demo/animation.gif").use { it.readBytes() })
            }
        try {
            preferences
                .edit()
                .putString("download-tree", tree.toString())
                .putString("download-label", "Papo test folder")
                .commit()
            assertEquals("Papo test folder", downloadLocation(context))
            InstrumentationRegistry.getInstrumentation()
                .uiAutomation
                .adoptShellPermissionIdentity("android.permission.MANAGE_DOCUMENTS")
            val uri = saveMediaDownload(context, file, "fixture.gif", "image/gif")
            try {
                assertArrayEquals(
                    file.readBytes(),
                    context.contentResolver.openInputStream(uri)!!.use { it.readBytes() },
                )
            } finally {
                android.provider.DocumentsContract.deleteDocument(context.contentResolver, uri)
            }
            preferences
                .edit()
                .putString("download-tree", "content://missing.papo.fixture/tree/root")
                .commit()
            var failed = false
            try {
                saveMediaDownload(context, file, "fixture.gif", "image/gif")
            } catch (_: Exception) {
                failed = true
            }
            assertTrue(
                "An unavailable folder must fail instead of silently saving somewhere else",
                failed,
            )
        } finally {
            InstrumentationRegistry.getInstrumentation().uiAutomation.dropShellPermissionIdentity()
            preferences.edit().clear().commit()
            file.delete()
        }
    }
}
