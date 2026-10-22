package io.github.chriexpe.papo.compose

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import io.github.chriexpe.papo.core.UploadFile
import java.io.File
import java.io.IOException
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

internal data class PendingAttachment(val path: String, val name: String, val mime: String, val size: Long) {
    fun native() = UploadFile(path, name, mime, size.toULong())
}

internal data class AttachmentDraft(
    val files: List<PendingAttachment> = emptyList(),
    val preparing: Boolean = false,
    val sending: Boolean = false,
    val uncertain: Boolean = false,
    val error: String = "",
)
internal data class UploadReceipt(val accepted: Boolean, val uncertain: Boolean = false, val error: String = "")

/** Private copies survive provider permission expiry and process death. */
internal object AttachmentFiles {
    fun directory(context: Context) = File(context.filesDir, "uploads").apply { mkdirs() }
    fun create(context: Context, extension: String): File = File(directory(context), "${UUID.randomUUID()}.$extension")
    fun owns(context: Context, file: File) = file.canonicalFile.parentFile == directory(context).canonicalFile
    fun copy(context: Context, uri: Uri): PendingAttachment {
        val resolver = context.contentResolver
        val name = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) cursor.getString(0) else null
        }?.takeIf { it.isNotBlank() } ?: "attachment"
        val mime = resolver.getType(uri) ?: "application/octet-stream"
        val extension = name.substringAfterLast('.', "bin").filter(Char::isLetterOrDigit).take(12).ifBlank { "bin" }
        val file = create(context, extension)
        try {
            (resolver.openInputStream(uri) ?: throw IOException("Couldn’t open $name")).use { input ->
                file.outputStream().use { input.copyTo(it, 64 * 1024) }
            }
            return PendingAttachment(file.absolutePath, name, mime, file.length())
        } catch (e: Exception) { file.delete(); throw e }
    }
}

internal class AttachmentDrafts(
    private val context: Context,
    private val scope: CoroutineScope,
    private val upload: suspend (ServerState, String, String?, List<PendingAttachment>) -> UploadReceipt,
) {
    private val preferences = context.getSharedPreferences("compose-attachments", Context.MODE_PRIVATE)
    private val mutableDrafts = MutableStateFlow<Map<String, AttachmentDraft>>(emptyMap())
    val drafts = mutableDrafts.asStateFlow()
    private val locks = mutableMapOf<String, Mutex>()
    private fun lock(key: String) = locks.getOrPut(key) { Mutex() }
    private var initialized = false
    private val restored = scope.async(Dispatchers.IO) {
        preferences.all.mapNotNull { (key, value) ->
            if (value !is String) return@mapNotNull null
            runCatching {
                val json = JSONObject(value)
                val array = json.getJSONArray("files")
                val files = (0 until array.length()).mapNotNull { i ->
                    val f = array.getJSONObject(i)
                    val file = File(f.getString("path"))
                    if (!AttachmentFiles.owns(context, file) || !file.isFile) null
                    else PendingAttachment(file.absolutePath, f.getString("name"), f.getString("mime"), file.length())
                }
                val uncertain = json.optBoolean("sending") || json.optBoolean("uncertain")
                key to AttachmentDraft(files, uncertain = uncertain, error = when {
                    uncertain -> "The previous upload was interrupted and may already have arrived. Check the conversation before retrying."
                    files.size != array.length() -> "Some attachments are no longer available. Choose them again."
                    else -> json.optString("error")
                })
            }.getOrNull()
        }.toMap()
    }
    init { scope.launch { ready() } }
    private suspend fun ready() {
        val saved = restored.await()
        if (!initialized) { initialized = true; mutableDrafts.value = saved }
    }
    private fun get(key: String) = mutableDrafts.value[key] ?: AttachmentDraft()
    private fun update(key: String, value: AttachmentDraft) { mutableDrafts.value = mutableDrafts.value + (key to value) }
    private suspend fun persist(key: String) {
        val draft = get(key)
        val json = JSONObject().put("files", JSONArray().apply { draft.files.forEach { file ->
            put(JSONObject().put("path", file.path).put("name", file.name).put("mime", file.mime).put("size", file.size))
        } }).put("sending", draft.sending).put("uncertain", draft.uncertain).put("error", draft.error)
        withContext(Dispatchers.IO) {
            check(preferences.edit().putString(key, json.toString()).commit()) { "Couldn’t save the attachment draft" }
        }
    }
    /** Persist the replacement before deleting the old private draft file. */
    suspend fun replaceEdited(key:String,original:PendingAttachment,image:io.github.chriexpe.papo.core.PreparedMediaImage) = lock(key).withLock {
        ready()
        val previous=get(key)
        check(!previous.sending && !previous.preparing && !previous.uncertain) {"Finish the current attachment action before editing"}
        check(original in previous.files && AttachmentFiles.owns(context,File(original.path))) {"This attachment is no longer in the draft"}
        val file=AttachmentFiles.create(context,image.extension)
        try {
            val replacement=withContext(Dispatchers.IO) {
                file.writeBytes(android.util.Base64.decode(image.blob,android.util.Base64.DEFAULT))
                PendingAttachment(file.absolutePath,original.name.substringBeforeLast('.')+"."+image.extension,image.mime,file.length())
            }
            withContext(kotlinx.coroutines.NonCancellable) {
                update(key,previous.copy(files=previous.files.map {if(it==original) replacement else it},error=""))
                try {persist(key)} catch(e:Exception) {update(key,previous);throw e}
                withContext(Dispatchers.IO) {File(original.path).delete()}
            }
        } catch(e:Throwable) {if(get(key).files.none {it.path==file.absolutePath}) file.delete();throw e}
    }
    fun report(key: String, message: String) = scope.launch { ready(); update(key, get(key).copy(error = message)) }
    fun stage(key: String, uris: List<Uri>) = scope.launch {
        lock(key).withLock {
            ready()
            if (uris.isEmpty()) return@launch
            if (get(key).sending || get(key).preparing) { report(key, "Wait for the current attachment action to finish."); return@launch }
            if (get(key).files.size + uris.size > 10) { report(key, "You can attach up to ten files to one message."); return@launch }
            update(key, get(key).copy(preparing = true, error = ""))
            val copied = mutableListOf<PendingAttachment>()
            try {
                withContext(Dispatchers.IO) { uris.forEach { copied += AttachmentFiles.copy(context, it) } }
                update(key, get(key).copy(files = get(key).files + copied, preparing = false))
                persist(key)
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                val retained = get(key).files.map { it.path }.toSet()
                withContext(Dispatchers.IO) { copied.filter { it.path !in retained }.forEach { File(it.path).delete() } }
                update(key, get(key).copy(preparing = false, error = e.message ?: "Couldn’t prepare these files"))
            }
        }
    }
    fun finishCapture(key: String, keep: Boolean, name: String, stop: (Boolean) -> File?, onComplete: (Boolean) -> Unit) = scope.launch {
        val file = withContext(kotlinx.coroutines.NonCancellable + Dispatchers.IO) { stop(keep) }
        val staged = if (keep && file != null) addCaptured(key, file, name, "audio/mp4") else false
        if (keep && file == null) report(key, "The recording was too short or couldn’t be saved. Try again.")
        onComplete(staged)
    }
    fun captured(key: String, file: File, name: String, mime: String) = scope.launch { addCaptured(key, file, name, mime) }
    private suspend fun addCaptured(key: String, file: File, name: String, mime: String): Boolean = lock(key).withLock {
        ready()
        if (!AttachmentFiles.owns(context, file) || !file.isFile || file.length() == 0L) {
            report(key, "The capture was empty. Try again."); return@withLock false
        }
        if (get(key).files.size >= 10) { file.delete(); report(key, "You can attach up to ten files to one message."); return@withLock false }
        update(key, get(key).copy(files = get(key).files + PendingAttachment(file.absolutePath, name, mime, file.length()), error = ""))
        try { persist(key); true }
        catch (e: Exception) { report(key, e.message ?: "Couldn’t save this attachment"); false }
    }
    fun remove(key: String, file: PendingAttachment) = scope.launch {
        lock(key).withLock {
            ready()
            if (get(key).sending || get(key).preparing) return@launch
            val remaining = get(key).files - file
            update(key, get(key).copy(files = remaining, uncertain = get(key).uncertain && remaining.isNotEmpty(), error = ""))
            try {
                persist(key)
                withContext(Dispatchers.IO) { File(file.path).takeIf { AttachmentFiles.owns(context, it) }?.delete() }
            } catch (e: Exception) { report(key, e.message ?: "Couldn’t remove this attachment") }
        }
    }
    fun send(key: String, server: ServerState, text: String, reply: String?, onResult: (Boolean) -> Unit) = scope.launch {
        lock(key).withLock {
            ready()
            val draft = get(key)
            if (draft.sending || draft.preparing || draft.files.isEmpty()) { onResult(false); return@launch }
            update(key, draft.copy(sending = true, error = ""))
            var dispatched = false
            try {
                // Persist the in-flight marker before dispatch. A restarted process
                // cannot silently resend a POST whose outcome it never received.
                persist(key)
                dispatched = true
                val result = upload(server, text, reply, draft.files)
                if (result.accepted) {
                    update(key, get(key).copy(files = get(key).files - draft.files.toSet(), sending = false, uncertain = false, error = ""))
                    persist(key)
                    withContext(Dispatchers.IO) { draft.files.forEach { File(it.path).takeIf { file -> AttachmentFiles.owns(context, file) }?.delete() } }
                    onResult(true)
                } else {
                    update(key, get(key).copy(sending = false, uncertain = result.uncertain, error = result.error))
                    persist(key)
                    onResult(false)
                }
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) {
                update(key, get(key).copy(files = (get(key).files + draft.files).distinctBy { it.path }, sending = false, uncertain = dispatched || draft.uncertain, error = e.message ?: "The upload couldn’t be confirmed. Check the conversation before retrying."))
                runCatching { persist(key) }
                onResult(false)
            }
        }
    }
}
