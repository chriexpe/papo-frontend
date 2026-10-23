package io.github.chriexpe.papo.compose

import android.content.Context
import java.io.DataInputStream
import java.net.ServerSocket
import java.net.Socket
import java.security.MessageDigest
import java.time.Instant
import java.util.Base64
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors
import org.json.JSONArray
import org.json.JSONObject

/**
 * Real REST + WebSocket fixture: received messages pass through the network worker, reducer,
 * UniFFI, actor and Compose, rather than pretending own sends have the same read semantics as
 * another person's arrivals.
 */
internal class IncomingChatFixture(
    private val context: Context,
    private val ownerIsMe: Boolean = false,
) : AutoCloseable {
    private val listener = ServerSocket(0)
    val url = "http://127.0.0.1:" + listener.localPort
    private val workers = Executors.newCachedThreadPool()
    private val sockets = CopyOnWriteArrayList<Socket>()
    private val peers = CopyOnWriteArrayList<Socket>()
    val requests = CopyOnWriteArrayList<String>()
    val sentBodies = CopyOnWriteArrayList<String>()
    val failures = CopyOnWriteArrayList<String>()
    private val messages = mutableListOf<JSONObject>()
    private val start = Instant.now().minusSeconds(120)
    @Volatile private var running = true
    @Volatile private var acknowledged = "m30"

    init {
        repeat(30) { messages.add(message("m" + (it + 1), "Earlier message " + (it + 1), it + 1)) }
        workers.execute {
            while (running) {
                val socket =
                    try {
                        listener.accept()
                    } catch (_: Exception) {
                        break
                    }
                sockets.add(socket)
                workers.execute {
                    runCatching { serve(socket) }
                        .onFailure { failures.add(it.stackTraceToString()) }
                    peers.remove(socket)
                    sockets.remove(socket)
                    socket.close()
                }
            }
        }
    }

    private fun message(id: String, text: String, second: Int) =
        JSONObject()
            .put("id", id)
            .put("channel_id", "incoming")
            .put("author_id", "other")
            .put("content", text)
            .put("created_at", start.plusSeconds(second.toLong()).toString())

    fun incoming(id: String, text: String, portrait: Boolean = false) {
        val value =
            synchronized(messages) {
                message(id, text, 31 + messages.size).also {
                    if (portrait) it.put("embeds", JSONArray().put(imageEmbed(id)))
                    messages.add(it)
                }
            }
        check(peers.isNotEmpty()) { "WebSocket is not connected" }
        broadcast(JSONObject(value.toString()).put("type", "message"))
    }

    fun expandWithPortrait(id: String) {
        synchronized(messages) {
            messages
                .single { it.getString("id") == id }
                .put("embeds", JSONArray().put(imageEmbed(id)))
        }
        broadcast(
            JSONObject()
                .put("type", "message_embeds_update")
                .put("channel_id", "incoming")
                .put("message_id", id)
                .put("embeds", JSONArray().put(imageEmbed(id)))
        )
    }

    private fun imageEmbed(id: String) =
        JSONObject()
            .put("id", "image-" + id)
            .put("url", "https://example.test/portrait.png")
            .put(
                "image",
                JSONObject().put("mime_type", "image/png").put("width", 540).put("height", 960),
            )

    private fun broadcast(value: JSONObject) {
        peers.forEach { frame(it, 1, value.toString().toByteArray()) }
    }

    private fun frame(socket: Socket, opcode: Int, bytes: ByteArray) =
        synchronized(socket) {
            val out = socket.getOutputStream()
            out.write(0x80 or opcode)
            if (bytes.size < 126) out.write(bytes.size)
            else {
                out.write(126)
                out.write(bytes.size shr 8)
                out.write(bytes.size and 255)
            }
            out.write(bytes)
            out.flush()
        }

    private fun serve(socket: Socket) {
        val input = DataInputStream(socket.getInputStream())
        fun line(): String {
            val bytes = java.io.ByteArrayOutputStream()
            while (true) {
                val b = input.read()
                if (b < 0 || b == 10) break
                if (b != 13) bytes.write(b)
            }
            return bytes.toString("UTF-8")
        }
        val request = line().split(' ')
        if (request.size < 2) return
        val method = request[0]
        val path = request[1].substringBefore('?')
        val headers = mutableMapOf<String, String>()
        while (true) {
            val h = line()
            if (h.isEmpty()) break
            headers[h.substringBefore(':').lowercase()] = h.substringAfter(':').trim()
        }
        val body = ByteArray(headers["content-length"]?.toInt() ?: 0)
        input.readFully(body)
        requests.add(method + " " + request[1])
        if (headers["upgrade"]?.lowercase() == "websocket") {
            val accept =
                Base64.getEncoder()
                    .encodeToString(
                        MessageDigest.getInstance("SHA-1")
                            .digest(
                                (headers.getValue("sec-websocket-key") +
                                        "258EAFA5-E914-47DA-95CA-C5AB0DC85B11")
                                    .toByteArray()
                            )
                    )
            socket
                .getOutputStream()
                .write(
                    ("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: " +
                            accept +
                            "\r\n\r\n")
                        .toByteArray()
                )
            socket.getOutputStream().flush()
            peers.add(socket)
            while (running) {
                val first = input.readUnsignedByte()
                val second = input.readUnsignedByte()
                var length = second and 127
                if (length == 126) length = input.readUnsignedShort()
                if (length == 127) length = input.readLong().toInt()
                require(length in 0..1_048_576)
                val mask =
                    if (second and 128 != 0) ByteArray(4).also { input.readFully(it) } else null
                val payload = ByteArray(length)
                input.readFully(payload)
                if (mask != null)
                    payload.indices.forEach {
                        payload[it] = (payload[it].toInt() xor mask[it % 4].toInt()).toByte()
                    }
                when (first and 15) {
                    8 -> return
                    9 -> frame(socket, 10, payload)
                    1 ->
                        if (
                            JSONObject(payload.toString(Charsets.UTF_8)).optString("type") ==
                                "heartbeat"
                        )
                            frame(socket, 1, "{\"type\":\"heartbeat_ack\"}".toByteArray())
                }
            }
            return
        }
        var status = "200 OK"
        var mime = "application/json"
        val response: ByteArray =
            when {
                path == "/messages" && method == "POST" -> {
                    sentBodies.add(body.toString(Charsets.UTF_8))
                    message("sent-${sentBodies.size}", "sent reply", 90 + sentBodies.size)
                        .put("author_id", "me")
                        .toString()
                        .toByteArray()
                }
                path == "/auth/login" ->
                    "{\"user\":{\"id\":\"me\",\"username\":\"reader\"}}".toByteArray()
                path == "/auth/whoami" ->
                    """{"id":"me","username":"reader","nickname":"Reader","settings":{"version":1,"config":{}}}"""
                        .toByteArray()
                path == "/auth/refresh" -> "{}".toByteArray()
                path == "/server" ->
                    JSONObject()
                        .put("id", "fixture")
                        .put("name", "Incoming fixture")
                        .put("public", true)
                        .put("owner_id", if (ownerIsMe) "me" else "other")
                        .toString()
                        .toByteArray()
                path == "/channels" ->
                    JSONObject()
                        .put(
                            "channels",
                            JSONArray()
                                .put(
                                    JSONObject()
                                        .put("id", "incoming")
                                        .put("name", "incoming")
                                        .put("type", "text")
                                        .put("last_read_message", acknowledged)
                                        .put(
                                            "last_message",
                                            synchronized(messages) { messages.last() },
                                        )
                                ),
                        )
                        .toString()
                        .toByteArray()
                path == "/channels/incoming/messages" -> {
                    val values =
                        synchronized(messages) { messages.map { JSONObject(it.toString()) } }
                    JSONObject()
                        .put("messages", JSONArray(values))
                        .put("has_more", false)
                        .toString()
                        .toByteArray()
                }
                path == "/channels/incoming/read" -> {
                    acknowledged = JSONObject(body.toString(Charsets.UTF_8)).getString("message_id")
                    "{}".toByteArray()
                }
                path == "/channels/incoming/pinned" -> "{\"pinned\":[]}".toByteArray()
                path == "/dms" -> "{\"dms\":[]}".toByteArray()
                path == "/users/blocks" -> "{\"users\":[]}".toByteArray()
                path == "/users/other/reset" && method == "POST" && ownerIsMe ->
                    JSONObject()
                        .put(
                            "reset_url",
                            "https://papo.cyberasilo.online/passwordchange/fixture_token",
                        )
                        .put("expires_at", Instant.now().plusSeconds(3600).toString())
                        .toString()
                        .toByteArray()
                path == "/users" ->
                    """{"users":[{"id":"me","username":"reader"},{"id":"other","username":"sender"}]}"""
                        .toByteArray()
                path == "/users/profile_batch" -> "{\"profiles\":[]}".toByteArray()
                path == "/roles" -> "{\"roles\":[]}".toByteArray()
                path == "/emojis" -> "{\"emojis\":[],\"has_more\":false}".toByteArray()
                path.endsWith("/notifications") -> "{\"notifications\":[]}".toByteArray()
                path.contains("/embeds/") || path.startsWith("/media/") -> {
                    mime = "image/png"
                    context.assets.open("demo/portrait.png").use { it.readBytes() }
                }
                else -> {
                    status = "404 Not Found"
                    "{}".toByteArray()
                }
            }
        val cookie =
            if (path == "/auth/login") "Set-Cookie: Auth=incoming-fixture; Path=/; HttpOnly\r\n"
            else ""
        val header =
            "HTTP/1.1 " +
                status +
                "\r\n" +
                cookie +
                "Content-Type: " +
                mime +
                "\r\nContent-Length: " +
                response.size +
                "\r\nConnection: close\r\n\r\n"
        socket.getOutputStream().write(header.toByteArray())
        socket.getOutputStream().write(response)
        socket.getOutputStream().flush()
    }

    override fun close() {
        running = false
        listener.close()
        sockets.forEach { runCatching { it.close() } }
        workers.shutdownNow()
    }
}
