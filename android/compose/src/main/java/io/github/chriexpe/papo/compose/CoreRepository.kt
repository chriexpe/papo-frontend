package io.github.chriexpe.papo.compose

import android.app.Application
import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.annotation.Keep
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.Credentials
import io.github.chriexpe.papo.core.MediaRequest
import io.github.chriexpe.papo.core.MediaResource
import io.github.chriexpe.papo.core.NativeClient
import io.github.chriexpe.papo.core.WakeListener
import java.security.KeyStore
import java.util.UUID
import java.util.concurrent.Executors
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.json.JSONArray

typealias ServerState = io.github.chriexpe.papo.core.ClientSnapshot

typealias Conversation = io.github.chriexpe.papo.core.ChannelView

typealias Person = io.github.chriexpe.papo.core.MemberView

typealias Reaction = io.github.chriexpe.papo.core.ReactionView

typealias ChatMessage = io.github.chriexpe.papo.core.MessageView

val ServerState.name
    get() = serverName
val ServerState.selected
    get() = selectedChannel
val ServerState.conversation
    get() = (channels + dms).firstOrNull { it.id == selected }
val ServerState.isDemo
    get() = url == "demo://papo"

class CoreWake(private val wake: () -> Unit) : WakeListener {
    override fun onWake() = wake()
}

/** Persists core credentials encrypted with an Android Keystore key. */
@Keep
class AndroidSecrets(context: Context) : Credentials {
    private val preferences = context.getSharedPreferences("core-secrets", Context.MODE_PRIVATE)
    private val key: SecretKey by lazy {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey("papo-core-v1", null) as? SecretKey)
            ?: KeyGenerator.getInstance(
                    KeyProperties.KEY_ALGORITHM_AES,
                    "AndroidKeyStore",
                )
                .apply {
                    init(
                        KeyGenParameterSpec.Builder(
                                "papo-core-v1",
                                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
                            )
                            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                            .build()
                    )
                }
                .generateKey()
    }

    @Synchronized
    override fun load(server: String, secret: String): String? {
        val id = "$server/$secret"
        val bytes =
            preferences.getString(id, null)?.let { Base64.decode(it, Base64.NO_WRAP) }
                ?: return null
        require(bytes.size >= 28) { ui(R.string.invalid_encrypted_credential) }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, bytes.copyOfRange(0, 12)))
        cipher.updateAAD(id.toByteArray(Charsets.UTF_8))
        return String(cipher.doFinal(bytes.copyOfRange(12, bytes.size)), Charsets.UTF_8)
    }

    @Synchronized
    override fun store(server: String, secret: String, value: String) {
        val id = "$server/$secret"
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key)
        cipher.updateAAD(id.toByteArray(Charsets.UTF_8))
        val bytes = cipher.iv + cipher.doFinal(value.toByteArray(Charsets.UTF_8))
        check(
            preferences.edit().putString(id, Base64.encodeToString(bytes, Base64.NO_WRAP)).commit()
        ) {
            ui(R.string.could_not_persist_credential)
        }
    }

    @Synchronized
    override fun remove(server: String, secret: String) {
        check(preferences.edit().remove("$server/$secret").commit()) {
            ui(R.string.could_not_remove_credential)
        }
    }
}

data class PapoState(
    val servers: List<ServerState> = emptyList(),
    val activeUrl: String = "",
    val error: String = "",
    val opening: Boolean = false,
) {
    val active
        get() = servers.firstOrNull { it.url == activeUrl }
}

class PapoViewModel(application: Application) : AndroidViewModel(application) {
    // Handles, snapshots and active-server selection belong to this one dispatcher. Slow native
    // calls first capture an account-scoped handle here, then run on IO and revalidate in core.
    private val executor = Executors.newSingleThreadExecutor { r -> Thread(r, "papo-core-actor") }
    private val dispatcher = executor.asCoroutineDispatcher()
    private val signals = Channel<Unit>(Channel.CONFLATED)
    private val preferences =
        application.getSharedPreferences("compose-client", Context.MODE_PRIVATE)
    private val secrets = AndroidSecrets(application)
    private val handles = linkedMapOf<String, NativeClient>()
    internal val mediaCache = MediaCache.get(application)
    private val waveforms = AudioWaveforms(application, viewModelScope)
    private val snapshots = linkedMapOf<String, ServerState>()
    private val mutableState = MutableStateFlow(PapoState())
    val state = mutableState.asStateFlow()
    private val forgottenServers = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()
    private var activeUrl = preferences.getString("active-server", "").orEmpty()
    private val connectivity = application.getSystemService(ConnectivityManager::class.java)
    private var networkEpoch = 0L
    private var lastTyping = 0L
    private var readRetry: kotlinx.coroutines.Job? = null
    private val pushJobs = mutableMapOf<String, kotlinx.coroutines.Job>()
    private val pushApplied = mutableMapOf<String, Triple<String, String, Boolean>>()
    private val pushRetryAt = mutableMapOf<String, Long>()
    private var notificationWorkState: List<Any> = emptyList()
    @Volatile private var visibleConversationUrl = ""
    private val pendingNotification = MutableStateFlow<NotificationTarget?>(null)
    internal val notificationTarget = pendingNotification.asStateFlow()
    private var callbackRegistered = false

    private data class PendingUpload(
        val server: String,
        val receipt: CompletableDeferred<UploadReceipt>,
    )

    private data class PendingMessage(val server: String, val callback: (Boolean) -> Unit)

    private val awaitingUploads = mutableMapOf<String, PendingUpload>()
    internal val expressions = ExpressionLibrary(getApplication())
    internal val attachments = AttachmentDrafts(application, viewModelScope, ::uploadFiles)
    private val awaitingPersistence = mutableMapOf<String, PendingMessage>()
    private var networkFingerprint: Pair<Long?, Boolean>? = null
    private val networkCallback =
        object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                networkChanged()
            }

            override fun onLost(network: Network) {
                networkChanged()
            }

            override fun onCapabilitiesChanged(
                network: Network,
                capabilities: NetworkCapabilities,
            ) {
                networkChanged()
            }
        }

    init {
        MessageAlerts.start(application)
        viewModelScope.launch(dispatcher) {
            MessageAlerts.changes.collect { signals.trySend(Unit) }
        }
        viewModelScope.launch(dispatcher) {
            MessageAlerts.pushHints.collect { message ->
                snapshots.values
                    .filter { !it.isDemo && it.screen == "chat" && it.me.isNotBlank() }
                    .forEach { server ->
                        val handle = handles[server.url] ?: return@forEach
                        launch(dispatcher) {
                            try {
                                withContext(kotlinx.coroutines.Dispatchers.IO) {
                                    handle.reconcileAlerts(server.me, message)
                                }
                                poll()
                            } catch (e: kotlinx.coroutines.CancellationException) {
                                throw e
                            } catch (_: Exception) {
                                signals.trySend(Unit)
                            }
                        }
                    }
            }
        }
        viewModelScope.launch(dispatcher) {
            runCatching { importPreviousAndroid(application, secrets) }
                .onFailure {
                    publish(
                        error = it.message ?: ui(R.string.could_not_import_previous_android_data)
                    )
                }
            // Resume the tombstone before opening saved servers. Forgetting must
            // survive process death during IO cleanup without reconnecting first.
            var cleanupFailed = false
            preferences.all.keys
                .filter { it.startsWith("forgetting/") }
                .forEach { marker ->
                    val url = marker.removePrefix("forgetting/")
                    forgottenServers += url
                    try {
                        clearServerLocalData(url)
                        check(preferences.edit().remove(marker).commit())
                    } catch (error: kotlinx.coroutines.CancellationException) {
                        throw error
                    } catch (_: Exception) {
                        // Keep the tombstone for retry without preventing other servers from
                        // opening.
                        cleanupFailed = true
                    }
                }
            activeUrl = preferences.getString("active-server", activeUrl).orEmpty()
            val saved = runCatching {
                JSONArray(preferences.getString("servers", "[]"))
            }
                .getOrDefault(JSONArray())
            for (i in 0 until saved.length()) open(saved.getString(i), activate = false)
            if (activeUrl !in handles) activeUrl = handles.keys.firstOrNull().orEmpty()
            if (cleanupFailed) publish(error = ui(R.string.forget_server_failed)) else publish()
            signals.trySend(Unit)
            for (signal in signals) poll()
        }
        runCatching {
            connectivity.registerDefaultNetworkCallback(networkCallback)
            callbackRegistered = true
        }
    }

    private fun publish(error: String = mutableState.value.error, opening: Boolean = false) {
        mutableState.value = PapoState(snapshots.values.toList(), activeUrl, error, opening)
    }

    private fun open(input: String, activate: Boolean = true): Boolean =
        try {
            publish(error = "", opening = true)
            val normalized = io.github.chriexpe.papo.core.canonicalServerUrl(input)
            check(!preferences.contains("forgetting/$normalized")) {
                ui(R.string.server_cleanup_pending)
            }
            val path = messageDatabasePath(getApplication())
            val handle =
                NativeClient.openNotifying(
                    normalized,
                    path,
                    secrets,
                    CoreWake { signals.trySend(Unit) },
                    MessageAlerts.listener,
                )
            val server = handle.poll()
            mediaCache.allowServer(server.url)
            attachments.allowServer(server.url)
            expressions.allowServer(server.url)
            forgottenServers.remove(server.url)
            val old = handles[server.url]
            if (old != null) {
                handle.release()
            } else {
                handles[server.url] = handle
                snapshots[server.url] = server
            }
            if (activate) activeUrl = server.url
            preferences
                .edit()
                .putString(
                    "servers",
                    JSONArray(handles.keys.filter { it != "demo://papo" }).toString(),
                )
                .putString("active-server", activeUrl)
                .apply()
            publish(error = "")
            networkChanged(force = true)
            true
        } catch (e: Exception) {
            publish(error = e.message ?: ui(R.string.could_not_open_server))
            false
        }

    private fun poll() {
        try {
            for ((url, handle) in handles) {
                handle.notificationVisibility(url == visibleConversationUrl)
                val snapshot = handle.poll()
                snapshots[url] = snapshot
                if (!snapshot.isDemo) MessageAlerts.setOwner(url, snapshot.me)
                if (
                    snapshot.conversation?.unread == false &&
                        snapshot.reading.mentions.isEmpty() &&
                        snapshot.me.isNotBlank() &&
                        !snapshot.isDemo
                )
                    MessageAlerts.clear(url, snapshot.me, snapshot.selected)
                for (event in snapshot.events) {
                    awaitingUploads
                        .remove(event.localId)
                        ?.receipt
                        ?.complete(
                            UploadReceipt(
                                event.kind == "accepted",
                                event.kind == "unknown",
                                event.error,
                            )
                        )
                    val callback = awaitingPersistence.remove(event.localId)?.callback ?: continue
                    val accepted = event.kind == "accepted"
                    if (!accepted) publish(error = event.error)
                    viewModelScope.launch(kotlinx.coroutines.Dispatchers.Main) {
                        callback(accepted)
                    }
                }
                if (snapshot.more) signals.trySend(Unit)
            }
            publish()
            syncNotifications()
            if (snapshots.values.any { it.reading.syncPending } && readRetry?.isActive != true) {
                readRetry =
                    viewModelScope.launch(dispatcher) {
                        kotlinx.coroutines.delay(10_000)
                        readRetry = null
                        signals.trySend(Unit)
                    }
            }
        } catch (e: Exception) {
            publish(error = e.message ?: ui(R.string.could_not_update_messages))
        }
    }

    internal fun visibleConversation(url: String) {
        if (visibleConversationUrl == url) return
        visibleConversationUrl = url
        viewModelScope.launch(dispatcher) {
            handles.forEach { (key, client) -> client.notificationVisibility(key == url) }
        }
    }

    internal fun offerNotification(target: NotificationTarget) {
        pendingNotification.value = target
    }

    internal fun consumeNotification(target: NotificationTarget, error: String = "") {
        if (pendingNotification.value == target) pendingNotification.value = null
        if (error.isNotBlank()) publish(error = error)
    }

    private fun syncNotifications() {
        val available = MessageAlerts.listener.available()
        val token = MessageAlerts.token
        for ((url, handle) in handles) {
            val server = snapshots[url] ?: continue
            if (server.isDemo || server.screen != "chat" || server.me.isBlank()) continue
            val enabled = available && MessageAlerts.configured && server.notificationsEnabled
            val registered =
                MessageAlerts.registration(url)?.takeIf { it.first == server.me }?.second
            val value = if (enabled) token else registered ?: token
            if (value.isBlank()) continue
            val desired = Triple(server.me, value, enabled)
            if (
                pushApplied[url] == desired ||
                    pushJobs[url]?.isActive == true ||
                    android.os.SystemClock.elapsedRealtime() < (pushRetryAt[url] ?: 0)
            )
                continue
            pushJobs[url] =
                viewModelScope.launch(dispatcher) {
                    try {
                        withContext(kotlinx.coroutines.Dispatchers.IO) {
                            if (registered != null && registered != value)
                                handle.pushDevice(
                                    server.me,
                                    registered,
                                    android.os.Build.MODEL,
                                    false,
                                )
                            handle.pushDevice(server.me, value, android.os.Build.MODEL, enabled)
                        }
                        if (snapshots[url]?.me == server.me) {
                            MessageAlerts.confirm(url, server.me, value, enabled)
                            pushApplied[url] = desired
                        }
                        pushRetryAt.remove(url)
                    } catch (e: kotlinx.coroutines.CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        pushRetryAt[url] = android.os.SystemClock.elapsedRealtime() + 30_000
                        viewModelScope.launch(dispatcher) {
                            kotlinx.coroutines.delay(30_000)
                            signals.trySend(Unit)
                        }
                    } finally {
                        pushJobs.remove(url)
                        signals.trySend(Unit)
                    }
                }
        }
        val workState =
            listOf(available, token) +
                snapshots.values
                    .filter { !it.isDemo }
                    .map {
                        listOf(
                            it.url,
                            it.me,
                            it.screen,
                            it.pushRegistered,
                            it.notificationsEnabled,
                            MessageAlerts.registered(it.url, it.me),
                        )
                    }
        if (workState != notificationWorkState) {
            notificationWorkState = workState
            MessageAlerts.schedule(snapshots.values.filter { it.notificationsEnabled })
        }
    }

    fun addServer(address: String, onSuccess: () -> Unit = {}) =
        viewModelScope.launch(dispatcher) {
            try {
                val url = io.github.chriexpe.papo.core.canonicalServerUrl(address)
                if (preferences.contains("forgetting/$url")) {
                    clearServerLocalData(url)
                    check(preferences.edit().remove("forgetting/$url").commit())
                }
                if (open(url)) launch(kotlinx.coroutines.Dispatchers.Main) { onSuccess() }
            } catch (error: kotlinx.coroutines.CancellationException) {
                throw error
            } catch (error: Exception) {
                publish(error = error.message ?: ui(R.string.could_not_open_server))
            }
        }

    fun exploreDemo() =
        viewModelScope.launch(dispatcher) {
            try {
                if ("demo://papo" !in handles) handles["demo://papo"] = NativeClient.demo()
                mediaCache.allowServer("demo://papo")
                attachments.allowServer("demo://papo")
                expressions.allowServer("demo://papo")
                forgottenServers.remove("demo://papo")
                activeUrl = "demo://papo"
                poll()
            } catch (e: Exception) {
                publish(error = e.message ?: ui(R.string.could_not_open_demo))
            }
        }

    fun leaveDemo() =
        viewModelScope.launch(dispatcher) {
            handles.remove("demo://papo")?.release()
            snapshots.remove("demo://papo")
            activeUrl = handles.keys.firstOrNull().orEmpty()
            publish(error = "")
        }

    /** Forget is local teardown, never an account-delete or remote logout request. */
    internal suspend fun forgetServer(server: ServerState) =
        withContext(dispatcher) {
            val client = handles[server.url]
            if (client == null && !preferences.contains("forgetting/${server.url}"))
                return@withContext
            val remaining = handles.keys.filter { it != server.url && it != "demo://papo" }
            val next =
                if (activeUrl == server.url) handles.keys.firstOrNull { it != server.url }.orEmpty()
                else activeUrl
            expressions.prepareForget(server)
            // Persist removal first so a process killed during cleanup cannot reconnect.
            check(
                preferences
                    .edit()
                    .putString("servers", JSONArray(remaining).toString())
                    .putString("active-server", next)
                    .putString("forgetting/${server.url}", server.key)
                    .commit()
            )
            forgottenServers += server.url
            pushJobs.remove(server.url)?.cancel()
            pushApplied.remove(server.url)
            pushRetryAt.remove(server.url)
            val token = MessageAlerts.registration(server.url)?.second
            MessageAlerts.forgetServer(server.url, server.key)
            if (pendingNotification.value?.server == server.url) pendingNotification.value = null
            withContext(kotlinx.coroutines.NonCancellable) {
                handles.remove(server.url)
                snapshots.remove(server.url)
                activeUrl = next
                awaitingUploads
                    .filterValues { it.server == server.url }
                    .keys
                    .toList()
                    .forEach {
                        awaitingUploads.remove(it)?.receipt?.cancel()
                    }
                awaitingPersistence
                    .filterValues { it.server == server.url }
                    .keys
                    .toList()
                    .forEach(awaitingPersistence::remove)
                publish(error = "")
                try {
                    withContext(kotlinx.coroutines.Dispatchers.IO) {
                        try {
                            client?.forgetServer(token)
                        } finally {
                            client?.close()
                        }
                    }
                    activeUrl = next
                    if (visibleConversationUrl == server.url) visibleConversationUrl = ""
                    publish(error = "")
                    syncNotifications()
                    clearServerLocalData(server.url)
                    check(preferences.edit().remove("forgetting/${server.url}").commit())
                } catch (error: Exception) {
                    // This screen may already be gone (last server); surface cleanup failures
                    // globally.
                    publish(error = ui(R.string.forget_server_failed))
                    throw error
                }
            }
        }

    private suspend fun clearServerLocalData(url: String) {
        attachments.forgetServer(url)
        withContext(kotlinx.coroutines.Dispatchers.IO) {
            if (!url.startsWith("demo:"))
                io.github.chriexpe.papo.core.forgetSavedServer(
                    url,
                    getApplication<Application>().filesDir.absolutePath,
                    messageDatabasePath(getApplication()),
                    secrets,
                )
            expressions.forgetServer(url)
            withContext(kotlinx.coroutines.Dispatchers.Main.immediate) {
                pendingGifs.keys
                    .filter { it.startsWith("$url/") }
                    .toList()
                    .forEach(pendingGifs::remove)
            }
            gifLocks.keys.filter { it.startsWith("$url/") }.toList().forEach(gifLocks::remove)
            mediaCache.forgetServer(url)
            androidx.work.WorkManager.getInstance(getApplication())
                .pruneWork()
                .result
                .get(10, java.util.concurrent.TimeUnit.SECONDS)
            val edit = preferences.edit()
            preferences.all.keys
                .filter { key ->
                    listOf("draft/", "draft-mentions/", "position/").any {
                        key.startsWith("$it$url/")
                    }
                }
                .forEach(edit::remove)
            check(edit.commit())
            val context = getApplication<Application>()
            listOf("image-editor", "attachment-options", "media-exports").forEach {
                context.cacheDir.resolve(it).deleteRecursively()
            }
            context.cacheDir
                .listFiles()
                .orEmpty()
                .filter { it.name.startsWith("download-image-") }
                .forEach { it.delete() }
        }
        waveforms.forgetAllMemory()
        withContext(kotlinx.coroutines.Dispatchers.Main.immediate) {
            MediaImages.loader(getApplication()).memoryCache?.clear()
            android.webkit.WebView(getApplication()).apply {
                clearCache(true)
                destroy()
            }
        }
    }

    fun activate(url: String) =
        viewModelScope.launch(dispatcher) {
            activeUrl = url
            preferences.edit().putString("active-server", url).apply()
            publish(error = "")
        }

    /**
     * Switch the active server and select the target in the same actor turn. Routes carry the
     * expected owner; a stale global row cannot enter a new account.
     */
    internal fun navigate(
        server: ServerState,
        action: Action,
        messageId: String? = null,
        onReady: () -> Unit,
    ) =
        viewModelScope.launch(dispatcher) {
            try {
                check(snapshots[server.url]?.me == server.me && server.me.isNotBlank()) {
                    ui(R.string.account_changed)
                }
                val client = handles[server.url] ?: error(ui(R.string.server_is_closed))
                client.dispatch(action)
                if (messageId != null && action is Action.SelectChannel)
                    client.dispatch(Action.JumpToMessage(action.channelId, messageId))
                activeUrl = server.url
                preferences.edit().putString("active-server", activeUrl).apply()
                poll()
                launch(kotlinx.coroutines.Dispatchers.Main) { onReady() }
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                publish(error = e.message ?: ui(R.string.couldn_t_open_conversation))
            }
        }

    internal suspend fun inbox(server: ServerState): List<io.github.chriexpe.papo.core.InboxItem> {
        val client = accountClient(server)
        val items = withContext(kotlinx.coroutines.Dispatchers.IO) { client.inbox(server.me) }
        withContext(dispatcher) { poll() }
        return items
    }

    internal suspend fun readInbox(server: ServerState, ids: List<String>) {
        val client = accountClient(server)
        withContext(kotlinx.coroutines.Dispatchers.IO) { client.inboxRead(server.me, ids) }
        withContext(dispatcher) { poll() }
    }

    fun command(
        action: Action,
        url: String = state.value.activeUrl,
        onFailure: () -> Unit = {},
        onSuccess: () -> Unit = {},
    ) =
        viewModelScope.launch(dispatcher) {
            try {
                val handle = handles[url] ?: error(ui(R.string.server_is_closed))
                handle.dispatch(action)
                poll()
                launch(kotlinx.coroutines.Dispatchers.Main) { onSuccess() }
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                publish(error = e.message ?: ui(R.string.could_not_complete_action))
                launch(kotlinx.coroutines.Dispatchers.Main) { onFailure() }
            }
        }

    internal fun publicMediaRequest(server: ServerState, url: String, gif: Boolean = false) =
        mediaCache.scoped(if (gif) publicGifSource(url) else publicMediaSource(url), server.url)

    suspend fun mediaRequest(url: String, resource: MediaResource): MediaRequest =
        withContext(dispatcher) {
            mediaCache.scoped(
                (handles[url] ?: error(ui(R.string.server_is_closed))).mediaRequest(resource),
                url,
            )
        }

    internal suspend fun analyzeSearch(
        server: ServerState,
        query: String,
        cursor: Int,
    ): io.github.chriexpe.papo.core.SearchAnalysis =
        withContext(dispatcher) {
            (handles[server.url] ?: error(ui(R.string.server_is_closed))).analyzeSearch(
                query,
                cursor.toUInt(),
            )
        }

    internal suspend fun serverIcon(server: ServerState): ByteArray =
        mediaCache.blob(
            "${server.url}/${server.me}/server-icon/${server.serverIconRevision}",
            server = server.url,
        ) {
            withContext(dispatcher) {
                (handles[server.url] ?: error(ui(R.string.server_is_closed))).serverIcon()
            }
        }

    internal suspend fun avatarImage(server: ServerState, id: String, revision: String): ByteArray =
        mediaCache.blob("${server.url}/${server.me}/avatar/$id/$revision", server = server.url) {
            withContext(dispatcher) {
                check(snapshots[server.url]?.me == server.me) { ui(R.string.account_changed) }
                (handles[server.url] ?: error(ui(R.string.server_is_closed))).avatarImage(id)
            }
        }

    internal suspend fun emojiImage(server: ServerState, id: String): ByteArray {
        val revision = server.emojis.firstOrNull { it.id == id }?.revision.orEmpty()
        return mediaCache.blob(
            "${server.url}/${server.me}/emoji/$id/$revision",
            server = server.url,
        ) {
            withContext(dispatcher) {
                check(snapshots[server.url]?.me == server.me) { ui(R.string.account_changed) }
                (handles[server.url] ?: error(ui(R.string.server_is_closed))).emojiImage(id)
            }
        }
    }

    private val accountMutex = Mutex()
    private val mutableRecoveryLink = MutableStateFlow<String?>(null)
    internal val recoveryLink = mutableRecoveryLink.asStateFlow()

    internal fun offerRecovery(link: String) {
        mutableRecoveryLink.value = link
    }

    internal fun consumeRecovery() {
        mutableRecoveryLink.value = null
    }

    internal suspend fun passwordResetLink(
        server: ServerState,
        target: String,
    ): io.github.chriexpe.papo.core.RecoveryReceipt = accountMutex.withLock {
        val client = accountClient(server)
        withContext(kotlinx.coroutines.Dispatchers.IO) {
            client.passwordResetLink(server.me, target)
        }
    }

    private suspend fun accountClient(server: ServerState): NativeClient =
        withContext(dispatcher) {
            check(snapshots[server.url]?.me == server.me) { ui(R.string.account_changed) }
            handles[server.url] ?: error(ui(R.string.server_is_closed))
        }

    internal suspend fun accountProfile(
        server: ServerState
    ): io.github.chriexpe.papo.core.AccountProfile = accountMutex.withLock {
        val client = accountClient(server)
        val profile =
            withContext(kotlinx.coroutines.Dispatchers.IO) { client.accountProfile(server.me) }
        withContext(dispatcher) { poll() }
        profile
    }

    internal suspend fun accountChange(
        server: ServerState,
        change: io.github.chriexpe.papo.core.AccountChange,
    ) = accountMutex.withLock {
        val client = accountClient(server)
        withContext(kotlinx.coroutines.Dispatchers.IO) { client.accountChange(server.me, change) }
        withContext(dispatcher) { poll() }
    }

    internal suspend fun accountSessions(
        server: ServerState
    ): List<io.github.chriexpe.papo.core.SessionView> = accountMutex.withLock {
        val client = accountClient(server)
        withContext(kotlinx.coroutines.Dispatchers.IO) { client.accountSessions(server.me) }
    }

    internal suspend fun administration(
        server: ServerState,
        refresh: Boolean = true,
    ): io.github.chriexpe.papo.core.AdministrationState = accountMutex.withLock {
        val client = accountClient(server)
        val result =
            withContext(kotlinx.coroutines.Dispatchers.IO) {
                client.administration(server.me, refresh)
            }
        withContext(dispatcher) { poll() }
        result
    }

    internal suspend fun administer(
        server: ServerState,
        change: io.github.chriexpe.papo.core.AdministrationChange,
    ): io.github.chriexpe.papo.core.AdministrationOutcome = accountMutex.withLock {
        val client = accountClient(server)
        try {
            withContext(kotlinx.coroutines.Dispatchers.IO) { client.administer(server.me, change) }
        } finally {
            withContext(dispatcher) { poll() }
        }
    }

    internal suspend fun channelRules(
        server: ServerState,
        id: String,
    ): List<io.github.chriexpe.papo.core.ChannelRule> = accountMutex.withLock {
        val client = accountClient(server)
        withContext(kotlinx.coroutines.Dispatchers.IO) { client.channelRules(server.me, id) }
    }

    internal suspend fun administrationAudit(
        server: ServerState,
        filter: io.github.chriexpe.papo.core.AuditFilter,
    ): io.github.chriexpe.papo.core.AuditPage = accountMutex.withLock {
        val client = accountClient(server)
        withContext(kotlinx.coroutines.Dispatchers.IO) {
            client.administrationAudit(server.me, filter)
        }
    }

    private val resultLoads = kotlinx.coroutines.sync.Semaphore(2)
    private val mutablePendingPins = MutableStateFlow<Set<String>>(emptySet())
    internal val pendingPins = mutablePendingPins.asStateFlow()

    internal fun pinKey(server: ServerState, channel: String, id: String) =
        "${server.url}/${server.me}/$channel/$id"

    internal fun watchPins(server: ServerState, channel: String, enabled: Boolean) =
        viewModelScope.launch(dispatcher) {
            runCatching { handles[server.url]?.watchPins(server.me, channel, enabled) }
            poll()
        }

    internal suspend fun loadPins(server: ServerState, channel: String) {
        val client = accountClient(server)
        try {
            withContext(kotlinx.coroutines.Dispatchers.IO) { client.loadPins(server.me, channel) }
        } finally {
            withContext(kotlinx.coroutines.NonCancellable + dispatcher) { poll() }
        }
    }

    internal suspend fun loadResultMessage(server: ServerState, channel: String, id: String) {
        resultLoads.acquire()
        try {
            val client = accountClient(server)
            try {
                withContext(kotlinx.coroutines.Dispatchers.IO) {
                    client.loadResultMessage(server.me, channel, id)
                }
            } finally {
                withContext(kotlinx.coroutines.NonCancellable + dispatcher) { poll() }
            }
        } finally {
            resultLoads.release()
        }
    }

    internal suspend fun setPinned(
        server: ServerState,
        channel: String,
        id: String,
        value: Boolean,
    ) {
        val key = pinKey(server, channel, id)
        val accepted =
            withContext(kotlinx.coroutines.Dispatchers.Main.immediate) {
                if (key in mutablePendingPins.value) false
                else {
                    mutablePendingPins.value += key
                    true
                }
            }
        if (!accepted) return
        try {
            val client = accountClient(server)
            withContext(kotlinx.coroutines.Dispatchers.IO) {
                client.setPin(server.me, channel, id, value)
            }
        } finally {
            withContext(kotlinx.coroutines.NonCancellable) {
                withContext(dispatcher) { poll() }
                withContext(kotlinx.coroutines.Dispatchers.Main.immediate) {
                    mutablePendingPins.value -= key
                }
            }
        }
    }

    private val gifLocks = java.util.concurrent.ConcurrentHashMap<String, Mutex>()
    private val pendingGifs = mutableMapOf<String, List<Pair<GifChoice, String>>>()

    internal fun react(server: ServerState, message: ChatMessage, emoji: EmojiChoice) {
        val add =
            !message.reactions.any {
                it.emoji == emoji.value && it.custom == emoji.custom && it.mine
            }
        command(
            Action.React(server.selected, message.id, emoji.value, emoji.custom, add),
            server.url,
        ) {
            if (add)
                viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) {
                    expressions.used(expressions.owner(server), listOf(emoji))
                }
        }
    }

    internal fun recordSent(
        server: ServerState,
        draftKey: String,
        text: String,
        files: Set<String>,
    ) =
        viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) {
            val owner = expressions.owner(server)
            expressions.used(owner, messageEmoji(text, server))
            val gifs =
                withContext(kotlinx.coroutines.Dispatchers.Main) {
                    pendingGifs.remove(draftKey).orEmpty()
                }
            gifs
                .filter { (g, path) ->
                    if (path.isNotBlank()) path in files
                    else
                        text.contains("giphy:${g.providerId}") ||
                            text.contains("https://giphy.com/gifs/${g.providerId}")
                }
                .forEach { (g, _) -> expressions.recent(owner, g) }
            GifChoice.marker(text)?.let { id ->
                if (gifs.none { it.first.providerId == id })
                    expressions.recent(owner, GifChoice("giphy:$id", "GIF", providerId = id))
            }
        }

    internal suspend fun gifPage(
        server: ServerState,
        query: String,
        offset: Int,
    ): io.github.chriexpe.papo.core.GifPageView {
        val client =
            withContext(dispatcher) {
                check(snapshots[server.url]?.me == server.me) { ui(R.string.account_changed) }
                handles[server.url] ?: error(ui(R.string.server_is_closed))
            }
        return withContext(kotlinx.coroutines.Dispatchers.IO) {
            client.gifPage(query, offset.toUInt())
        }
    }

    internal suspend fun gifItem(server: ServerState, id: String): GifChoice {
        val client =
            withContext(dispatcher) {
                check(snapshots[server.url]?.me == server.me) { ui(R.string.account_changed) }
                handles[server.url] ?: error(ui(R.string.server_is_closed))
            }
        return withContext(kotlinx.coroutines.Dispatchers.IO) { GifChoice.from(client.gifItem(id)) }
    }

    internal suspend fun gifCopy(server: ServerState, gif: GifChoice): java.io.File =
        withContext(kotlinx.coroutines.Dispatchers.IO) {
            val owner = expressions.owner(server)
            val target = expressions.file(owner, gif.key)
            if (target.isFile) return@withContext target
            val temporary = java.io.File.createTempFile("gif-", ".part", target.parentFile)
            try {
                when {
                    gif.embed.isNotBlank() ->
                        temporary.writeBytes(embedImage(server, gif.embed, gif.revision))
                    gif.attachment.isNotBlank() -> {
                        val source =
                            withContext(dispatcher) {
                                check(snapshots[server.url]?.me == server.me)
                                mediaCache.scoped(
                                    (handles[server.url] ?: error(ui(R.string.server_is_closed)))
                                        .mediaRequest(
                                            MediaResource.Attachment(gif.attachment, false)
                                        ),
                                    server.url,
                                )
                            }
                        mediaCache.export(source, temporary)
                    }
                    else -> {
                        val resolved =
                            if (gif.url.isBlank() && gif.providerId.isNotBlank())
                                gifItem(server, gif.providerId)
                            else gif
                        val source = publicMediaRequest(server, resolved.url, gif = true)
                        mediaCache.export(source, temporary)
                    }
                }
                check(temporary.length() > 0) { ui(R.string.gif_is_empty) }
                check(temporary.renameTo(target)) { ui(R.string.couldn_t_save_this_gif) }
                target
            } finally {
                temporary.delete()
            }
        }

    internal fun toggleGif(server: ServerState, gif: GifChoice) =
        viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) {
            val owner = expressions.owner(server)
            gifLocks
                .getOrPut("$owner/${gif.key}") { Mutex() }
                .withLock {
                    try {
                        val removing = expressions.state(owner).favorites.any { it.key == gif.key }
                        val saved =
                            if (
                                !removing && (gif.attachment.isNotBlank() || gif.embed.isNotBlank())
                            )
                                gif.copy(path = gifCopy(server, gif).absolutePath)
                            else gif
                        expressions.favorite(owner, saved)
                    } catch (e: kotlinx.coroutines.CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        withContext(dispatcher) {
                            publish(error = e.message ?: ui(R.string.couldn_t_save_this_gif))
                        }
                    }
                }
        }

    internal fun pickGif(
        server: ServerState,
        draftKey: String,
        gif: GifChoice,
        onMarker: (String) -> Unit,
        onDone: () -> Unit,
    ) = viewModelScope.launch {
        try {
            if (server.url in forgottenServers) return@launch
            if (gif.providerId.isNotBlank()) {
                pendingGifs[draftKey] = pendingGifs[draftKey].orEmpty() + (gif to "")
                onMarker("giphy:${gif.providerId}")
            } else {
                val file = gifCopy(server, gif)
                val copy =
                    withContext(kotlinx.coroutines.Dispatchers.IO) {
                        AttachmentFiles.create(getApplication(), "gif").also {
                            file.copyTo(it, true)
                        }
                    }
                val title = gif.title.ifBlank { "Animation" }.take(80)
                attachments
                    .captured(
                        draftKey,
                        copy,
                        if (title.endsWith(".gif", true)) title else "$title.gif",
                        "image/gif",
                    )
                    .join()
                if (server.url in forgottenServers) return@launch
                pendingGifs[draftKey] =
                    pendingGifs[draftKey].orEmpty() +
                        (gif.copy(path = file.absolutePath) to copy.absolutePath)
            }
            onDone()
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e
        } catch (e: Exception) {
            attachments.report(draftKey, e.message ?: ui(R.string.couldn_t_attach_this_gif))
        }
    }

    internal suspend fun audioWaveform(
        server: ServerState,
        resource: MediaResource,
    ): AudioWaveform {
        val source =
            withContext(dispatcher) {
                check(snapshots[server.url]?.me == server.me) { ui(R.string.account_changed) }
                mediaCache.scoped(
                    (handles[server.url] ?: error(ui(R.string.server_is_closed))).mediaRequest(
                        resource
                    ),
                    server.url,
                )
            }
        return waveforms.load(source)
    }

    internal suspend fun publicAudioWaveform(server: ServerState, url: String): AudioWaveform =
        waveforms.load(publicMediaRequest(server, url))

    suspend fun embedImage(
        server: ServerState,
        id: String,
        revision: String,
        author: Boolean = false,
    ): ByteArray =
        mediaCache.blob(
            "${server.url}/${server.me}/embed/$id/$revision/$author",
            server = server.url,
        ) {
            val client =
                withContext(dispatcher) {
                    check(snapshots[server.url]?.me == server.me) { ui(R.string.account_changed) }
                    handles[server.url] ?: error(ui(R.string.server_is_closed))
                }
            withContext(kotlinx.coroutines.Dispatchers.IO) { client.embedImage(id, author) }
        }

    fun send(
        server: ServerState,
        text: String,
        reply: String?,
        onResult: (Boolean) -> Unit,
    ) =
        viewModelScope.launch(dispatcher) {
            val id = UUID.randomUUID().toString()
            try {
                val handle = handles[server.url] ?: error(ui(R.string.server_is_closed))
                check(snapshots[server.url]?.me == server.me) {
                    ui(R.string.account_changed_before_sending)
                }
                awaitingPersistence[id] = PendingMessage(server.url, onResult)
                handle.dispatch(Action.Send(id, server.selected, text, reply))
                signals.trySend(Unit)
            } catch (e: Exception) {
                awaitingPersistence.remove(id)
                publish(error = e.message ?: ui(R.string.could_not_queue_message))
                launch(kotlinx.coroutines.Dispatchers.Main) { onResult(false) }
            }
        }

    private suspend fun uploadFiles(
        server: ServerState,
        text: String,
        reply: String?,
        files: List<PendingAttachment>,
    ): UploadReceipt {
        val result =
            withContext(dispatcher) {
                val id = UUID.randomUUID().toString()
                val completion = CompletableDeferred<UploadReceipt>()
                try {
                    val client = handles[server.url] ?: error(ui(R.string.server_is_closed))
                    check(snapshots[server.url]?.me == server.me) {
                        ui(R.string.account_changed_before_sending)
                    }
                    awaitingUploads[id] = PendingUpload(server.url, completion)
                    client.dispatch(
                        Action.SendAttachments(
                            id,
                            server.selected,
                            text,
                            reply,
                            files.map { it.native() },
                        )
                    )
                    signals.trySend(Unit)
                } catch (e: Exception) {
                    awaitingUploads.remove(id)
                    completion.complete(
                        UploadReceipt(
                            false,
                            error = e.message ?: ui(R.string.couldn_t_send_these_attachments),
                        )
                    )
                }
                completion
            }
        return result.await()
    }

    fun typing(server: ServerState) {
        val now = android.os.SystemClock.elapsedRealtime()
        if (now - lastTyping > 3000) {
            lastTyping = now
            command(Action.Typing(server.selected))
        }
    }

    private fun networkChanged(force: Boolean = false) =
        viewModelScope.launch(dispatcher) {
            val available =
                connectivity.activeNetwork
                    ?.let { connectivity.getNetworkCapabilities(it) }
                    ?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true
            val fingerprint = connectivity.activeNetwork?.networkHandle to available
            if (!force && fingerprint == networkFingerprint) return@launch
            networkFingerprint = fingerprint
            networkEpoch++
            handles.keys.toList().forEach { url ->
                command(Action.Network(available, networkEpoch.toULong()), url = url)
            }
        }

    fun resume() {
        networkChanged()
        viewModelScope.launch(dispatcher) {
            handles.keys.toList().forEach { command(Action.Refresh, url = it) }
        }
    }

    fun clearError() {
        mutableState.value = mutableState.value.copy(error = "")
        command(Action.ClearError)
    }

    internal fun readingPosition(key: String): Pair<String, Int>? = runCatching {
        val saved = org.json.JSONObject(preferences.getString("position/$key", null) ?: return null)
        saved.getString("id") to saved.getInt("offset")
    }
        .getOrNull()

    internal fun saveReadingPosition(key: String, id: String, offset: Int) {
        if (forgottenServers.any { key.startsWith("$it/") }) return
        preferences
            .edit()
            .putString(
                "position/$key",
                org.json.JSONObject().put("id", id).put("offset", offset).toString(),
            )
            .apply()
    }

    fun draft(key: String) = preferences.getString("draft/$key", "").orEmpty()

    fun draftMentions(key: String) = preferences.getString("draft-mentions/$key", "[]").orEmpty()

    fun saveDraft(key: String, text: String, mentions: String = "[]") {
        if (forgottenServers.any { key.startsWith("$it/") }) return
        preferences
            .edit()
            .putString("draft/$key", text)
            .putString("draft-mentions/$key", mentions)
            .apply()
    }

    internal suspend fun editableMessage(server: ServerState, source: String): ComposerDraft =
        withContext(dispatcher) {
            val client = accountClient(server)
            ComposerDraft.fromEditable(client.editableMessage(source))
        }

    internal suspend fun editMessage(server: ServerState, id: String, draft: ComposerDraft) =
        withContext(dispatcher) {
            val client = accountClient(server)
            val mentions =
                draft.mentions.filter { it.valid(draft.text) }.map { it.native(draft.text) }
            val wire = client.encodeMessage(draft.text, mentions)
            client.dispatch(Action.Edit(id, wire))
            poll()
        }

    internal suspend fun encodeMessage(
        server: ServerState,
        text: String,
        mentions: List<ComposerMention>,
    ): String =
        withContext(dispatcher) {
            check(snapshots[server.url]?.me == server.me) { ui(R.string.account_changed) }
            val valid = mentions.filter { it.valid(text) }.map { it.native(text) }
            (handles[server.url] ?: error(ui(R.string.server_is_closed))).encodeMessage(text, valid)
        }

    override fun onCleared() {
        if (callbackRegistered) connectivity.unregisterNetworkCallback(networkCallback)
        signals.close()
        executor.execute {
            handles.values.forEach { it.release() }
            dispatcher.close()
        }
    }
}

private fun NativeClient.release() {
    shutdown()
    close()
}
