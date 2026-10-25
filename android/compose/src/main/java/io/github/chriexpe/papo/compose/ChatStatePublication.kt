package io.github.chriexpe.papo.compose

import io.github.chriexpe.papo.core.ReadingView
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/** Reading progress has its own owner/channel stream; it is not timeline content. */
internal class ChatStatePublication {
    private data class Scope(val url: String, val owner: String, val channel: String)

    private fun ServerState.scope() = Scope(url, me, selected)

    private val readings = ConcurrentHashMap<Scope, MutableStateFlow<ReadingView>>()
    private val application = MutableStateFlow(PapoState())
    val state = application.asStateFlow()

    fun reading(server: ServerState): StateFlow<ReadingView> =
        readings.getOrPut(server.scope()) { MutableStateFlow(server.reading) }

    /** Called by the core actor, including full polls and viewport reductions. */
    fun publish(next: PapoState): Boolean {
        val scopes = next.servers.map { it.scope() }.toSet()
        readings.keys.retainAll(scopes)
        next.servers.forEach { server ->
            readings.getOrPut(server.scope()) { MutableStateFlow(server.reading) }.value =
                server.reading
        }
        val previous = application.value
        val oldServers = previous.servers.associateBy { it.url }
        val content =
            next.copy(
                servers =
                    next.servers.map { server ->
                        val old = oldServers[server.url]
                        // UniFFI includes reading in its snapshot record. Treat that field as the
                        // stream's seed, keeping the last content snapshot when only reading
                        // changes.
                        if (
                            old != null &&
                                old.me == server.me &&
                                old.selected == server.selected &&
                                server.copy(reading = old.reading) == old
                        )
                            old
                        else server
                    }
            )
        if (content == previous) return false
        application.value = content
        return true
    }

    fun clearError() {
        application.update { it.copy(error = "") }
    }
}
