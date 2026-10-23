package io.github.chriexpe.papo.compose

import android.content.Context
import androidx.compose.runtime.*
import io.github.chriexpe.papo.core.AppearanceChange
import io.github.chriexpe.papo.core.AppearanceException
import io.github.chriexpe.papo.core.AppearanceView
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

internal data class AppearanceOwner(val server: String, val user: String)

internal fun ServerState.appearanceOwner() = AppearanceOwner(url, me)

private fun appearanceHash(value: String) =
    java.security.MessageDigest.getInstance("SHA-256").digest(value.toByteArray()).joinToString(
        ""
    ) {
        "%02x".format(it)
    }

internal fun appearanceServerPrefix(server: String) =
    "account-appearance/${appearanceHash(server)}/"

internal fun AppearanceOwner.cachePrefix() =
    appearanceServerPrefix(server) + appearanceHash(user) + "/"

internal data class AccountAppearanceStatus(
    val busy: Boolean = false,
    val loaded: Boolean = false,
    val error: Int? = null,
    val failedChange: AppearanceChange? = null,
)

/** Requests belong to the ViewModel, so leaving settings cannot cancel an acknowledged save. */
@Stable
internal class AccountAppearanceCoordinator(
    context: Context,
    private val scope: CoroutineScope,
    private val current: (ServerState) -> Boolean,
    private val apply: suspend (ServerState, AppearanceChange?) -> AppearanceView,
) {
    private val storage = context.getSharedPreferences("compose-appearance", 0)
    val states = mutableStateMapOf<AppearanceOwner, AccountAppearanceStatus>()
    private val jobs = mutableMapOf<AppearanceOwner, Job>()
    private val tickets = mutableMapOf<AppearanceOwner, Long>()
    private var serial = 0L

    fun synchronize(servers: List<ServerState>) {
        val owners =
            servers
                .filter { it.me.isNotBlank() && it.screen == "chat" }
                .map { it.appearanceOwner() }
                .toSet()
        (states.keys - owners).forEach { owner ->
            jobs.remove(owner)?.cancel()
            tickets.remove(owner)
            states.remove(owner)
        }
    }

    fun forget(server: String) {
        states.keys
            .filter { it.server == server }
            .toList()
            .forEach {
                jobs.remove(it)?.cancel()
                tickets.remove(it)
                states.remove(it)
            }
        val editor = storage.edit()
        storage.all.keys
            .filter { it.startsWith(appearanceServerPrefix(server)) }
            .forEach(editor::remove)
        check(editor.commit())
    }

    fun refresh(server: ServerState, force: Boolean = false) {
        if (!force && states[server.appearanceOwner()]?.loaded == true) return
        request(server, null)
    }

    fun change(server: ServerState, change: AppearanceChange) = request(server, change)

    fun retry(server: ServerState) = request(server, states[server.appearanceOwner()]?.failedChange)

    private fun request(server: ServerState, change: AppearanceChange?) {
        val owner = server.appearanceOwner()
        if (!current(server) || states[owner]?.busy == true) return
        val previous = states[owner] ?: AccountAppearanceStatus()
        val ticket = ++serial
        tickets[owner] = ticket
        states[owner] = previous.copy(busy = true, error = null)
        jobs[owner] = scope.launch {
            try {
                val prefix = owner.cachePrefix()
                val import =
                    !storage.getBoolean(prefix + "migrated", false) &&
                        storage.getBoolean("compact", false)
                var result =
                    apply(server, if (import) AppearanceChange.ImportLegacyCompact else change)
                if (import && change != null) result = apply(server, change)
                // An owner/token check already guarded the native ACK. Recheck the UI owner
                // before writing its offline cache or migration marker after navigation/logout.
                if (!current(server) || tickets[owner] != ticket || owner !in states) return@launch
                check(
                    storage
                        .edit()
                        .putString(prefix + "font", result.fontSize)
                        .putString(prefix + "density", result.messageDensity)
                        .putBoolean(prefix + "migrated", true)
                        .commit()
                )
                states[owner] = AccountAppearanceStatus(loaded = true)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                if (current(server) && tickets[owner] == ticket && owner in states) {
                    states[owner] =
                        previous.copy(error = appearanceErrorResource(error), failedChange = change)
                }
            } finally {
                if (tickets[owner] == ticket && states[owner]?.busy == true)
                    states[owner] = states.getValue(owner).copy(busy = false)
            }
        }
    }
}

private fun appearanceErrorResource(error: Exception) =
    when (error) {
        is AppearanceException.Network -> R.string.appearance_network_error
        is AppearanceException.RateLimited -> R.string.appearance_rate_limited
        is AppearanceException.AccountChanged -> R.string.account_changed
        is AppearanceException.Unavailable -> R.string.appearance_unavailable
        is AppearanceException.Incompatible -> R.string.appearance_incompatible
        else -> R.string.appearance_update_failed
    }
