@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import coil3.compose.AsyncImage
import io.github.chriexpe.papo.core.MediaEditPurpose
import io.github.chriexpe.papo.core.PreparedMediaImage
import java.io.File
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Disposal can race an IO callback. Registration and closing share one lock. */
private class AttachmentOptionFiles {
    private val files = mutableListOf<File>()
    private var closed = false

    @Synchronized
    fun open(directory: File, extension: String): Pair<File, java.io.OutputStream> {
        check(!closed)
        val file = File.createTempFile("crop-", ".$extension", directory)
        try {
            // Open before publishing ownership: close() may unlink an open file,
            // but a late writer cannot recreate its path after disposal.
            val output = file.outputStream()
            files += file
            return file to output
        } catch (error: Exception) {
            file.delete()
            throw error
        }
    }

    @Synchronized
    fun close() {
        closed = true
        files.forEach(File::delete)
    }
}

private val privacyImageTypes = setOf("image/jpeg", "image/png", "image/webp", "image/gif")

/** The sheet owns tentative edits. Only Apply publishes files or options to the durable draft. */
@Composable
internal fun AttachmentOptions(
    original: PendingAttachment,
    draftKey: String,
    server: ServerState,
    model: PapoViewModel,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var stem by rememberSaveable(original.path) { mutableStateOf(attachmentStem(original.name)) }
    var spoiler by rememberSaveable(original.path) { mutableStateOf(original.spoiler) }
    var removeMetadata by
        rememberSaveable(original.path) { mutableStateOf(original.sanitized != null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var cropSource by remember { mutableStateOf<MediaEditorSource?>(null) }
    var edited by remember { mutableStateOf<PreparedMediaImage?>(null) }
    var preview by remember { mutableStateOf<File?>(null) }
    val temporaryFiles = remember { AttachmentOptionFiles() }
    // Temporary crop previews belong to this options transaction, not the media
    // editor: cancelling either screen must not touch the durable attachment.
    DisposableEffect(original.path) {
        onDispose { temporaryFiles.close() }
    }
    val extension =
        edited?.extension
            ?: if (removeMetadata && original.sanitized != null) attachmentExtension(original.name)
            else original.sourceExtension
    val filename =
        (if (spoiler && !stem.startsWith("SPOILER_")) "SPOILER_" else "") +
            stem +
            if (extension.isEmpty()) "" else ".$extension"
    val valid = validAttachmentName(stem) && validAttachmentName(filename)
    val canStrip = (edited?.mime ?: original.mime) in privacyImageTypes
    val source = cropSource
    if (source != null) {
        MediaEditor(
            source,
            MediaEditPurpose.ATTACHMENT,
            server,
            model,
            onDismiss = { cropSource = null },
        ) { prepared, _ ->
            val file =
                withContext(Dispatchers.IO) {
                    val directory = File(context.cacheDir, "attachment-options").apply { mkdirs() }
                    directory
                        .listFiles()
                        .orEmpty()
                        .filter {
                            System.currentTimeMillis() - it.lastModified() > 24 * 60 * 60 * 1000L
                        }
                        .forEach(File::delete)
                    val (file, output) = temporaryFiles.open(directory, prepared.extension)
                    output.use {
                        it.write(
                            android.util.Base64.decode(prepared.blob, android.util.Base64.DEFAULT)
                        )
                    }
                    file
                }
            preview = file
            edited = prepared
        }
        return
    }
    ModalBottomSheet(
        onDismissRequest = { if (!busy) onDismiss() },
        sheetState =
            rememberModalBottomSheetState(
                skipPartiallyExpanded = true,
                confirmValueChange = { !busy },
            ),
    ) {
        Column(
            Modifier.fillMaxWidth()
                .imePadding()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp)
                .padding(bottom = 20.dp)
                .testTag("attachment-options"),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(ui(R.string.attachment_options), style = MaterialTheme.typography.titleLarge)
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                AttachmentThumbnail(
                    preview?.absolutePath ?: original.path,
                    original.mime,
                    spoiler,
                    Modifier.size(76.dp),
                )
                Column(Modifier.weight(1f)) {
                    Text(
                        filename,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.titleSmall,
                    )
                    Text(
                        attachmentSize(edited?.bytes?.toLong() ?: original.uploadSize),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            OutlinedTextField(
                value = stem,
                onValueChange = {
                    if (it.startsWith("SPOILER_")) {
                        spoiler = true
                        stem = it.removePrefix("SPOILER_")
                    } else stem = it
                    error = ""
                },
                enabled = !busy,
                label = { Text(ui(R.string.attachment_filename)) },
                prefix =
                    if (spoiler && !stem.startsWith("SPOILER_")) ({ Text("SPOILER_") }) else null,
                suffix = if (extension.isNotEmpty()) ({ Text(".$extension") }) else null,
                trailingIcon = {
                    IconButton(
                        onClick = { stem = UUID.randomUUID().toString().replace("-", "").take(16) },
                        enabled = !busy,
                    ) {
                        Icon(Icons.Outlined.Shuffle, ui(R.string.attachment_random_name))
                    }
                },
                singleLine = true,
                isError = !valid,
                supportingText =
                    if (!valid) ({ Text(ui(R.string.attachment_filename_invalid)) }) else null,
                modifier = Modifier.fillMaxWidth().testTag("attachment-filename"),
            )
            AttachmentOptionToggle(
                ui(R.string.attachment_spoiler),
                ui(R.string.attachment_spoiler_description),
                spoiler,
                !busy,
                "attachment-spoiler",
            ) {
                spoiler = it
            }
            if (canStrip)
                AttachmentOptionToggle(
                    ui(R.string.attachment_remove_metadata),
                    ui(R.string.attachment_metadata_description),
                    removeMetadata,
                    !busy,
                    "attachment-metadata",
                ) {
                    removeMetadata = it
                }
            if (original.mime.startsWith("image/")) {
                OutlinedButton(
                    enabled = !busy,
                    modifier = Modifier.fillMaxWidth().testTag("attachment-edit-image"),
                    onClick = {
                        busy = true
                        error = ""
                        scope.launch {
                            try {
                                cropSource =
                                    withContext(Dispatchers.IO) {
                                        val file =
                                            preview?.let { original.copy(path = it.absolutePath) }
                                                ?: original
                                        MediaEditorSource.attachment(file)
                                    }
                            } catch (e: CancellationException) {
                                throw e
                            } catch (_: Exception) {
                                error = ui(R.string.couldn_t_edit_this_image)
                            } finally {
                                busy = false
                            }
                        }
                    },
                ) {
                    Icon(Icons.Outlined.Crop, null, Modifier.size(20.dp))
                    Spacer(Modifier.width(8.dp))
                    Text(ui(R.string.edit_image))
                }
                Text(
                    ui(R.string.attachment_reencode_description),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (error.isNotEmpty())
                Text(
                    error,
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedButton(
                    onClick = onDismiss,
                    enabled = !busy,
                    modifier = Modifier.weight(1f),
                ) {
                    Text(ui(R.string.cancel))
                }
                Button(
                    enabled = valid && !busy,
                    modifier = Modifier.weight(1f).testTag("apply-attachment-options"),
                    onClick = {
                        busy = true
                        error = ""
                        scope.launch {
                            try {
                                model.attachments.applyOptions(
                                    draftKey,
                                    original,
                                    stem,
                                    spoiler,
                                    removeMetadata,
                                    edited,
                                )
                                onDismiss()
                            } catch (e: CancellationException) {
                                throw e
                            } catch (e: Exception) {
                                error = e.message ?: ui(R.string.attachment_options_failed)
                            } finally {
                                busy = false
                            }
                        }
                    },
                ) {
                    if (busy) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                    else Text(ui(R.string.attachment_apply))
                }
            }
        }
    }
}

@Composable
private fun AttachmentOptionToggle(
    title: String,
    description: String,
    checked: Boolean,
    enabled: Boolean,
    tag: String,
    onChange: (Boolean) -> Unit,
) {
    Row(
        Modifier.fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .background(MaterialTheme.colorScheme.surfaceContainer)
            .clickable(enabled = enabled, role = Role.Switch) { onChange(!checked) }
            .padding(14.dp)
            .testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleSmall)
            Text(
                description,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Switch(checked = checked, onCheckedChange = onChange, enabled = enabled)
    }
}

internal fun attachmentSize(bytes: Long): String =
    if (bytes >= 1024 * 1024) "${bytes / (1024 * 1024)} MB"
    else "${(bytes / 1024).coerceAtLeast(1)} KB"

/** Shared thumbnail treatment for the pending strip and its options sheet. */
@Composable
internal fun AttachmentThumbnail(
    path: String,
    mime: String,
    spoiler: Boolean,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier
            .clip(RoundedCornerShape(12.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh),
        contentAlignment = Alignment.Center,
    ) {
        if (spoiler) Icon(Icons.Outlined.VisibilityOff, ui(R.string.attachment_spoiler))
        else if (mime.startsWith("image/"))
            AsyncImage(
                File(path),
                null,
                imageLoader = MediaImages.loader(LocalContext.current),
                modifier = Modifier.fillMaxSize(),
                contentScale = ContentScale.Crop,
            )
        else
            Icon(
                if (mime.startsWith("audio/")) Icons.Outlined.Mic
                else if (mime.startsWith("video/")) Icons.Outlined.Videocam
                else Icons.Outlined.InsertDriveFile,
                null,
            )
    }
}
