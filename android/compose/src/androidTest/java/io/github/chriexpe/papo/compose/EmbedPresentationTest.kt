package io.github.chriexpe.papo.compose

import android.graphics.BitmapFactory
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.MediaResource
import io.github.chriexpe.papo.core.NativeClient
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class EmbedPresentationTest {
    @Test
    fun socialTitlesKeepAuthorAndHandleSeparateFromCaptionAndStatistics() {
        val client = NativeClient.demo()
        try {
            client.dispatch(Action.SelectChannel("media"))
            val embed =
                client.poll().messages.flatMap { it.embeds }.first { it.id == "embed-video" }
            val details =
                embed
                    .copy(
                        title = "An author (@author)",
                        description = "❤️ 5,530  💬 234\n\nA caption\nwith another line.",
                    )
                    .presentation()
            assertEquals("An author", details.author)
            assertEquals("@author", details.handle)
            assertEquals("", details.title)
            assertEquals("A caption\nwith another line.", details.caption)
            assertEquals("❤️ 5,530  💬 234", details.statistics)
            val generic =
                embed
                    .copy(
                        title = "Video (with context)",
                        author = "An author",
                        description = "A normal description",
                    )
                    .presentation()
            assertEquals("An author", generic.author)
            assertEquals("Video (with context)", generic.title)
            assertEquals("", generic.handle)
            assertEquals("", generic.statistics)
            assertEquals("A normal description", generic.caption)
        } finally {
            client.shutdown()
            client.close()
        }
    }

    @Test
    fun posterDecodesTheVideoCachesItsFrameAndHonorsAccountRevocation() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        AppLanguage.update(context)
        val directory = context.cacheDir.resolve("poster-test-${System.nanoTime()}")
        val cache = MediaCache.isolated(context, directory)
        val client = NativeClient.demo()
        val server = "https://poster-test.example"
        cache.allowServer(server)
        try {
            val source =
                cache.scoped(client.mediaRequest(MediaResource.EmbedVideo("embed-video")), server)
            val first = VideoPosters.load(cache, source, "revision")
            val bitmap = requireNotNull(BitmapFactory.decodeByteArray(first, 0, first.size))
            try {
                assertEquals(640, bitmap.width)
                assertEquals(360, bitmap.height)
            } finally {
                bitmap.recycle()
            }
            val size = cache.size
            assertTrue(size > first.size)
            assertArrayEquals(first, VideoPosters.load(cache, source, "revision"))
            assertEquals(size, cache.size)
            cache.forgetServer(server)
            assertEquals(0, cache.size)
            assertTrue(runCatching { VideoPosters.load(cache, source, "revision") }.isFailure)
        } finally {
            client.shutdown()
            client.close()
            cache.close()
            directory.deleteRecursively()
        }
    }
}
