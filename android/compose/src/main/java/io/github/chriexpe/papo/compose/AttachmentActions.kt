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
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.result.PickVisualMediaRequest
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
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
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import coil3.compose.AsyncImage
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
internal fun AttachmentActions(key: String, model: PapoViewModel, enabled: Boolean, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var expanded by remember(key) { mutableStateOf(false) }
    var recorder by remember(key) { mutableStateOf(false) }
    var cameraPath by rememberSaveable { mutableStateOf("") }
    var cameraKey by rememberSaveable { mutableStateOf("") }
    var pickerKey by rememberSaveable { mutableStateOf("") }
    var galleryKey by rememberSaveable { mutableStateOf("") }
    val gallery = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(10)) { uris ->
        uris.forEach { uri -> runCatching { context.contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION) } }
        model.attachments.stage(galleryKey, uris)
    }
    val files = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        uris.forEach { uri -> runCatching { context.contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION) } }
        model.attachments.stage(pickerKey, uris)
    }
    val camera = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { accepted ->
        val path = cameraPath
        cameraPath = ""
        if (path.isNotEmpty()) {
            val file = File(path)
            if (accepted) model.attachments.captured(cameraKey, file, captureName("Photo", "jpg"), "image/jpeg")
            else file.delete()
        }
    }
    val microphone = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
        if (allowed) recorder = true
        else model.attachments.report(key, "Microphone permission is needed to record audio. You can enable it in Android settings.")
    }
    BackHandler(expanded) { expanded = false }
    Box(modifier.width(60.dp).height(60.dp)) {
        Column(Modifier.align(Alignment.BottomStart).offset(y = (-72).dp).wrapContentSize(Alignment.BottomStart, unbounded = true)) {
            AnimatedVisibility(expanded && enabled, enter = expandVertically(expandFrom = Alignment.Bottom) + fadeIn(), exit = shrinkVertically(shrinkTowards = Alignment.Bottom) + fadeOut()) {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp), modifier = Modifier.width(184.dp)) {
                    AttachmentAction("Record audio", Icons.Outlined.Mic) {
                        expanded = false
                        if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) recorder = true
                        else microphone.launch(Manifest.permission.RECORD_AUDIO)
                    }
                    AttachmentAction("Choose files", Icons.Outlined.AttachFile) {
                        expanded = false; pickerKey = key
                        runCatching { files.launch(arrayOf("*/*")) }.onFailure { model.attachments.report(key, "No file picker is available on this device.") }
                    }
                    AttachmentAction("Camera", Icons.Outlined.PhotoCamera) {
                        expanded = false
                        scope.launch {
                            val file = withContext(Dispatchers.IO) { AttachmentFiles.create(context, "jpg") }
                            cameraPath = file.absolutePath; cameraKey = key
                            try { camera.launch(FileProvider.getUriForFile(context, "${context.packageName}.files", file)) }
                            catch (e: Exception) { file.delete(); cameraPath = ""; model.attachments.report(key, "No camera app is available on this device.") }
                        }
                    }
                }
            }
        }
        GlassPill(Modifier.fillMaxSize(), radius = 20) {
            Box(Modifier.fillMaxSize().clip(RoundedCornerShape(20.dp))
                .combinedClickable(enabled = enabled, role = Role.Button,
                    onClickLabel = "Open gallery", onLongClickLabel = "More attachment options",
                    onClick = {
                        expanded = false; galleryKey = key
                        runCatching { gallery.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo)) }
                            .onFailure { model.attachments.report(key, "No gallery picker is available on this device.") }
                    }, onLongClick = { expanded = !expanded })
                .semantics { contentDescription = "Open gallery"; stateDescription = if (expanded) "Expanded" else "Collapsed" },
                contentAlignment = Alignment.Center) {
                Icon(Icons.Outlined.PhotoLibrary, null, tint = MaterialTheme.colorScheme.onSurface.copy(alpha = if (enabled) 1f else .38f))
            }
        }
    }

    if (recorder) AudioRecordingDialog(key, model) { recorder = false }
}

@Composable
private fun AttachmentAction(label: String, icon: ImageVector, onClick: () -> Unit) {
    GlassPill(Modifier.fillMaxWidth(), radius = 20) {
        TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth().heightIn(min = 52.dp), contentPadding = PaddingValues(horizontal = 16.dp)) {
            Icon(icon, null, Modifier.size(24.dp)); Spacer(Modifier.width(12.dp))
            Text(label, Modifier.weight(1f))
        }
    }
}

@Composable
internal fun PendingAttachments(draft: AttachmentDraft, remove: (PendingAttachment) -> Unit) {
    if (draft.files.isNotEmpty()) Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        draft.files.forEach { file ->
            Row(Modifier.width(220.dp).clip(RoundedCornerShape(16.dp)).background(MaterialTheme.colorScheme.surfaceContainer).padding(start = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                if (file.mime.startsWith("image/")) AsyncImage(File(file.path), "Selected ${file.name}", modifier = Modifier.size(44.dp).clip(RoundedCornerShape(10.dp)), contentScale = androidx.compose.ui.layout.ContentScale.Crop)
                else Icon(if (file.mime.startsWith("audio/")) Icons.Outlined.Mic else Icons.Outlined.InsertDriveFile, null, Modifier.size(36.dp).padding(4.dp))
                Column(Modifier.weight(1f).padding(start = 8.dp)) {
                    Text(file.name, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.labelMedium)
                    Text(if (file.size >= 1024 * 1024) "${file.size / (1024 * 1024)} MB" else "${(file.size / 1024).coerceAtLeast(1)} KB", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                IconButton(onClick = { remove(file) }, enabled = !draft.sending && !draft.preparing) { Icon(Icons.Outlined.Close, "Remove ${file.name}", Modifier.size(18.dp)) }
            }
        }
    }
    if (draft.preparing || draft.sending) Row(Modifier.padding(horizontal = 18.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp); Spacer(Modifier.width(8.dp))
        Text(if (draft.sending) "Sending attachments…" else "Preparing attachments…", style = MaterialTheme.typography.labelMedium)
    }
    if (draft.error.isNotBlank()) Text(draft.error, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 18.dp, vertical = 8.dp))
}

private fun captureName(prefix: String, extension: String) = "$prefix ${SimpleDateFormat("yyyy-MM-dd HH.mm.ss", Locale.ROOT).format(Date())}.$extension"

private class AudioCapture(private val context: Context) {
    private var recorder: MediaRecorder? = null
    private var file: File? = null
    @Synchronized fun start() {
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
            input.prepare(); input.start()
        } catch (e: Exception) { stop(false); throw e }
    }
    @Synchronized fun stop(keep: Boolean): File? {
        val input = recorder ?: return null
        recorder = null
        val output = file; file = null
        val stopped = runCatching { input.stop() }.isSuccess
        input.release()
        return if (keep && stopped && output?.isFile == true && output.length() > 0) output
        else { output?.delete(); null }
    }
}

@Composable
private fun AudioRecordingDialog(key: String, model: PapoViewModel, close: () -> Unit) {
    val context = LocalContext.current
    val capture = remember { AudioCapture(context.applicationContext) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var recording by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var started by remember { mutableLongStateOf(0L) }
    var elapsed by remember { mutableLongStateOf(0L) }
    val finish: (Boolean) -> Unit = { keep ->
        if (!busy) {
            busy = true
            model.attachments.finishCapture(key, keep, captureName("Voice note", "m4a"), capture::stop, close)
        }
    }
    val latestFinish by rememberUpdatedState(finish)
    LaunchedEffect(capture) {
        try { withContext(Dispatchers.IO) { capture.start() }; started = SystemClock.elapsedRealtime(); recording = true }
        catch (e: Exception) { error = "Couldn’t start the microphone. Check its permission and try again." }
    }
    LaunchedEffect(recording) {
        while (recording) { elapsed = (SystemClock.elapsedRealtime() - started) / 1000; delay(250) }
    }
    DisposableEffect(capture, lifecycle) {
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_PAUSE) latestFinish(true) }
        lifecycle.addObserver(observer)
        onDispose {
            lifecycle.removeObserver(observer)
            if (!busy) { busy = true; model.attachments.finishCapture(key, true, captureName("Voice note", "m4a"), capture::stop) {} }
        }
    }
    AlertDialog(onDismissRequest = { finish(false) }, icon = { Icon(Icons.Outlined.Mic, null, tint = MaterialTheme.colorScheme.error) },
        title = { Text("Record audio") },
        text = { Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
            if (error.isNotBlank()) Text(error)
            else { Text("%d:%02d".format(elapsed / 60, elapsed % 60), style = MaterialTheme.typography.headlineLarge); Text(if (recording) "Recording" else "Starting microphone…", style = MaterialTheme.typography.bodyMedium) }
        } },
        confirmButton = { TextButton(onClick = { finish(true) }, enabled = recording && !busy) { Text("Stop and attach") } },
        dismissButton = { TextButton(onClick = { finish(false) }, enabled = !busy) { Text("Cancel") } })
}
