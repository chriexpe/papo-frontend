@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package io.github.chriexpe.papo.compose

import androidx.media3.datasource.DataSourceInputStream
import androidx.media3.datasource.DataSpec
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.MediaRequest
import java.net.ServerSocket
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class MediaCacheTest {
    private val context
        get() = InstrumentationRegistry.getInstrumentation().targetContext

    private class Fixture(val bytes: ByteArray) : java.io.Closeable {
        private val server = ServerSocket(0)
        val url = "http://127.0.0.1:${server.localPort}/media"
        val count = AtomicInteger()
        val requests = java.util.concurrent.CopyOnWriteArrayList<List<String>>()
        private val executor = Executors.newSingleThreadExecutor()

        init {
            executor.submit {
                try {
                    while (!server.isClosed) runCatching {
                        server.accept().use { socket ->
                            socket.soTimeout = 5000
                            val reader = socket.getInputStream().bufferedReader()
                            val headers = mutableListOf<String>()
                            while (true) {
                                val line = reader.readLine() ?: break
                                if (line.isEmpty()) break
                                headers += line
                            }
                            requests += headers
                            count.incrementAndGet()
                            val range =
                                headers
                                    .firstOrNull { it.startsWith("Range:", true) }
                                    ?.substringAfter("bytes=")
                            val start = range?.substringBefore('-')?.toIntOrNull() ?: 0
                            val end =
                                (range?.substringAfter('-')?.toIntOrNull() ?: bytes.lastIndex)
                                    .coerceAtMost(bytes.lastIndex)
                            val unchanged = headers.any {
                                it.equals("If-None-Match: fixture", true)
                            }
                            val output = socket.getOutputStream()
                            if (unchanged)
                                output.write(
                                    "HTTP/1.1 304 Not Modified\r\nETag: fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                        .toByteArray()
                                )
                            else {
                                val body = bytes.copyOfRange(start, end + 1)
                                val status = if (range == null) "200 OK" else "206 Partial Content"
                                val contentRange =
                                    if (range == null) ""
                                    else "Content-Range: bytes $start-$end/${bytes.size}\r\n"
                                output.write(
                                    "HTTP/1.1 $status\r\n${contentRange}Content-Type: application/octet-stream\r\nETag: fixture\r\nContent-Length: ${body.size}\r\nConnection: close\r\n\r\n"
                                        .toByteArray()
                                )
                                output.write(body)
                            }
                        }
                    }
                } catch (_: java.net.SocketException) {}
            }
        }

        fun source(account: String = "public") =
            MediaRequest(url, emptyMap(), "$account/$url", url.substringBefore("/media"))

        override fun close() {
            server.close()
            executor.shutdownNow()
        }
    }

    @Test
    fun imagesAndPlaybackSharePersistentBytesAndSeparateAccounts() = runBlocking {
        val directory = context.cacheDir.resolve("shared-media-${System.nanoTime()}")
        val fixture = context.assets.open("demo/portrait.png").use { it.readBytes() }
        try {
            Fixture(fixture).use { server ->
                val source = server.source()
                MediaCache.isolated(context, directory).use { cache ->
                    assertArrayEquals(fixture, cache.images(source).toList().single())
                    val requests = server.count.get()
                    DataSourceInputStream(
                            cache.factory(source).createDataSource(),
                            DataSpec.Builder()
                                .setUri(source.url)
                                .setPosition(10)
                                .setLength(64)
                                .build(),
                        )
                        .use { assertArrayEquals(fixture.copyOfRange(10, 74), it.readBytes()) }
                    assertEquals(
                        "A player reads the image-filled cache",
                        requests,
                        server.count.get(),
                    )
                }
                MediaCache.isolated(context, directory).use { cache ->
                    assertArrayEquals(fixture, cache.images(source).toList().single())
                    assertEquals("Reopening uses saved bytes", 1, server.count.get())
                    assertArrayEquals(
                        fixture,
                        cache.images(server.source("other-account")).toList().single(),
                    )
                    assertEquals("Account identity is part of the cache key", 2, server.count.get())
                }
                MediaCache.isolated(context, directory, freshFor = 0).use { cache ->
                    assertArrayEquals(fixture, cache.images(source).first())
                    assertArrayEquals(fixture, cache.images(source).toList().single())
                    assertTrue(
                        server.requests.last().any { it.equals("If-None-Match: fixture", true) }
                    )
                }
                assertTrue(
                    server.requests.flatten().none {
                        it.startsWith("Cookie:", true) || it.startsWith("Authorization:", true)
                    }
                )
            }
        } finally {
            directory.deleteRecursively()
        }
    }

    @Test
    fun partialStreamingAndNativeBlobsUseTheSameBoundedLruStore() = runBlocking {
        val directory = context.cacheDir.resolve("shared-ranges-${System.nanoTime()}")
        try {
            Fixture(ByteArray(32 * 1024) { (it % 251).toByte() }).use { server ->
                MediaCache.isolated(context, directory, budget = 96 * 1024L).use { cache ->
                    val source = server.source()
                    val spec =
                        DataSpec.Builder()
                            .setUri(source.url)
                            .setPosition(100)
                            .setLength(1024)
                            .build()
                    repeat(2) {
                        DataSourceInputStream(cache.factory(source).createDataSource(), spec).use {
                            assertArrayEquals(server.bytes.copyOfRange(100, 1124), it.readBytes())
                        }
                    }
                    assertEquals(
                        "Only the requested video/audio range is fetched",
                        1,
                        server.count.get(),
                    )
                    assertTrue(cache.size < server.bytes.size)
                    val playlist = source.copy(url = source.url + ".m3u8", cacheKey = "playlist")
                    repeat(2) {
                        DataSourceInputStream(
                                cache.factory(playlist).createDataSource(),
                                DataSpec.Builder().setUri(playlist.url).build(),
                            )
                            .use { it.readBytes() }
                    }
                    assertEquals(
                        "Live playlists are refreshed instead of frozen in the cache",
                        3,
                        server.count.get(),
                    )
                    repeat(8) { i ->
                        cache.blob("native/avatar-or-sticker/$i") {
                            ByteArray(24 * 1024) { i.toByte() }
                        }
                    }
                    assertTrue(
                        "Native image bytes and streamed ranges share the disk budget",
                        cache.size <= 96 * 1024,
                    )
                    var loaded = false
                    cache.blob("native/avatar-or-sticker/7") {
                        loaded = true
                        byteArrayOf(1)
                    }
                    assertFalse("A recent native blob is retained", loaded)
                    cache.blob("native/avatar-or-sticker/0") {
                        loaded = true
                        byteArrayOf(1)
                    }
                    assertTrue("Old media is evicted regardless of its type", loaded)
                }
            }
        } finally {
            directory.deleteRecursively()
        }
    }

    @Test
    fun clearingSharedMediaForcesRefetchWithoutDeletingSavedFiles() = runBlocking {
        val directory = context.cacheDir.resolve("clear-media-${System.nanoTime()}")
        val saved =
            context.filesDir.resolve("saved-fixture-${System.nanoTime()}").apply {
                writeText("saved")
            }
        try {
            MediaCache.isolated(context, directory).use { cache ->
                cache.blob("sticker") { byteArrayOf(1, 2, 3) }
                assertTrue(cache.size > 0)
                cache.clear()
                assertEquals(0L, cache.size)
                assertEquals("saved", saved.readText())
                var reloaded = false
                assertArrayEquals(
                    byteArrayOf(4),
                    cache.blob("sticker") {
                        reloaded = true
                        byteArrayOf(4)
                    },
                )
                assertTrue(reloaded)
            }
        } finally {
            directory.deleteRecursively()
            saved.delete()
        }
    }
}
