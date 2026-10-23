package io.github.chriexpe.papo.compose

import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.MediaRequest
import org.junit.Assert.*
import org.junit.Test

class MediaLifecycleTest {
    @Test
    fun missingGeometryLearnsDecodedSizeAndVideoOutranksItsPoster() {
        val frame = MediaFrameState(null)
        assertNull(frame.ratio)
        frame.reportImageRatio(Float.NaN)
        frame.reportVideoRatio(0f)
        assertNull(frame.ratio)
        frame.reportImageRatio(4f / 3f)
        assertEquals(4f / 3f, requireNotNull(frame.ratio), .0001f)
        frame.reportImageRatio(1.334f)
        assertEquals(
            "Downsampling rounding must not feed back into layout",
            4f / 3f,
            requireNotNull(frame.ratio),
            .0001f,
        )
        frame.reportVideoRatio(16f / 9f)
        frame.reportImageRatio(4f / 3f)
        assertEquals(
            "Reattaching the poster must not resize a known video",
            16f / 9f,
            requireNotNull(frame.ratio),
            .0001f,
        )
    }

    @Test
    fun suppliedGeometryStaysAuthoritativeThroughDecodingAndPlayback() {
        val frame = MediaFrameState(540f / 960f)
        frame.reportImageRatio(4f / 3f)
        frame.reportVideoRatio(16f / 9f)
        assertEquals(540f / 960f, requireNotNull(frame.ratio), .0001f)
        assertNull(MediaFrameState(null).ratio)
    }

    private fun onMain(block: () -> Unit) {
        var failure: Throwable? = null
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            try {
                block()
            } catch (e: Throwable) {
                failure = e
            }
        }
        failure?.let { throw it }
    }

    @Test
    fun visibilityPausesAudioAndVideoWhileFullscreenKeepsTheSamePlayerAlive() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        onMain {
            for (file in listOf("clip.mp4", "voice-note.m4a")) {
                val context = instrumentation.targetContext
                val player =
                    ExoPlayer.Builder(context).build().apply {
                        setMediaItem(MediaItem.fromUri("asset:///demo/$file"))
                        prepare()
                    }
                var saved: PlaybackBookmark? = null
                val handle =
                    PlaybackHandle(
                        player,
                        MediaRequest("asset:///demo/$file", emptyMap(), file, "demo"),
                        true,
                    ) {
                        saved = it
                    }
                val inline = Any()
                val fullscreen = Any()
                handle.acquire(inline)
                assertFalse(player.playWhenReady)
                handle.setVisible(inline, true)
                assertTrue(player.playWhenReady)
                player.seekTo(300)
                handle.setVisible(inline, false)
                assertFalse("Offscreen playback must pause", player.playWhenReady)
                assertEquals(300L, player.currentPosition)
                handle.setVisible(inline, true)
                assertTrue("Visibility resumes prior play intent", player.playWhenReady)
                player.pause()
                handle.setVisible(inline, false)
                handle.setVisible(inline, true)
                assertFalse("Manual pause survives scrolling", player.playWhenReady)
                player.play()
                handle.acquire(fullscreen)
                handle.setVisible(fullscreen, true)
                handle.setVisible(inline, false)
                assertTrue(
                    "An open viewer remains active when its row is offscreen",
                    player.playWhenReady,
                )
                handle.release(inline)
                assertTrue(player.playWhenReady)
                handle.setVisible(fullscreen, false)
                handle.release(fullscreen)
                assertEquals(Player.STATE_IDLE, player.playbackState)
                assertEquals(PlaybackBookmark(300, true), saved)
            }
        }
    }

    @Test
    fun bookmarksKeepPositionsBoundedAndSeparateEveryMediaId() {
        val bookmarks = PlaybackBookmarks()
        for (i in 0 until 140) bookmarks.save("media-$i", PlaybackBookmark(i.toLong(), i % 2 == 0))
        assertNull(bookmarks["media-0"])
        assertEquals(PlaybackBookmark(139, false), bookmarks["media-139"])
        bookmarks.save("media-139", PlaybackBookmark(700, false))
        assertEquals(PlaybackBookmark(700, false), bookmarks["media-139"])
        assertEquals(PlaybackBookmark(138, true), bookmarks["media-138"])
    }

    @Test
    fun sharedDecoderReusesStaticImagesWithoutSharingAnimationClocks() =
        kotlinx.coroutines.runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val loader = MediaImages.loader(context)
            val owner = "decode-${System.nanoTime()}"
            var staticLoads = 0
            fun request(identity: String, asset: String, onLoad: () -> Unit) =
                coil3.request.ImageRequest.Builder(context)
                    .data(
                        CachedImage(identity) {
                            onLoad()
                            context.assets.open(asset).use { it.readBytes() }
                        }
                    )
                    .size(256)
                    .build()
            val first =
                loader.execute(request("$owner/avatar", "demo/portrait.png") { staticLoads++ })
                    as coil3.request.SuccessResult
            val second =
                loader.execute(request("$owner/avatar", "demo/portrait.png") { staticLoads++ })
                    as coil3.request.SuccessResult
            assertEquals(1, staticLoads)
            assertEquals(coil3.decode.DataSource.MEMORY_CACHE, second.dataSource)
            assertSame(first.image, second.image)
            loader.execute(request("$owner/other-account", "demo/portrait.png") { staticLoads++ })
            assertEquals(
                "Identity boundaries must miss the old account’s decoded key",
                2,
                staticLoads,
            )
            var animationLoads = 0
            val animated =
                loader.execute(
                    request("$owner/animated", "demo/animation.gif") { animationLoads++ }
                ) as coil3.request.SuccessResult
            val another =
                loader.execute(
                    request("$owner/animated", "demo/animation.gif") { animationLoads++ }
                ) as coil3.request.SuccessResult
            assertEquals(2, animationLoads)
            assertFalse(animated.image.shareable)
            assertNotSame(animated.image, another.image)
            loader.memoryCache?.clear()
            Unit
        }
}
