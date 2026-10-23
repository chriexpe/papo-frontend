@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)

package io.github.chriexpe.papo.compose

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.MediaRecorder
import android.os.SystemClock
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Anchored to the left of the composer; the roll-up overlays the timeline. */
@Composable
internal fun AttachmentActions(
    key: String,
    model: PapoViewModel,
    enabled: Boolean,
    onRecord: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var expanded by remember(key) { mutableStateOf(false) }
    var cameraPath by rememberSaveable { mutableStateOf("") }
    var cameraKey by rememberSaveable { mutableStateOf("") }
    var pickerKey by rememberSaveable { mutableStateOf("") }
    var galleryKey by rememberSaveable { mutableStateOf("") }
    val gallery =
        rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(10)) {
            uris ->
            uris.forEach { uri ->
                runCatching {
                    context.contentResolver.takePersistableUriPermission(
                        uri,
                        Intent.FLAG_GRANT_READ_URI_PERMISSION,
                    )
                }
            }
            model.attachments.stage(galleryKey, uris)
        }
    val files =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
            uris.forEach { uri ->
                runCatching {
                    context.contentResolver.takePersistableUriPermission(
                        uri,
                        Intent.FLAG_GRANT_READ_URI_PERMISSION,
                    )
                }
            }
            model.attachments.stage(pickerKey, uris)
        }
    val camera =
        rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { accepted ->
            val path = cameraPath
            cameraPath = ""
            if (path.isNotEmpty()) {
                val file = File(path)
                if (accepted)
                    model.attachments.captured(
                        cameraKey,
                        file,
                        captureName("Photo", "jpg"),
                        "image/jpeg",
                    )
                else file.delete()
            }
        }
    val microphone =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
            if (allowed) onRecord()
            else
                model.attachments.report(
                    key,
                    ui(R.string.microphone_permission_description),
                )
        }
    BackHandler(expanded) { expanded = false }
    Box(modifier.size(LocalUiDimensions.current.attachmentSize)) {
        Column(
            Modifier.align(Alignment.BottomStart)
                .offset(y = (-72).dp)
                .wrapContentSize(Alignment.BottomStart, unbounded = true)
        ) {
            AnimatedVisibility(
                expanded && enabled,
                enter = expandVertically(expandFrom = Alignment.Bottom) + fadeIn(),
                exit = shrinkVertically(shrinkTowards = Alignment.Bottom) + fadeOut(),
            ) {
                Column(
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                    modifier = Modifier.width(184.dp),
                ) {
                    AttachmentAction(ui(R.string.record_audio), Icons.Outlined.Mic) {
                        expanded = false
                        if (
                            ContextCompat.checkSelfPermission(
                                context,
                                Manifest.permission.RECORD_AUDIO,
                            ) == PackageManager.PERMISSION_GRANTED
                        )
                            onRecord()
                        else microphone.launch(Manifest.permission.RECORD_AUDIO)
                    }
                    AttachmentAction(ui(R.string.choose_files), Icons.Outlined.AttachFile) {
                        expanded = false
                        pickerKey = key
                        runCatching { files.launch(arrayOf("*/*")) }
                            .onFailure {
                                model.attachments.report(
                                    key,
                                    ui(R.string.no_file_picker_is_available_on_this_device),
                                )
                            }
                    }
                    AttachmentAction("Camera", Icons.Outlined.PhotoCamera) {
                        expanded = false
                        scope.launch {
                            val file =
                                withContext(Dispatchers.IO) {
                                    AttachmentFiles.create(context, "jpg")
                                }
                            cameraPath = file.absolutePath
                            cameraKey = key
                            try {
                                camera.launch(
                                    FileProvider.getUriForFile(
                                        context,
                                        "${context.packageName}.files",
                                        file,
                                    )
                                )
                            } catch (e: Exception) {
                                file.delete()
                                cameraPath = ""
                                model.attachments.report(
                                    key,
                                    ui(R.string.no_camera_app_is_available_on_this_device),
                                )
                            }
                        }
                    }
                }
            }
        }
        GlassPill(Modifier.fillMaxSize(), radius = 20) {
            Box(
                Modifier.fillMaxSize()
                    .clip(RoundedCornerShape(20.dp))
                    .combinedClickable(
                        enabled = enabled,
                        role = Role.Button,
                        onClickLabel = ui(R.string.open_gallery),
                        onLongClickLabel = ui(R.string.more_attachment_options),
                        onClick = {
                            expanded = false
                            galleryKey = key
                            runCatching {
                                gallery.launch(
                                    PickVisualMediaRequest(
                                        ActivityResultContracts.PickVisualMedia.ImageAndVideo
                                    )
                                )
                            }
                                .onFailure {
                                    model.attachments.report(
                                        key,
                                        ui(R.string.no_gallery_picker_is_available_on_this_device),
                                    )
                                }
                        },
                        onLongClick = { expanded = !expanded },
                    )
                    .semantics {
                        contentDescription = ui(R.string.open_gallery)
                        stateDescription =
                            if (expanded) ui(R.string.expanded) else ui(R.string.collapsed)
                    },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    Icons.Outlined.PhotoLibrary,
                    null,
                    tint =
                        MaterialTheme.colorScheme.onSurface.copy(alpha = if (enabled) 1f else .38f),
                )
            }
        }
    }
}

@Composable
private fun AttachmentAction(label: String, icon: ImageVector, onClick: () -> Unit) {
    GlassPill(Modifier.fillMaxWidth(), radius = 20) {
        TextButton(
            onClick = onClick,
            modifier = Modifier.fillMaxWidth().heightIn(min = 52.dp),
            contentPadding = PaddingValues(horizontal = 16.dp),
        ) {
            Icon(icon, null, Modifier.size(24.dp))
            Spacer(Modifier.width(12.dp))
            Text(label, Modifier.weight(1f))
        }
    }
}

@Composable
internal fun PendingAttachments(
    draft: AttachmentDraft,
    edit: (PendingAttachment) -> Unit,
    remove: (PendingAttachment) -> Unit,
) {
    if (draft.files.isNotEmpty())
        Row(
            Modifier.fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 12.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            draft.files.forEach { file ->
                PendingAttachmentTile(
                    file,
                    enabled = !draft.sending && !draft.preparing && !draft.uncertain,
                    removable = !draft.sending && !draft.preparing,
                    edit = { edit(file) },
                    remove = { remove(file) },
                )
            }
        }

    if (draft.preparing || draft.sending)
        Row(
            Modifier.padding(horizontal = 18.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
            Spacer(Modifier.width(8.dp))
            Text(
                if (draft.sending) ui(R.string.sending_attachments)
                else ui(R.string.preparing_attachments),
                style = MaterialTheme.typography.labelMedium,
            )
        }
    if (draft.error.isNotBlank())
        Text(
            draft.error,
            color = MaterialTheme.colorScheme.error,
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(horizontal = 18.dp, vertical = 8.dp),
        )
}

/**
 * Compact tiles leave room for several selections; the thumbnail opens the shared sheet pattern.
 */
@Composable
private fun PendingAttachmentTile(
    file: PendingAttachment,
    enabled: Boolean,
    removable: Boolean,
    edit: () -> Unit,
    remove: () -> Unit,
) {
    Column(
        Modifier.width(104.dp)
            .clip(RoundedCornerShape(14.dp))
            .background(MaterialTheme.colorScheme.surfaceContainer)
            .clickable(enabled = enabled, role = Role.Button, onClick = edit)
            .semantics { contentDescription = ui(R.string.attachment_options_named, file.wireName) }
            .padding(6.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Box(Modifier.fillMaxWidth().height(72.dp)) {
            AttachmentThumbnail(file.path, file.mime, file.spoiler, Modifier.fillMaxSize())
            FilledTonalIconButton(
                onClick = remove,
                enabled = removable,
                shape = RoundedCornerShape(10.dp),
                modifier = Modifier.align(Alignment.TopEnd).size(32.dp),
            ) {
                Icon(
                    Icons.Outlined.Close,
                    ui(R.string.remove_named, file.name),
                    Modifier.size(18.dp),
                )
            }
        }
        Text(
            file.wireName,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            style = MaterialTheme.typography.labelSmall,
        )
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                attachmentSize(file.uploadSize),
                Modifier.weight(1f),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (file.spoiler)
                Icon(
                    Icons.Outlined.VisibilityOff,
                    ui(R.string.attachment_spoiler),
                    Modifier.size(12.dp),
                )
            if (file.sanitized != null)
                Icon(
                    Icons.Outlined.Shield,
                    ui(R.string.attachment_metadata_removed),
                    Modifier.size(12.dp),
                )
        }
    }
}

private fun captureName(prefix: String, extension: String) =
    "$prefix ${SimpleDateFormat("yyyy-MM-dd HH.mm.ss", Locale.ROOT).format(Date())}.$extension"

private class AudioCapture(private val context: Context) {
    private var recorder: MediaRecorder? = null
    private var file: File? = null

    @Synchronized
    fun start() {
        val output = AttachmentFiles.create(context, "m4a")
        file = output
        val input = MediaRecorder(context)
        recorder = input
        try {
            input.setAudioSource(MediaRecorder.AudioSource.MIC)
            input.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
            input.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
            input.setAudioEncodingBitRate(96_000)
            input.setAudioSamplingRate(44_100)
            input.setOutputFile(output.absolutePath)
            input.prepare()
            input.start()
        } catch (e: Exception) {
            stop(false)
            throw e
        }
    }

    @Synchronized fun amplitude(): Int = runCatching { recorder?.maxAmplitude ?: 0 }.getOrDefault(0)

    @Synchronized
    fun stop(keep: Boolean): File? {
        val input = recorder ?: return null
        recorder = null
        val output = file
        file = null
        val stopped = runCatching { input.stop() }.isSuccess
        input.release()
        return if (keep && stopped && output?.isFile == true && output.length() > 0) output
        else {
            output?.delete()
            null
        }
    }
}

/** Waveform bars are peaks measured from the active recorder, never animation fixtures. */
@Composable
internal fun RecordingComposer(
    key: String,
    model: PapoViewModel,
    onClose: () -> Unit,
    onSend: () -> Unit,
) {
    val context = LocalContext.current
    val capture = remember { AudioCapture(context.applicationContext) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var recording by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var started by remember { mutableLongStateOf(0L) }
    var elapsed by remember { mutableLongStateOf(0L) }
    val samples = remember { mutableStateListOf<Float>() }
    val finish: (Boolean, Boolean) -> Unit = { keep, send ->
        if (!busy) {
            busy = true
            model.attachments.finishCapture(
                key,
                keep && recording,
                captureName(ui(R.string.voice_note), "m4a"),
                capture::stop,
            ) { staged ->
                onClose()
                if (staged && send) onSend()
            }
        }
    }
    val latestFinish by rememberUpdatedState(finish)
    // Leaving the recorder creates an unsent attachment rather than navigating
    // away from the conversation or silently losing the captured audio.
    BackHandler { finish(true, false) }
    LaunchedEffect(capture) {
        try {
            withContext(Dispatchers.IO) { capture.start() }
            started = SystemClock.elapsedRealtime()
            recording = true
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e
        } catch (e: Exception) {
            error = ui(R.string.microphone_unavailable)
        }
    }
    LaunchedEffect(recording, busy) {
        while (recording && !busy) {
            elapsed = (SystemClock.elapsedRealtime() - started) / 1000
            val peak = withContext(Dispatchers.IO) { capture.amplitude() }
            samples.add(kotlin.math.sqrt(peak / 32767f).coerceIn(0f, 1f))
            if (samples.size > 96) samples.removeAt(0)
            delay(80)
        }
    }
    DisposableEffect(capture, lifecycle) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_PAUSE) latestFinish(true, false)
        }
        lifecycle.addObserver(observer)
        onDispose {
            lifecycle.removeObserver(observer)
            if (!busy) {
                busy = true
                model.attachments.finishCapture(
                    key,
                    true,
                    captureName(ui(R.string.voice_note), "m4a"),
                    capture::stop,
                ) {}
            }
        }
    }
    Row(
        Modifier.fillMaxWidth().padding(6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        IconButton(onClick = { finish(false, false) }, enabled = !busy) {
            Icon(
                Icons.Outlined.DeleteOutline,
                ui(R.string.discard_recording),
                tint = MaterialTheme.colorScheme.error,
            )
        }
        Text(
            "%d:%02d".format(elapsed / 60, elapsed % 60),
            style = MaterialTheme.typography.labelLarge,
            fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
        )
        if (error.isNotEmpty())
            Text(
                error,
                Modifier.weight(1f),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.error,
            )
        else {
            val color = MaterialTheme.colorScheme.primary
            androidx.compose.foundation.Canvas(
                Modifier.weight(1f).height(28.dp).semantics {
                    contentDescription = ui(R.string.live_microphone_waveform)
                }
            ) {
                val gap = 4.dp.toPx()
                val count = (size.width / gap).toInt().coerceAtLeast(1)
                val visible = samples.takeLast(count)
                visible.forEachIndexed { index, level ->
                    val x = size.width - (visible.size - index) * gap + gap / 2
                    val half = (level * size.height / 2).coerceAtLeast(1.dp.toPx())
                    drawLine(
                        color,
                        androidx.compose.ui.geometry.Offset(x, size.height / 2 - half),
                        androidx.compose.ui.geometry.Offset(x, size.height / 2 + half),
                        2.dp.toPx(),
                        androidx.compose.ui.graphics.StrokeCap.Round,
                    )
                }
            }
        }
        FilledIconButton(
            onClick = { finish(true, true) },
            enabled = recording && !busy,
            modifier = Modifier.size(48.dp),
        ) {
            if (busy) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
            else Icon(Icons.Outlined.ArrowUpward, ui(R.string.send_recording))
        }
    }
}
