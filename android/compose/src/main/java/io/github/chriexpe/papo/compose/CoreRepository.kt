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
import java.security.KeyStore
import java.util.UUID
import java.util.concurrent.Executors
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.json.JSONArray
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.NativeClient
import io.github.chriexpe.papo.core.Credentials
import io.github.chriexpe.papo.core.WakeListener
import io.github.chriexpe.papo.core.MediaResource
import io.github.chriexpe.papo.core.MediaRequest
import kotlinx.coroutines.withContext
import kotlinx.coroutines.async
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import android.util.LruCache

typealias ServerState = io.github.chriexpe.papo.core.ClientSnapshot
typealias Conversation = io.github.chriexpe.papo.core.ChannelView
typealias Person = io.github.chriexpe.papo.core.MemberView
typealias Reaction = io.github.chriexpe.papo.core.ReactionView
typealias ChatMessage = io.github.chriexpe.papo.core.MessageView
val ServerState.name get() = serverName
val ServerState.selected get() = selectedChannel
val ServerState.conversation get() = (channels + dms).firstOrNull { it.id == selected }
val ServerState.isDemo get() = url == "demo://papo"

class CoreWake(private val wake: () -> Unit) : WakeListener {
    override fun onWake() = wake()
}

/** Persists core credentials encrypted with an Android Keystore key. */
@Keep
class AndroidSecrets(context: Context) : Credentials {
    private val preferences = context.getSharedPreferences("core-secrets", Context.MODE_PRIVATE)
    private val key: SecretKey by lazy {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey("papo-core-v1", null) as? SecretKey) ?: KeyGenerator.getInstance(
            KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore"
        ).apply {
            init(KeyGenParameterSpec.Builder("papo-core-v1", KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        }.generateKey()
    }

    @Synchronized override fun load(server: String, secret: String): String? {
        val id = "$server/$secret"
        val bytes = preferences.getString(id, null)?.let { Base64.decode(it, Base64.NO_WRAP) } ?: return null
        require(bytes.size >= 28) { "Invalid encrypted credential" }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, bytes.copyOfRange(0, 12)))
        cipher.updateAAD(id.toByteArray(Charsets.UTF_8))
        return String(cipher.doFinal(bytes.copyOfRange(12, bytes.size)), Charsets.UTF_8)
    }

    @Synchronized override fun store(server: String, secret: String, value: String) {
        val id = "$server/$secret"
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key)
        cipher.updateAAD(id.toByteArray(Charsets.UTF_8))
        val bytes = cipher.iv + cipher.doFinal(value.toByteArray(Charsets.UTF_8))
        check(preferences.edit().putString(id, Base64.encodeToString(bytes, Base64.NO_WRAP)).commit()) {
            "Could not persist credential"
        }
    }

    @Synchronized override fun remove(server: String, secret: String) {
        check(preferences.edit().remove("$server/$secret").commit()) { "Could not remove credential" }
    }
}

data class PapoState(val servers: List<ServerState> = emptyList(), val activeUrl: String = "", val error: String = "", val opening: Boolean = false) {
    val active get() = servers.firstOrNull { it.url == activeUrl }
}

class PapoViewModel(application: Application) : AndroidViewModel(application) {
    private val executor = Executors.newSingleThreadExecutor { r -> Thread(r, "papo-core-actor") }
    private val dispatcher = executor.asCoroutineDispatcher()
    private val signals = Channel<Unit>(Channel.CONFLATED)
    private val preferences = application.getSharedPreferences("compose-client", Context.MODE_PRIVATE)
    private val secrets = AndroidSecrets(application)
    private val handles = linkedMapOf<String, NativeClient>()
    private data class EmbedKey(val server: String, val account: String, val id: String, val revision: String, val author: Boolean)
    private val waveforms = AudioWaveforms(application, viewModelScope)
    private val embedLock = Mutex()
    private val embedLoads = mutableMapOf<EmbedKey, Deferred<ByteArray>>()
    private val embedCache = object : LruCache<EmbedKey, ByteArray>(16 * 1024 * 1024) {
        override fun sizeOf(key: EmbedKey, value: ByteArray) = value.size
    }
    private val snapshots = linkedMapOf<String, ServerState>()
    private val mutableState = MutableStateFlow(PapoState())
    val state = mutableState.asStateFlow()
    private var activeUrl = preferences.getString("active-server", "").orEmpty()
    private val connectivity = application.getSystemService(ConnectivityManager::class.java)
    private var networkEpoch = 0L
    private var lastTyping = 0L
    private var callbackRegistered = false
    private val awaitingUploads = mutableMapOf<String, CompletableDeferred<UploadReceipt>>()
    internal val attachments = AttachmentDrafts(application, viewModelScope, ::uploadFiles)
    private val awaitingPersistence = mutableMapOf<String, (Boolean) -> Unit>()
    private var networkFingerprint: Pair<Long?, Boolean>? = null
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) { networkChanged() }
        override fun onLost(network: Network) { networkChanged() }
        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) { networkChanged() }
    }

    init {
        viewModelScope.launch(dispatcher) {
            val saved = runCatching { JSONArray(preferences.getString("servers", "[]")) }.getOrDefault(JSONArray())
            for (i in 0 until saved.length()) open(saved.getString(i), activate = false)
            if (activeUrl !in handles) activeUrl = handles.keys.firstOrNull().orEmpty()
            publish()
            signals.trySend(Unit)
            for (signal in signals) poll()
        }
        runCatching { connectivity.registerDefaultNetworkCallback(networkCallback); callbackRegistered = true }
    }

    private fun publish(error: String = mutableState.value.error, opening: Boolean = false) {
        mutableState.value = PapoState(snapshots.values.toList(), activeUrl, error, opening)
    }

    private fun open(input: String, activate: Boolean = true): Boolean = try {
        publish(error = "", opening = true)
        val path = getApplication<Application>().filesDir.resolve("compose-messages.db").absolutePath
        val handle = NativeClient.open(input, path, secrets, CoreWake { signals.trySend(Unit) })
        val server = handle.poll()
        val old = handles[server.url]
        if (old != null) { handle.release() } else {
            handles[server.url] = handle
            snapshots[server.url] = server
        }
        if (activate) activeUrl = server.url
        preferences.edit().putString("servers", JSONArray(handles.keys.filter { it != "demo://papo" }).toString()).putString("active-server", activeUrl).apply()
        publish(error = "")
        networkChanged(force = true)
        true
    } catch (e: Exception) { publish(error = e.message ?: "Could not open server"); false }

    private fun poll() {
        try {
            for ((url, handle) in handles) {
                val snapshot = handle.poll()
                snapshots[url] = snapshot
                for (event in snapshot.events) {
                    awaitingUploads.remove(event.localId)?.complete(UploadReceipt(
                        event.kind == "accepted", event.kind == "unknown", event.error
                    ))
                    val callback = awaitingPersistence.remove(event.localId) ?: continue
                    val accepted = event.kind == "accepted"
                    if (!accepted) publish(error = event.error)
                    viewModelScope.launch(kotlinx.coroutines.Dispatchers.Main) { callback(accepted) }
                }
                if (snapshot.more) signals.trySend(Unit)
            }
            publish()
        } catch (e: Exception) { publish(error = e.message ?: "Could not update messages") }
    }

    fun addServer(address: String, onSuccess: () -> Unit = {}) = viewModelScope.launch(dispatcher) {
        if (open(address)) launch(kotlinx.coroutines.Dispatchers.Main) { onSuccess() }
    }

    fun exploreDemo() = viewModelScope.launch(dispatcher) {
        try {
            if ("demo://papo" !in handles) handles["demo://papo"] = NativeClient.demo()
            activeUrl = "demo://papo"
            poll()
        } catch (e: Exception) { publish(error = e.message ?: "Could not open demo") }
    }

    fun leaveDemo() = viewModelScope.launch(dispatcher) {
        handles.remove("demo://papo")?.release()
        snapshots.remove("demo://papo")
        activeUrl = handles.keys.firstOrNull().orEmpty()
        publish(error = "")
    }

    fun activate(url: String) = viewModelScope.launch(dispatcher) {
        activeUrl = url
        preferences.edit().putString("active-server", url).apply()
        publish(error = "")
    }

    fun command(action: Action, url: String = state.value.activeUrl, onSuccess: () -> Unit = {}) = viewModelScope.launch(dispatcher) {
        try {
            val handle = handles[url] ?: return@launch
            handle.dispatch(action)
            poll()
            launch(kotlinx.coroutines.Dispatchers.Main) { onSuccess() }
        } catch (e: Exception) { publish(error = e.message ?: "Could not complete action") }
    }

    suspend fun mediaRequest(url: String, resource: MediaResource): MediaRequest = withContext(dispatcher) {
        (handles[url] ?: error("Server is closed")).mediaRequest(resource)
    }

    internal suspend fun audioWaveform(server: ServerState, resource: MediaResource): AudioWaveform {
        val source = withContext(dispatcher) {
            check(snapshots[server.url]?.me == server.me) { "Account changed" }
            (handles[server.url] ?: error("Server is closed")).mediaRequest(resource)
        }
        return waveforms.load(source)
    }

    suspend fun embedImage(server: ServerState, id: String, revision: String, author: Boolean = false): ByteArray {
        val key = EmbedKey(server.url, server.me, id, revision, author)
        val load = embedLock.withLock {
            embedCache.get(key)?.let { return it }
            embedLoads.getOrPut(key) {
                viewModelScope.async(kotlinx.coroutines.Dispatchers.IO) {
                    try {
                        val client = withContext(dispatcher) {
                            check(snapshots[server.url]?.me == server.me) { "Account changed" }
                            handles[server.url] ?: error("Server is closed")
                        }
                        val bytes = client.embedImage(id, author)
                        embedLock.withLock { embedCache.put(key, bytes) }
                        bytes
                    } finally {
                        embedLock.withLock { embedLoads.remove(key) }
                    }
                }
            }
        }
        return load.await()
    }

    fun send(server: ServerState, text: String, reply: String?, onResult: (Boolean) -> Unit) = viewModelScope.launch(dispatcher) {
        val id = UUID.randomUUID().toString()
        try {
            val handle = handles[server.url] ?: error("Server is closed")
            awaitingPersistence[id] = onResult
            handle.dispatch(Action.Send(id, server.selected, text, reply))
            signals.trySend(Unit)
        } catch (e: Exception) {
            awaitingPersistence.remove(id)
            publish(error = e.message ?: "Could not queue message")
            launch(kotlinx.coroutines.Dispatchers.Main) { onResult(false) }
        }
    }

    private suspend fun uploadFiles(server: ServerState, text: String, reply: String?, files: List<PendingAttachment>): UploadReceipt {
        val result = withContext(dispatcher) {
            val id = UUID.randomUUID().toString()
            val completion = CompletableDeferred<UploadReceipt>()
            try {
                val client = handles[server.url] ?: error("Server is closed")
                check(snapshots[server.url]?.me == server.me) { "Account changed before sending" }
                awaitingUploads[id] = completion
                client.dispatch(Action.SendAttachments(id, server.selected, text, reply, files.map { it.native() }))
                signals.trySend(Unit)
            } catch (e: Exception) {
                awaitingUploads.remove(id)
                completion.complete(UploadReceipt(false, error = e.message ?: "Couldn’t send these attachments"))
            }
            completion
        }
        return result.await()
    }

    fun typing(server: ServerState) {
        val now = android.os.SystemClock.elapsedRealtime()
        if (now - lastTyping > 3000) { lastTyping = now; command(Action.Typing(server.selected)) }
    }

    private fun networkChanged(force: Boolean = false) = viewModelScope.launch(dispatcher) {
        val available = connectivity.activeNetwork?.let { connectivity.getNetworkCapabilities(it) }
            ?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true
        val fingerprint = connectivity.activeNetwork?.networkHandle to available
        if (!force && fingerprint == networkFingerprint) return@launch
        networkFingerprint = fingerprint
        networkEpoch++
        handles.keys.toList().forEach { url -> command(Action.Network(available, networkEpoch.toULong()), url = url) }
    }

    fun resume() { networkChanged(); viewModelScope.launch(dispatcher) { handles.keys.toList().forEach { command(Action.Refresh, url = it) } } }
    fun clearError() { mutableState.value = mutableState.value.copy(error = ""); command(Action.ClearError) }
    fun draft(key: String) = preferences.getString("draft/$key", "").orEmpty()
    fun saveDraft(key: String, text: String) { preferences.edit().putString("draft/$key", text).apply() }

    override fun onCleared() {
        if (callbackRegistered) connectivity.unregisterNetworkCallback(networkCallback)
        signals.close()
        executor.execute { handles.values.forEach { it.release() }; dispatcher.close() }
    }
}

private fun NativeClient.release() { shutdown(); close() }
