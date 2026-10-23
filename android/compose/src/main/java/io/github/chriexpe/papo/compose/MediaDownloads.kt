package io.github.chriexpe.papo.compose

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.MediaStore
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/** Android Downloads publish after copying; document providers remove incomplete saves. */
internal suspend fun saveMediaDownload(
    context: Context,
    file: File,
    name: String,
    mime: String,
): Uri =
    withContext(Dispatchers.IO) {
        val resolver = context.contentResolver
        val safeName =
            name.replace(Regex("[/\\\\\\p{Cntrl}]"), "_").take(180).ifBlank {
                ui(R.string.papo_media)
            }
        val tree =
            context
                .getSharedPreferences("compose-appearance", 0)
                .getString("download-tree", "")
                .orEmpty()
        if (tree.isNotBlank()) {
            val root = Uri.parse(tree)
            val parent =
                DocumentsContract.buildDocumentUriUsingTree(
                    root,
                    DocumentsContract.getTreeDocumentId(root),
                )
            val uri =
                DocumentsContract.createDocument(resolver, parent, mime, safeName)
                    ?: error(ui(R.string.couldn_t_create_the_download_in_your_chosen_folder))
            try {
                requireNotNull(resolver.openOutputStream(uri, "w")).use { out ->
                    file.inputStream().use { input ->
                        val work = kotlinx.coroutines.currentCoroutineContext()
                        val buffer = ByteArray(64 * 1024)
                        while (true) {
                            work.ensureActive()
                            val count = input.read(buffer)
                            if (count < 0) break
                            out.write(buffer, 0, count)
                        }
                    }
                }
                kotlinx.coroutines.currentCoroutineContext().ensureActive()
                return@withContext uri
            } catch (e: Exception) {
                runCatching { DocumentsContract.deleteDocument(resolver, uri) }
                throw e
            }
        }
        val values =
            ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, safeName)
                put(MediaStore.MediaColumns.MIME_TYPE, mime)
                put(
                    MediaStore.MediaColumns.RELATIVE_PATH,
                    "${Environment.DIRECTORY_DOWNLOADS}/Papo",
                )
                put(MediaStore.MediaColumns.IS_PENDING, 1)
            }
        val uri =
            resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
                ?: error(ui(R.string.couldn_t_create_the_download))
        try {
            val work = kotlinx.coroutines.currentCoroutineContext()
            requireNotNull(resolver.openOutputStream(uri)).use { output ->
                file.inputStream().use { input ->
                    val buffer = ByteArray(64 * 1024)
                    while (true) {
                        work.ensureActive()
                        val count = input.read(buffer)
                        if (count < 0) break
                        output.write(buffer, 0, count)
                    }
                }
            }
            work.ensureActive()
            check(
                resolver.update(
                    uri,
                    ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) },
                    null,
                    null,
                ) == 1
            ) {
                ui(R.string.couldn_t_finish_the_download)
            }
            uri
        } catch (e: Exception) {
            resolver.delete(uri, null, null)
            throw e
        }
    }

internal fun downloadLocation(context: Context): String {
    val tree =
        context
            .getSharedPreferences("compose-appearance", 0)
            .getString("download-tree", "")
            .orEmpty()
    if (tree.isBlank()) return "Downloads/Papo"
    return context
        .getSharedPreferences("compose-appearance", 0)
        .getString("download-label", "Selected folder")
        .orEmpty()
        .ifBlank { ui(R.string.selected_folder) }
}
