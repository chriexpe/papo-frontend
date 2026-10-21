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
import org.json.JSONObject

@Keep
object NativeCore {
    init { System.loadLibrary("papo_compose") }
    external fun open(url: String, database: String, secrets: AndroidSecrets, listener: CoreWake): Long
    external fun poll(handle: Long): String
    external fun dispatch(handle: Long, action: String)
    external fun close(handle: Long)
    external fun openDemo(): Long
}

@Keep
class CoreWake(private val wake: () -> Unit) {
    fun onWake() = wake()
}

/** Tokens never leave native core except through this Keystore-backed store. */
@Keep
class AndroidSecrets(context: Context) {
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

    @Synchronized fun load(server: String, secret: String): String? {
        val id = "$server/$secret"
        val bytes = preferences.getString(id, null)?.let { Base64.decode(it, Base64.NO_WRAP) } ?: return null
        require(bytes.size >= 28) { "Invalid encrypted credential" }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, bytes.copyOfRange(0, 12)))
        cipher.updateAAD(id.toByteArray(Charsets.UTF_8))
        return String(cipher.doFinal(bytes.copyOfRange(12, bytes.size)), Charsets.UTF_8)
    }

    @Synchronized fun store(server: String, secret: String, value: String) {
        val id = "$server/$secret"
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key)
        cipher.updateAAD(id.toByteArray(Charsets.UTF_8))
        val bytes = cipher.iv + cipher.doFinal(value.toByteArray(Charsets.UTF_8))
        check(preferences.edit().putString(id, Base64.encodeToString(bytes, Base64.NO_WRAP)).commit()) {
            "Could not persist credential"
        }
    }

    @Synchronized fun remove(server: String, secret: String) {
        check(preferences.edit().remove("$server/$secret").commit()) { "Could not remove credential" }
    }
}

internal fun JSONObject.string(key: String, fallback: String = "") = if (isNull(key)) fallback else optString(key, fallback)
internal fun JSONArray.objects(): List<JSONObject> = (0 until length()).mapNotNull { optJSONObject(it) }
internal fun JSONObject.objects(key: String) = optJSONArray(key)?.objects().orEmpty()

data class Conversation(val id: String, val name: String, val kind: String, val topic: String, val unread: Boolean, val mentions: Int)
data class Person(val id: String, val name: String, val presence: String)
data class Reaction(val emoji: String, val count: Int, val mine: Boolean, val custom: Boolean)
data class ChatMessage(
    val id: String, val authorId: String, val author: String, val content: String, val at: Long,
    val mine: Boolean, val edited: Boolean, val pending: Boolean, val delivery: String,
    val replyTo: String, val replyContent: String, val reactions: List<Reaction>, val attachments: List<String>,
)
data class ServerState(
    val url: String, val name: String, val screen: String, val connection: String, val me: String,
    val myName: String, val selected: String, val channels: List<Conversation>, val members: List<Person>,
    val dms: List<Conversation>, val messages: List<ChatMessage>, val locked: Boolean, val busy: Boolean,
    val error: String, val typing: List<String>, val canLoadOlder: Boolean, val loadingOlder: Boolean,
) {
    val conversation get() = (channels + dms).firstOrNull { it.id == selected }
    val isDemo get() = url == "demo://papo"
    companion object {
        fun parse(json: JSONObject): ServerState = ServerState(
            url = json.string("url"), name = json.string("serverName"), screen = json.string("screen"),
            connection = json.string("connection"), me = json.string("me"), myName = json.string("myName"),
            selected = json.string("selectedChannel"), channels = json.objects("channels").map {
                Conversation(it.string("id"), it.string("name"), it.string("kind"), it.string("topic"), it.optBoolean("unread"), it.optInt("mentions"))
            }, members = json.objects("members").map { Person(it.string("id"), it.string("name"), it.string("presence")) },
            dms = json.objects("dms").map { Conversation(it.string("id"), it.string("name"), "direct", "", it.optInt("unread") > 0, it.optInt("unread")) },
            messages = json.objects("messages").map { m -> ChatMessage(
                m.string("id"), m.string("authorId"), m.string("author"), m.string("content"), m.optLong("at"),
                m.optBoolean("mine"), m.optBoolean("edited"), m.optBoolean("pending"), m.string("delivery"),
                m.string("replyTo"), m.string("replyContent"), m.objects("reactions").map {
                    Reaction(it.string("emoji"), it.optInt("count"), it.optBoolean("mine"), it.optBoolean("custom"))
                }, m.objects("attachments").map { it.string("name") }
            ) }, locked = json.optBoolean("locked"), busy = json.optBoolean("busy"), error = json.string("error"),
            typing = json.optJSONArray("typing")?.let { a -> (0 until a.length()).map { a.getString(it) } }.orEmpty(),
            canLoadOlder = json.optBoolean("canLoadOlder"), loadingOlder = json.optBoolean("loadingOlder"),
        )
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
    private val handles = linkedMapOf<String, Long>()
    private val snapshots = linkedMapOf<String, ServerState>()
    private val mutableState = MutableStateFlow(PapoState())
    val state = mutableState.asStateFlow()
    private var activeUrl = preferences.getString("active-server", "").orEmpty()
    private val connectivity = application.getSystemService(ConnectivityManager::class.java)
    private var networkEpoch = 0L
    private var lastTyping = 0L
    private var callbackRegistered = false
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
        val handle = NativeCore.open(input, path, secrets, CoreWake { signals.trySend(Unit) })
        val snapshot = JSONObject(NativeCore.poll(handle))
        val server = ServerState.parse(snapshot)
        val old = handles[server.url]
        if (old != null) { NativeCore.close(handle) } else {
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
                val json = JSONObject(NativeCore.poll(handle))
                snapshots[url] = ServerState.parse(json)
                for (event in json.objects("events")) {
                    val callback = awaitingPersistence.remove(event.string("localId")) ?: continue
                    val accepted = event.string("type") == "accepted"
                    if (!accepted) publish(error = event.string("error"))
                    viewModelScope.launch(kotlinx.coroutines.Dispatchers.Main) { callback(accepted) }
                }
                if (json.optBoolean("more")) signals.trySend(Unit)
            }
            publish()
        } catch (e: Exception) { publish(error = e.message ?: "Could not update messages") }
    }

    fun addServer(address: String, onSuccess: () -> Unit = {}) = viewModelScope.launch(dispatcher) {
        if (open(address)) launch(kotlinx.coroutines.Dispatchers.Main) { onSuccess() }
    }

    fun exploreDemo() = viewModelScope.launch(dispatcher) {
        try {
            if ("demo://papo" !in handles) handles["demo://papo"] = NativeCore.openDemo()
            activeUrl = "demo://papo"
            poll()
        } catch (e: Exception) { publish(error = e.message ?: "Could not open demo") }
    }

    fun leaveDemo() = viewModelScope.launch(dispatcher) {
        handles.remove("demo://papo")?.let(NativeCore::close)
        snapshots.remove("demo://papo")
        activeUrl = handles.keys.firstOrNull().orEmpty()
        publish(error = "")
    }

    fun activate(url: String) = viewModelScope.launch(dispatcher) {
        activeUrl = url
        preferences.edit().putString("active-server", url).apply()
        publish(error = "")
    }

    fun command(type: String, vararg arguments: Pair<String, Any?>, url: String = state.value.activeUrl, onSuccess: () -> Unit = {}) = viewModelScope.launch(dispatcher) {
        try {
            val handle = handles[url] ?: return@launch
            val json = JSONObject().put("type", type)
            arguments.forEach { (key, value) -> json.put(key, value ?: JSONObject.NULL) }
            NativeCore.dispatch(handle, json.toString())
            poll()
            launch(kotlinx.coroutines.Dispatchers.Main) { onSuccess() }
        } catch (e: Exception) { publish(error = e.message ?: "Could not complete action") }
    }

    fun send(server: ServerState, text: String, reply: String?, onResult: (Boolean) -> Unit) = viewModelScope.launch(dispatcher) {
        val id = UUID.randomUUID().toString()
        try {
            val handle = handles[server.url] ?: error("Server is closed")
            awaitingPersistence[id] = onResult
            NativeCore.dispatch(handle, JSONObject().put("type", "send").put("local_id", id)
                .put("channel_id", server.selected).put("content", text).put("reply_to", reply ?: JSONObject.NULL).toString())
            signals.trySend(Unit)
        } catch (e: Exception) {
            awaitingPersistence.remove(id)
            publish(error = e.message ?: "Could not queue message")
            launch(kotlinx.coroutines.Dispatchers.Main) { onResult(false) }
        }
    }

    fun typing(server: ServerState) {
        val now = android.os.SystemClock.elapsedRealtime()
        if (now - lastTyping > 3000) { lastTyping = now; command("typing", "channel_id" to server.selected) }
    }

    private fun networkChanged(force: Boolean = false) = viewModelScope.launch(dispatcher) {
        val available = connectivity.activeNetwork?.let { connectivity.getNetworkCapabilities(it) }
            ?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true
        val fingerprint = connectivity.activeNetwork?.networkHandle to available
        if (!force && fingerprint == networkFingerprint) return@launch
        networkFingerprint = fingerprint
        networkEpoch++
        handles.keys.toList().forEach { url -> command("network", "available" to available, "epoch" to networkEpoch, url = url) }
    }

    fun resume() { networkChanged(); viewModelScope.launch(dispatcher) { handles.keys.toList().forEach { command("refresh", url = it) } } }
    fun clearError() { mutableState.value = mutableState.value.copy(error = ""); command("clear_error") }
    fun draft(key: String) = preferences.getString("draft/$key", "").orEmpty()
    fun saveDraft(key: String, text: String) { preferences.edit().putString("draft/$key", text).apply() }

    override fun onCleared() {
        if (callbackRegistered) connectivity.unregisterNetworkCallback(networkCallback)
        signals.close()
        executor.execute { handles.values.forEach(NativeCore::close); dispatcher.close() }
    }
}
