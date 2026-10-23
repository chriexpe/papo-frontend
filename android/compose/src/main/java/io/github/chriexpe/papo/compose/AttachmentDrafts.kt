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

/** A derived privacy copy never replaces its source: disabling the option is reversible. */
internal data class SanitizedAttachment(val path: String, val mime: String, val size: Long)

internal fun attachmentExtension(name: String): String =
    name.substringAfterLast('.', "").takeIf { name.lastIndexOf('.') > 0 }.orEmpty()

internal fun attachmentStem(name: String): String =
    if (attachmentExtension(name).isEmpty()) name else name.substringBeforeLast('.')

internal fun validAttachmentName(name: String): Boolean =
    name.isNotBlank() &&
        name != "." &&
        name != ".." &&
        name.toByteArray(Charsets.UTF_8).size <= 255 &&
        name.none { it.isISOControl() || it == '/' || it == '\\' }

internal data class PendingAttachment(
    val path: String,
    val name: String,
    val mime: String,
    val size: Long,
    val spoiler: Boolean = false,
    val sourceExtension: String = attachmentExtension(name),
    val sanitized: SanitizedAttachment? = null,
) {
    val wireName
        get() = if (spoiler && !name.startsWith("SPOILER_")) "SPOILER_$name" else name

    val uploadMime
        get() = sanitized?.mime ?: mime

    val uploadSize
        get() = sanitized?.size ?: size

    fun paths() = listOfNotNull(path, sanitized?.path)

    fun native() =
        UploadFile(sanitized?.path ?: path, name, uploadMime, uploadSize.toULong(), spoiler)
}

internal data class AttachmentDraft(
    val files: List<PendingAttachment> = emptyList(),
    val preparing: Boolean = false,
    val sending: Boolean = false,
    val uncertain: Boolean = false,
    val error: String = "",
)

internal data class UploadReceipt(
    val accepted: Boolean,
    val uncertain: Boolean = false,
    val error: String = "",
)

/** Private copies survive provider permission expiry and process death. */
internal object AttachmentFiles {
    fun directory(context: Context) = File(context.filesDir, "uploads").apply { mkdirs() }

    fun create(context: Context, extension: String): File =
        File(directory(context), "${UUID.randomUUID()}.$extension")

    fun owns(context: Context, file: File) =
        file.canonicalFile.parentFile == directory(context).canonicalFile

    fun copy(context: Context, uri: Uri): PendingAttachment {
        val resolver = context.contentResolver
        val name =
            resolver
                .query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
                ?.use { cursor ->
                    if (cursor.moveToFirst()) cursor.getString(0) else null
                }
                ?.takeIf { it.isNotBlank() } ?: "attachment"
        val mime = resolver.getType(uri) ?: "application/octet-stream"
        val extension =
            name.substringAfterLast('.', "bin").filter(Char::isLetterOrDigit).take(12).ifBlank {
                "bin"
            }
        val file = create(context, extension)
        try {
            (resolver.openInputStream(uri)
                    ?: throw IOException(ui(R.string.attachment_open_failed, name)))
                .use { input ->
                    file.outputStream().use { input.copyTo(it, 64 * 1024) }
                }
            return PendingAttachment(
                file.absolutePath,
                name.removePrefix("SPOILER_"),
                mime,
                file.length(),
                spoiler = name.startsWith("SPOILER_"),
            )
        } catch (e: Exception) {
            file.delete()
            throw e
        }
    }
}

internal class AttachmentDrafts(
    private val context: Context,
    private val scope: CoroutineScope,
    private val upload:
        suspend (ServerState, String, String?, Boolean, List<PendingAttachment>) -> UploadReceipt,
) {
    private val preferences =
        context.getSharedPreferences("compose-attachments", Context.MODE_PRIVATE)
    private val mutableDrafts = MutableStateFlow<Map<String, AttachmentDraft>>(emptyMap())
    val drafts = mutableDrafts.asStateFlow()
    private val locks = mutableMapOf<String, Mutex>()

    private val forgotten = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()
    private val sendJobs = mutableMapOf<String, kotlinx.coroutines.Job>()

    private fun blocked(key: String) = forgotten.any { key.startsWith("$it/") }

    fun allowServer(server: String) {
        forgotten.remove(server)
    }

    suspend fun forgetServer(server: String) {
        withContext(Dispatchers.Main.immediate) {
            forgotten += server
            sendJobs
                .filterKeys { it.startsWith("$server/") }
                .values
                .toList()
                .forEach { it.cancel() }
            sendJobs.filterKeys { it.startsWith("$server/") }.values.toList().forEach { it.join() }
            ready()
            mutableDrafts.value.keys
                .filter { it.startsWith("$server/") }
                .forEach { key ->
                    lock(key).withLock {
                        val draft = get(key)
                        withContext(Dispatchers.IO) {
                            draft.files
                                .flatMap { it.paths() }
                                .distinct()
                                .forEach { path ->
                                    val file = File(path)
                                    if (AttachmentFiles.owns(context, file) && file.exists())
                                        check(file.delete())
                                }
                            check(preferences.edit().remove(key).commit())
                        }
                        mutableDrafts.value -= key
                    }
                }
        }
    }

    private fun lock(key: String) = locks.getOrPut(key) { Mutex() }

    private var initialized = false
    private val restored =
        scope.async(Dispatchers.IO) {
            preferences.all
                .mapNotNull { (key, value) ->
                    if (value !is String) return@mapNotNull null
                    runCatching {
                        val json = JSONObject(value)
                        val array = json.getJSONArray("files")
                        val files =
                            (0 until array.length()).mapNotNull { i ->
                                val f = array.getJSONObject(i)
                                val file = File(f.getString("path"))
                                if (!AttachmentFiles.owns(context, file) || !file.isFile) null
                                else
                                    PendingAttachment(
                                        file.absolutePath,
                                        f.getString("name"),
                                        f.getString("mime"),
                                        file.length(),
                                        f.optBoolean("spoiler"),
                                        f.optString(
                                            "sourceExtension",
                                            attachmentExtension(f.getString("name")),
                                        ),
                                        f.optJSONObject("sanitized")?.let { clean ->
                                            val copy = File(clean.getString("path"))
                                            // A missing privacy copy must fail closed at upload;
                                            // never silently substitute its unprocessed source.
                                            check(AttachmentFiles.owns(context, copy))
                                            SanitizedAttachment(
                                                copy.absolutePath,
                                                clean.getString("mime"),
                                                clean.getLong("size"),
                                            )
                                        },
                                    )
                            }
                        val uncertain = json.optBoolean("sending") || json.optBoolean("uncertain")
                        key to
                            AttachmentDraft(
                                files,
                                uncertain = uncertain,
                                error =
                                    when {
                                        uncertain -> ui(R.string.upload_interrupted_description)
                                        files.size != array.length() ->
                                            ui(
                                                R.string
                                                    .some_attachments_are_no_longer_available_choose_them_again
                                            )
                                        else -> json.optString("error")
                                    },
                            )
                    }
                        .getOrNull()
                }
                .toMap()
        }

    init {
        scope.launch { ready() }
    }

    private suspend fun ready() {
        val saved = restored.await()
        if (!initialized) {
            initialized = true
            mutableDrafts.value = saved
        }
    }

    private fun get(key: String) = mutableDrafts.value[key] ?: AttachmentDraft()

    private fun update(key: String, value: AttachmentDraft) {
        mutableDrafts.value = mutableDrafts.value + (key to value)
    }

    private suspend fun persist(key: String) {
        val draft = get(key)
        val json =
            JSONObject()
                .put(
                    "files",
                    JSONArray().apply {
                        draft.files.forEach { file ->
                            put(
                                JSONObject()
                                    .put("path", file.path)
                                    .put("name", file.name)
                                    .put("mime", file.mime)
                                    .put("size", file.size)
                                    .put("spoiler", file.spoiler)
                                    .put("sourceExtension", file.sourceExtension)
                                    .put(
                                        "sanitized",
                                        file.sanitized?.let {
                                            JSONObject()
                                                .put("path", it.path)
                                                .put("mime", it.mime)
                                                .put("size", it.size)
                                        },
                                    )
                            )
                        }
                    },
                )
                .put("sending", draft.sending)
                .put("uncertain", draft.uncertain)
                .put("error", draft.error)
        withContext(Dispatchers.IO) {
            check(preferences.edit().putString(key, json.toString()).commit()) {
                ui(R.string.couldn_t_save_the_attachment_draft)
            }
        }
    }

    /** Options and generated files commit together; a failed save keeps the entire old draft. */
    suspend fun applyOptions(
        key: String,
        original: PendingAttachment,
        stem: String,
        spoiler: Boolean,
        removeMetadata: Boolean,
        image: io.github.chriexpe.papo.core.PreparedMediaImage? = null,
    ) =
        lock(key).withLock {
            ready()
            val previous = get(key)
            check(!previous.sending && !previous.preparing && !previous.uncertain) {
                ui(R.string.finish_the_current_attachment_action_before_editing)
            }
            check(
                original in previous.files && AttachmentFiles.owns(context, File(original.path))
            ) {
                ui(R.string.this_attachment_is_no_longer_in_the_draft)
            }
            val created = mutableListOf<File>()
            update(key, previous.copy(preparing = true, error = ""))
            var committed = false
            try {
                val replacement =
                    withContext(Dispatchers.IO) {
                        fun write(prepared: io.github.chriexpe.papo.core.PreparedMediaImage): File {
                            val file = AttachmentFiles.create(context, prepared.extension)
                            created += file
                            file.writeBytes(
                                android.util.Base64.decode(
                                    prepared.blob,
                                    android.util.Base64.DEFAULT,
                                )
                            )
                            return file
                        }
                        val source =
                            image?.let { prepared ->
                                val file = write(prepared)
                                original.copy(
                                    path = file.absolutePath,
                                    mime = prepared.mime,
                                    size = file.length(),
                                    sourceExtension = prepared.extension,
                                    sanitized = null,
                                )
                            } ?: original
                        val clean =
                            if (!removeMetadata) null
                            else
                                source.sanitized
                                    ?: run {
                                        val prepared =
                                            try {
                                                io.github.chriexpe.papo.core
                                                    .stripAttachmentMetadata(source.path)
                                            } catch (
                                                error:
                                                    io.github.chriexpe.papo.core.AttachmentImageException) {
                                                throw IOException(
                                                    ui(R.string.attachment_metadata_failed),
                                                    error,
                                                )
                                            }
                                        val file = write(prepared)
                                        SanitizedAttachment(
                                            file.absolutePath,
                                            prepared.mime,
                                            file.length(),
                                        )
                                    }
                        val extension =
                            if (clean != null) attachmentExtension(File(clean.path).name)
                            else source.sourceExtension
                        require(validAttachmentName(stem)) {
                            ui(R.string.attachment_filename_invalid)
                        }
                        val name = stem + if (extension.isEmpty()) "" else ".$extension"
                        val wireName =
                            if (spoiler && !name.startsWith("SPOILER_")) "SPOILER_$name" else name
                        require(validAttachmentName(wireName)) {
                            ui(R.string.attachment_filename_invalid)
                        }
                        source.copy(name = name, spoiler = spoiler, sanitized = clean)
                    }
                // Cancellation may not split persistence from publication/deletion.
                withContext(kotlinx.coroutines.NonCancellable) {
                    update(
                        key,
                        previous.copy(
                            files = previous.files.map { if (it == original) replacement else it },
                            error = "",
                        ),
                    )
                    try {
                        persist(key)
                    } catch (error: Exception) {
                        update(key, previous)
                        throw error
                    }
                    committed = true
                    withContext(Dispatchers.IO) {
                        (original.paths() - replacement.paths().toSet()).forEach {
                            File(it).delete()
                        }
                    }
                }
            } finally {
                if (!committed) {
                    update(key, previous)
                    withContext(kotlinx.coroutines.NonCancellable + Dispatchers.IO) {
                        created.forEach(File::delete)
                    }
                }
            }
        }

    /** Crop callers share the same atomic replacement and preserve per-file options. */
    suspend fun replaceEdited(
        key: String,
        original: PendingAttachment,
        image: io.github.chriexpe.papo.core.PreparedMediaImage,
    ) =
        applyOptions(
            key,
            original,
            attachmentStem(original.name),
            original.spoiler,
            original.sanitized != null,
            image,
        )

    fun report(key: String, message: String) = scope.launch {
        ready()
        if (!blocked(key)) update(key, get(key).copy(error = message))
    }

    fun stage(key: String, uris: List<Uri>) = scope.launch {
        lock(key).withLock {
            ready()
            if (uris.isEmpty() || blocked(key)) return@launch
            if (get(key).sending || get(key).preparing) {
                report(key, ui(R.string.wait_for_the_current_attachment_action_to_finish))
                return@launch
            }
            if (get(key).files.size + uris.size > 10) {
                report(key, ui(R.string.you_can_attach_up_to_ten_files_to_one_message))
                return@launch
            }
            update(key, get(key).copy(preparing = true, error = ""))
            val copied = mutableListOf<PendingAttachment>()
            try {
                withContext(Dispatchers.IO) {
                    uris.forEach { copied += AttachmentFiles.copy(context, it) }
                }
                update(key, get(key).copy(files = get(key).files + copied, preparing = false))
                persist(key)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val retained = get(key).files.map { it.path }.toSet()
                withContext(Dispatchers.IO) {
                    copied.filter { it.path !in retained }.forEach { File(it.path).delete() }
                }
                update(
                    key,
                    get(key)
                        .copy(
                            preparing = false,
                            error = e.message ?: ui(R.string.couldn_t_prepare_these_files),
                        ),
                )
            }
        }
    }

    fun finishCapture(
        key: String,
        keep: Boolean,
        name: String,
        stop: (Boolean) -> File?,
        onComplete: (Boolean) -> Unit,
    ) = scope.launch {
        val file = withContext(kotlinx.coroutines.NonCancellable + Dispatchers.IO) { stop(keep) }
        val staged = if (keep && file != null) addCaptured(key, file, name, "audio/mp4") else false
        if (keep && file == null)
            report(key, ui(R.string.the_recording_was_too_short_or_couldn_t_be_saved_try_again))
        onComplete(staged)
    }

    fun captured(key: String, file: File, name: String, mime: String) = scope.launch {
        addCaptured(key, file, name, mime)
    }

    private suspend fun addCaptured(key: String, file: File, name: String, mime: String): Boolean =
        lock(key).withLock {
            ready()
            if (blocked(key)) {
                file.delete()
                return@withLock false
            }
            if (!AttachmentFiles.owns(context, file) || !file.isFile || file.length() == 0L) {
                report(key, ui(R.string.the_capture_was_empty_try_again))
                return@withLock false
            }
            if (get(key).files.size >= 10) {
                file.delete()
                report(key, ui(R.string.you_can_attach_up_to_ten_files_to_one_message))
                return@withLock false
            }
            update(
                key,
                get(key)
                    .copy(
                        files =
                            get(key).files +
                                PendingAttachment(file.absolutePath, name, mime, file.length()),
                        error = "",
                    ),
            )
            try {
                persist(key)
                true
            } catch (e: Exception) {
                report(key, e.message ?: ui(R.string.couldn_t_save_this_attachment))
                false
            }
        }

    fun remove(key: String, file: PendingAttachment) = scope.launch {
        lock(key).withLock {
            ready()
            if (blocked(key) || get(key).sending || get(key).preparing) return@launch
            val remaining = get(key).files - file
            update(
                key,
                get(key)
                    .copy(
                        files = remaining,
                        uncertain = get(key).uncertain && remaining.isNotEmpty(),
                        error = "",
                    ),
            )
            try {
                persist(key)
                withContext(Dispatchers.IO) {
                    file.paths().forEach { path ->
                        File(path).takeIf { AttachmentFiles.owns(context, it) }?.delete()
                    }
                }
            } catch (e: Exception) {
                report(key, e.message ?: ui(R.string.couldn_t_remove_this_attachment))
            }
        }
    }

    fun send(
        key: String,
        server: ServerState,
        text: String,
        reply: String?,
        notifyReply: Boolean = true,
        onResult: (Boolean) -> Unit,
    ) =
        scope
            .launch {
                lock(key).withLock {
                    ready()
                    val draft = get(key)
                    if (blocked(key) || draft.sending || draft.preparing || draft.files.isEmpty()) {
                        onResult(false)
                        return@launch
                    }
                    update(key, draft.copy(sending = true, error = ""))
                    var dispatched = false
                    try {
                        // Persist the in-flight marker before dispatch. A restarted process
                        // cannot silently resend a POST whose outcome it never received.
                        persist(key)
                        dispatched = true
                        val result = upload(server, text, reply, notifyReply, draft.files)
                        if (result.accepted) {
                            update(
                                key,
                                get(key)
                                    .copy(
                                        files = get(key).files - draft.files.toSet(),
                                        sending = false,
                                        uncertain = false,
                                        error = "",
                                    ),
                            )
                            persist(key)
                            withContext(Dispatchers.IO) {
                                draft.files.forEach {
                                    it.paths().forEach { path ->
                                        File(path)
                                            .takeIf { file -> AttachmentFiles.owns(context, file) }
                                            ?.delete()
                                    }
                                }
                            }
                            onResult(true)
                        } else {
                            update(
                                key,
                                get(key)
                                    .copy(
                                        sending = false,
                                        uncertain = result.uncertain,
                                        error = result.error,
                                    ),
                            )
                            persist(key)
                            onResult(false)
                        }
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        update(
                            key,
                            get(key)
                                .copy(
                                    files = (get(key).files + draft.files).distinctBy { it.path },
                                    sending = false,
                                    uncertain = dispatched || draft.uncertain,
                                    error =
                                        e.message ?: ui(R.string.upload_unconfirmed_description),
                                ),
                        )
                        runCatching { persist(key) }
                        onResult(false)
                    }
                }
            }
            .also { job ->
                sendJobs[key] = job
                job.invokeOnCompletion {
                    scope.launch { if (sendJobs[key] === job) sendJobs.remove(key) }
                }
            }
}
