package io.github.chriexpe.papo.compose

import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.chriexpe.papo.core.MediaRequest
import java.io.IOException
import java.net.ServerSocket
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import okhttp3.Request
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MediaNetworkTest {
    @Test
    fun rangeRequestsKeepAuthenticationOnTheirOriginAndDoNotFollowRedirects() {
        ServerSocket(0).use { server ->
            server.soTimeout = 5000
            val origin = "http://127.0.0.1:${server.localPort}"
            val source =
                MediaRequest(
                    "$origin/embeds/movie/video",
                    mapOf("Cookie" to "Auth=fixture-token"),
                    "fixture/account/movie",
                    origin,
                )
            val executor = Executors.newSingleThreadExecutor()
            try {
                val received =
                    executor.submit<List<String>> {
                        server.accept().use { socket ->
                            socket.soTimeout = 5000
                            val reader = socket.getInputStream().bufferedReader()
                            val lines = mutableListOf<String>()
                            while (true) {
                                val line = reader.readLine() ?: break
                                if (line.isEmpty()) break
                                lines += line
                            }
                            socket
                                .getOutputStream()
                                .write(
                                    "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 4-6/10\r\nConnection: close\r\n\r\n456"
                                        .toByteArray()
                                )
                            lines
                        }
                    }
                val client = MediaNetwork.client(source)
                client
                    .newCall(Request.Builder().url(source.url).header("Range", "bytes=4-6").build())
                    .execute()
                    .use {
                        assertEquals(206, it.code)
                        assertEquals("456", it.body!!.string())
                    }
                val headers = received.get(5, TimeUnit.SECONDS)
                assertTrue(
                    headers.any { it.equals("Cookie: Auth=fixture-token", ignoreCase = true) }
                )
                assertTrue(headers.any { it.equals("Range: bytes=4-6", ignoreCase = true) })
                try {
                    client
                        .newCall(
                            Request.Builder()
                                .url("http://localhost:${server.localPort}/foreign")
                                .build()
                        )
                        .execute()
                        .close()
                    fail("An authenticated media request must reject a different origin")
                } catch (_: IOException) {}
                val redirect = executor.submit {
                    server.accept().use { socket ->
                        val reader = socket.getInputStream().bufferedReader()
                        while (!reader.readLine().isNullOrEmpty()) {}
                        socket
                            .getOutputStream()
                            .write(
                                "HTTP/1.1 302 Found\r\nLocation: http://localhost:${server.localPort}/foreign\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                    .toByteArray()
                            )
                    }
                }
                client.newCall(Request.Builder().url(source.url).build()).execute().use {
                    assertEquals(302, it.code)
                }
                redirect.get(5, TimeUnit.SECONDS)
            } finally {
                executor.shutdownNow()
            }
        }
    }
}
