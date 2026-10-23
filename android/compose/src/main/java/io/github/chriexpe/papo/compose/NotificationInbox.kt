package io.github.chriexpe.papo.compose

import androidx.compose.runtime.*
import io.github.chriexpe.papo.core.InboxItem
import io.github.chriexpe.papo.core.InboxPage
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch

internal data class InboxOwner(val url: String, val user: String)

internal fun ServerState.inboxOwner() = InboxOwner(url, me)

internal data class NotificationPage(
    val epoch: Long,
    val data: InboxPage? = null,
    val loading: Boolean = false,
    val error: String = "",
    val failedMore: Boolean = false,
) {
    val items
        get() = data?.items.orEmpty()

    val hasMore
        get() = data?.hasMore == true
}

internal data class ScopedNotice(val server: ServerState, val item: InboxItem)

/**
 * UI request ownership is independent per account. A live refresh queues behind pagination;
 * cancellation/account replacement cannot publish into a new page.
 */
@Stable
internal class NotificationInbox(
    private val model: PapoViewModel,
    private val scope: CoroutineScope,
) {
    private val notices = Channel<String>(Channel.BUFFERED)
    val messages = notices.receiveAsFlow()

    private suspend fun report(message: String) {
        notices.send(message)
    }

    private var visible = false
    private val dirty = mutableSetOf<InboxOwner>()

    fun show(current: List<ServerState>) {
        visible = true
        dirty.addAll(current.map { it.inboxOwner() })
        synchronize(current)
    }

    fun hide() {
        visible = false
    }

    val pages = mutableStateMapOf<InboxOwner, NotificationPage>()
    var reading by mutableStateOf<Set<Pair<InboxOwner, String>>>(emptySet())
        private set

    var marking by mutableStateOf(false)
        private set

    private var serial = 0L
    private var servers = emptyMap<InboxOwner, ServerState>()
    private val counts = mutableMapOf<InboxOwner, Pair<UInt, ULong>>()
    private val jobs = mutableMapOf<InboxOwner, Job>()
    private val refreshPending = mutableSetOf<InboxOwner>()

    fun synchronize(current: List<ServerState>) {
        servers = current.associateBy { it.inboxOwner() }
        (pages.keys - servers.keys).forEach { owner ->
            pages.remove(owner)
            jobs.remove(owner)?.cancel()
            counts.remove(owner)
            refreshPending.remove(owner)
            dirty.remove(owner)
        }
        reading = reading.filter { it.first in servers }.toSet()
        current.forEach { server ->
            val owner = server.inboxOwner()
            val version = server.notificationCount to server.inboxRevision
            val changed = counts.put(owner, version) != version
            if (owner !in pages) pages[owner] = NotificationPage(++serial)
            if (changed) dirty.add(owner)
            if (visible && dirty.remove(owner)) load(owner, false)
        }
    }

    fun retry(owner: InboxOwner) = load(owner, pages[owner]?.failedMore == true)

    fun more(owner: InboxOwner) {
        if (pages[owner]?.hasMore == true) load(owner, true)
    }

    private fun load(owner: InboxOwner, more: Boolean) {
        val server = servers[owner] ?: return
        val before = pages[owner] ?: return
        if (before.loading) {
            if (!more) refreshPending.add(owner)
            return
        }
        pages[owner] = before.copy(loading = true, error = "", failedMore = more)
        jobs[owner] = scope.launch {
            try {
                val result = model.inboxPage(server, more)
                val current = pages[owner]?.takeIf { it.epoch == before.epoch } ?: return@launch
                // An ACK can return to Main before an older page result. Read
                // flags are monotonic in both native storage and presentation.
                val read = current.items.filter { it.read }.map { it.id }.toSet()
                pages[owner] =
                    current.copy(
                        data =
                            result.copy(
                                items =
                                    result.items.map { it.copy(read = it.read || it.id in read) }
                            ),
                        loading = false,
                        error = "",
                    )
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                pages[owner]
                    ?.takeIf { it.epoch == before.epoch }
                    ?.let {
                        pages[owner] =
                            it.copy(
                                loading = false,
                                error = ui(R.string.inbox_load_failed),
                                failedMore = more,
                            )
                    }
            } finally {
                pages[owner]
                    ?.takeIf { it.epoch == before.epoch }
                    ?.let { pages[owner] = it.copy(loading = false) }
                if (pages[owner]?.epoch == before.epoch && refreshPending.remove(owner)) {
                    if (visible) load(owner, false) else dirty.add(owner)
                }
            }
        }
    }

    fun readOne(owner: InboxOwner, id: String) {
        if (marking || (owner to id) in reading) return
        val server = servers[owner] ?: return
        val epoch = pages[owner]?.epoch ?: return
        reading = reading + (owner to id)
        scope.launch {
            try {
                acknowledge(server, epoch, listOf(id))
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                if (pages[owner]?.epoch == epoch)
                    report(ui(R.string.couldn_t_mark_notification_as_read))
            } finally {
                reading = reading - (owner to id)
            }
        }
    }

    fun markLoaded() {
        if (marking || reading.isNotEmpty()) return
        val captured = pages.mapNotNull { (owner, page) ->
            val server = servers[owner] ?: return@mapNotNull null
            val ids = page.items.filter { !it.read }.map { it.id }
            Triple(server, page.epoch, ids).takeIf { ids.isNotEmpty() }
        }
        if (captured.isEmpty()) return
        marking = true
        reading = captured.flatMap { (s, _, ids) -> ids.map { s.inboxOwner() to it } }.toSet()
        scope.launch {
            var marked = 0
            var failed = 0
            try {
                captured.forEach { (server, epoch, ids) ->
                    var completed = 0
                    try {
                        for (batch in ids.chunked(1000)) {
                            if (pages[server.inboxOwner()]?.epoch != epoch) break
                            acknowledge(server, epoch, batch)
                            completed += batch.size
                        }
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        /* Keep each prior ACK; continue with other servers. */
                    }
                    if (pages[server.inboxOwner()]?.epoch == epoch) {
                        marked += completed
                        failed += ids.size - completed
                    }
                }
                if (failed > 0) report(ui(R.string.inbox_read_partial, marked, failed))
                else if (marked > 0) report(uiQuantity(R.plurals.inbox_marked_count, marked))
            } finally {
                marking = false
                reading = emptySet()
            }
        }
    }

    private suspend fun acknowledge(server: ServerState, epoch: Long, ids: List<String>) {
        model.readInbox(server, ids)
        val owner = server.inboxOwner()
        val current = pages[owner]?.takeIf { it.epoch == epoch } ?: return
        pages[owner] =
            current.copy(
                data =
                    current.data?.copy(
                        items = current.items.map { if (it.id in ids) it.copy(read = true) else it }
                    )
            )
    }
}
