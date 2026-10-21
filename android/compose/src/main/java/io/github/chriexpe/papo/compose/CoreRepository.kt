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

    private val emojiImages = object : LruCache<String, ByteArray>(16 * 1024 * 1024) {
        override fun sizeOf(key: String, value: ByteArray) = value.size
    }
    private val emojiImageLock = Mutex()
    internal suspend fun emojiImage(server: ServerState, id: String): ByteArray {
        val key="${server.url}/${server.me}/$id"
        emojiImages.get(key)?.let { return it }
        return emojiImageLock.withLock {
            emojiImages.get(key)?.let { return@withLock it }
            val client=withContext(dispatcher) { check(snapshots[server.url]?.me==server.me) { "Account changed" }; handles[server.url] ?: error("Server is closed") }
            withContext(kotlinx.coroutines.Dispatchers.IO) { client.emojiImage(id).also {emojiImages.put(key,it)} }
        }
    }

    internal val expressions = ExpressionLibrary(getApplication())
    private val gifLocks = java.util.concurrent.ConcurrentHashMap<String, Mutex>()
    private val pendingGifs = mutableMapOf<String, List<Pair<GifChoice,String>>>()

    internal fun react(server: ServerState, message: ChatMessage, emoji: EmojiChoice) {
        val add = !message.reactions.any { it.emoji == emoji.value && it.custom == emoji.custom && it.mine }
        command(Action.React(server.selected, message.id, emoji.value, emoji.custom, add), server.url) {
            if (add) viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) { expressions.used(expressions.owner(server), listOf(emoji)) }
        }
    }
    internal fun recordSent(server: ServerState, draftKey: String, text: String, files: Set<String>) = viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) {
        val owner=expressions.owner(server)
        expressions.used(owner, messageEmoji(text, server))
        val gifs=withContext(kotlinx.coroutines.Dispatchers.Main) { pendingGifs.remove(draftKey).orEmpty() }
        gifs.filter { (g,path) -> if(path.isNotBlank()) path in files else text.contains("giphy:${g.providerId}") || text.contains("https://giphy.com/gifs/${g.providerId}") }.forEach { (g,_) -> expressions.recent(owner,g) }
        GifChoice.marker(text)?.let { id -> if(gifs.none {it.first.providerId==id}) expressions.recent(owner,GifChoice("giphy:$id","GIF",providerId=id)) }
    }
    internal suspend fun gifPage(server: ServerState, query: String, offset: Int): io.github.chriexpe.papo.core.GifPageView {
        val client=withContext(dispatcher) { check(snapshots[server.url]?.me==server.me) { "Account changed" }; handles[server.url] ?: error("Server is closed") }
        return withContext(kotlinx.coroutines.Dispatchers.IO) { client.gifPage(query,offset.toUInt()) }
    }
    internal suspend fun gifItem(server: ServerState, id: String): GifChoice {
        val client=withContext(dispatcher) { check(snapshots[server.url]?.me==server.me) { "Account changed" }; handles[server.url] ?: error("Server is closed") }
        return withContext(kotlinx.coroutines.Dispatchers.IO) { GifChoice.from(client.gifItem(id)) }
    }
    internal suspend fun gifCopy(server: ServerState, gif: GifChoice): java.io.File = withContext(kotlinx.coroutines.Dispatchers.IO) {
        val owner=expressions.owner(server)
        val target=expressions.file(owner,gif.key)
        if (target.isFile) return@withContext target
        val temporary=java.io.File.createTempFile("gif-", ".part", target.parentFile)
        try {
            when {
                gif.embed.isNotBlank() -> temporary.writeBytes(embedImage(server,gif.embed,gif.revision))
                gif.attachment.isNotBlank() -> {
                    val source=withContext(dispatcher) { check(snapshots[server.url]?.me==server.me); (handles[server.url] ?: error("Server is closed")).mediaRequest(MediaResource.Attachment(gif.attachment,false)) }
                    MediaNetwork.download(getApplication(),source,"image.gif").copyTo(temporary,true)
                }
                else -> {
                    val resolved=if(gif.url.isBlank() && gif.providerId.isNotBlank()) gifItem(server,gif.providerId) else gif
                    val source=publicGifSource(resolved.url)
                    MediaNetwork.download(getApplication(),source,"image.gif").copyTo(temporary,true)
                }
            }
            check(temporary.length()>0) { "GIF is empty" }
            check(temporary.renameTo(target)) { "Couldn’t save this GIF" }
            target
        } finally { temporary.delete() }
    }
    internal fun toggleGif(server: ServerState, gif: GifChoice) = viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) {
        val owner=expressions.owner(server)
        gifLocks.getOrPut("$owner/${gif.key}") { Mutex() }.withLock {
            try {
                val removing=expressions.state(owner).favorites.any { it.key==gif.key }
                val saved=if(!removing && (gif.attachment.isNotBlank() || gif.embed.isNotBlank())) gif.copy(path=gifCopy(server,gif).absolutePath) else gif
                expressions.favorite(owner,saved)
            } catch(e:kotlinx.coroutines.CancellationException) { throw e }
            catch(e:Exception) { withContext(dispatcher) { publish(error=e.message ?: "Couldn’t save this GIF") } }
        }
    }
    internal fun pickGif(server: ServerState, draftKey: String, gif: GifChoice, onMarker: (String)->Unit, onDone: ()->Unit) = viewModelScope.launch {
        try {
            if(gif.providerId.isNotBlank()) {
                pendingGifs[draftKey]=pendingGifs[draftKey].orEmpty()+ (gif to "")
                onMarker("giphy:${gif.providerId}")
            } else {
                val file=gifCopy(server,gif)
                val copy=withContext(kotlinx.coroutines.Dispatchers.IO) { AttachmentFiles.create(getApplication(),"gif").also { file.copyTo(it,true) } }
                val title=gif.title.ifBlank { "Animation" }.take(80)
                attachments.captured(draftKey,copy,if(title.endsWith(".gif",true)) title else "$title.gif","image/gif").join()
                pendingGifs[draftKey]=pendingGifs[draftKey].orEmpty()+ (gif.copy(path=file.absolutePath) to copy.absolutePath)
            }
            onDone()
        } catch(e:kotlinx.coroutines.CancellationException) { throw e }
        catch(e:Exception) { attachments.report(draftKey,e.message ?: "Couldn’t attach this GIF") }
    }

    internal suspend fun audioWaveform(server: ServerState, resource: MediaResource): AudioWaveform {
        val source = withContext(dispatcher) {
            check(snapshots[server.url]?.me == server.me) { "Account changed" }
            (handles[server.url] ?: error("Server is closed")).mediaRequest(resource)
        }
        return waveforms.load(source)
    }
    internal suspend fun publicAudioWaveform(url:String):AudioWaveform = waveforms.load(publicMediaSource(url))

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
    fun draftMentions(key:String) = preferences.getString("draft-mentions/$key","[]").orEmpty()
    fun saveDraft(key: String, text: String, mentions:String="[]") { preferences.edit().putString("draft/$key", text).putString("draft-mentions/$key",mentions).apply() }
    internal suspend fun encodeMessage(server:ServerState,text:String,mentions:List<ComposerMention>):String = withContext(dispatcher) {
        check(snapshots[server.url]?.me==server.me) {"Account changed"}
        val valid=mentions.filter {it.valid(text)}.map {it.native(text)}
        (handles[server.url] ?: error("Server is closed")).encodeMessage(text,valid)
    }

    override fun onCleared() {
        if (callbackRegistered) connectivity.unregisterNetworkCallback(networkCallback)
        signals.close()
        executor.execute { handles.values.forEach { it.release() }; dispatcher.close() }
    }
}

private fun NativeClient.release() { shutdown(); close() }
