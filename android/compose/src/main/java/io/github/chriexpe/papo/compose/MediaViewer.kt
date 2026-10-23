package io.github.chriexpe.papo.compose

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.ActivityInfo
import android.graphics.BitmapFactory
import android.webkit.MimeTypeMap
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.*
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import coil3.request.ImageRequest
import dev.chrisbanes.haze.HazeState
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.EmbedView
import io.github.chriexpe.papo.core.MediaResource
import java.io.File
import java.util.Locale
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal data class ViewerMedia(
    val id: String,
    val title: String,
    val kind: String,
    val mime: String = "",
    val width: Int = 0,
    val height: Int = 0,
    val resource: MediaResource? = null,
    val embed: EmbedView? = null,
    val gif: GifChoice? = null,
    val url: String = "",
    val playback: PlaybackHandle? = null,
)

internal val LocalMediaViewer =
    staticCompositionLocalOf<(ViewerMedia) -> Unit> { error(ui(R.string.missing_media_viewer)) }
internal val LocalViewedMedia = compositionLocalOf<ViewerMedia?> { null }
internal val LocalInlinePlayer = compositionLocalOf<String?> { null }

private tailrec fun Context.activity(): Activity? =
    when (this) {
        is Activity -> this
        is ContextWrapper -> baseContext.activity()
        else -> null
    }

@Composable
internal fun MediaViewer(
    server: ServerState,
    model: PapoViewModel,
    media: ViewerMedia,
    onClose: () -> Unit,
) {
    // Keep the viewer ahead of conversation navigation even when a focused
    // native PlayerView routes Back through the activity's dispatcher.
    BackHandler(onBack = onClose)
    val context = LocalContext.current
    val activity = remember(context) { context.activity() }
    // FULL_SENSOR follows physical rotation even when the user's auto-rotate
    // setting is off. The policy belongs only to an open media viewer.
    DisposableEffect(activity, media.kind) {
        val previous = activity?.requestedOrientation
        activity?.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR
        onDispose { if (previous != null) activity?.requestedOrientation = previous }
    }
    var ratio by
        remember(media.id) {
            mutableFloatStateOf(
                if (media.width > 0 && media.height > 0) media.width.toFloat() / media.height
                else if (media.kind == "audio") 3f else 16f / 9f
            )
        }
    var scale by rememberSaveable(media.id) { mutableFloatStateOf(1f) }
    var offset by remember(media.id) { mutableStateOf(Offset.Zero) }
    var download by rememberSaveable(media.id) { mutableStateOf("") }
    var saving by remember(media.id) { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val density = LocalDensity.current
    val haze = remember { HazeState() }
    val colors =
        MaterialTheme.colorScheme.copy(
            background = Color.Black,
            surface = Color(0xFF242428),
            onSurface = Color.White,
            onSurfaceVariant = Color(0xFFCBCBD1),
        )
    Dialog(
        onDismissRequest = onClose,
        properties =
            DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false),
    ) {
        MaterialTheme(colorScheme = colors) {
            CompositionLocalProvider(
                LocalHaze provides haze,
                LocalContentColor provides Color.White,
            ) {
                BoxWithConstraints(
                    Modifier.fillMaxSize()
                        .background(Color.Black)
                        .clipToBounds()
                        .testTag("media-viewer")
                ) {
                    val contentHeight = if (media.kind == "audio") 140.dp else maxHeight
                    val width =
                        if (media.kind == "audio") (maxWidth - 32.dp).coerceAtLeast(1.dp)
                        else minOf(maxWidth, contentHeight * ratio)
                    val height = if (media.kind == "audio") contentHeight else width / ratio
                    val maxX =
                        with(density) { ((width * scale - maxWidth) / 2).toPx().coerceAtLeast(0f) }
                    val maxY =
                        with(density) {
                            ((height * scale - contentHeight) / 2).toPx().coerceAtLeast(0f)
                        }
                    val bounded =
                        Offset(offset.x.coerceIn(-maxX, maxX), offset.y.coerceIn(-maxY, maxY))
                    val center = with(density) { Offset(maxWidth.toPx() / 2, maxHeight.toPx() / 2) }
                    Box(
                        Modifier.fillMaxSize()
                            .hazeSource(haze)
                            .testTag("viewer-backdrop")
                            .clickable(onClickLabel = ui(R.string.close_media), onClick = onClose)
                            .pointerInput(media.id, center, width, height) {
                                // Observe before PlayerView so two-finger zoom also
                                // works over video controls. Single taps/seeks pass on.
                                awaitEachGesture {
                                    awaitFirstDown(
                                        requireUnconsumed = false,
                                        pass = PointerEventPass.Initial,
                                    )
                                    // Video keeps all single-finger gestures for its
                                    // own controls; use two fingers to zoom and pan.
                                    val allowPan = media.kind != "video"
                                    var transforming = false
                                    do {
                                        val event = awaitPointerEvent(PointerEventPass.Initial)
                                        if (event.changes.count { it.pressed } >= 2)
                                            transforming = true
                                        if (
                                            media.kind != "audio" &&
                                                (transforming || (scale > 1f && allowPan)) &&
                                                event.changes.any {
                                                    it.pressed &&
                                                        it.previousPressed &&
                                                        it.positionChanged()
                                                }
                                        ) {
                                            val next =
                                                (scale * event.calculateZoom()).coerceIn(1f, 5f)
                                            val pivot =
                                                event.calculateCentroid(useCurrent = false) - center
                                            offset =
                                                if (next == 1f) Offset.Zero
                                                else
                                                    (offset - pivot) * (next / scale) +
                                                        pivot +
                                                        event.calculatePan()
                                            scale = next
                                            event.changes.forEach {
                                                if (it.positionChanged()) it.consume()
                                            }
                                        }
                                    } while (event.changes.any { it.pressed })
                                }
                            }
                    ) {
                        Box(
                            Modifier.align(Alignment.Center)
                                .size(width, height)
                                .graphicsLayer {
                                    scaleX =
                                        if (media.kind in listOf("audio", "video")) 1f else scale
                                    scaleY = scaleX
                                    translationX = if (media.kind == "video") 0f else bounded.x
                                    translationY = if (media.kind == "video") 0f else bounded.y
                                }
                                .testTag("viewer-content")
                                .semantics {
                                    stateDescription =
                                        "Zoom ${String.format(Locale.ROOT,"%.2f",scale)}x"
                                    customActions =
                                        listOf(
                                            CustomAccessibilityAction(ui(R.string.zoom_in)) {
                                                scale = (scale * 1.5f).coerceAtMost(5f)
                                                true
                                            },
                                            CustomAccessibilityAction(ui(R.string.zoom_out)) {
                                                scale = (scale / 1.5f).coerceAtLeast(1f)
                                                if (scale == 1f) offset = Offset.Zero
                                                true
                                            },
                                            CustomAccessibilityAction(ui(R.string.reset_zoom)) {
                                                scale = 1f
                                                offset = Offset.Zero
                                                true
                                            },
                                        )
                                }
                                .then(
                                    if (media.kind == "video") Modifier
                                    else
                                        Modifier.clickable {
                                            /* Content taps must not dismiss the viewer. */
                                        }
                                )
                        ) {
                            if (media.kind in listOf("video", "audio")) {
                                val waveform by
                                    produceState(WaveformState(), media.id) {
                                        if (media.kind == "audio")
                                            try {
                                                value =
                                                    WaveformState(
                                                        if (media.resource != null)
                                                            model.audioWaveform(
                                                                server,
                                                                media.resource,
                                                            )
                                                        else
                                                            model.publicAudioWaveform(
                                                                server,
                                                                media.url,
                                                            )
                                                    )
                                            } catch (e: CancellationException) {
                                                throw e
                                            } catch (_: Exception) {
                                                value = WaveformState(failed = true)
                                            }
                                    }
                                PlayableMedia(
                                    server,
                                    model,
                                    media.id,
                                    media.title,
                                    true,
                                    {},
                                    media.resource,
                                    audio = media.kind == "audio",
                                    waveform = waveform,
                                    publicUrl = media.url,
                                    onVideoRatio = { ratio = it },
                                    videoScale = scale,
                                    videoPan = bounded,
                                    handoff = media.playback,
                                    fullscreen = true,
                                    onFullscreen = { onClose() },
                                )
                            } else ViewerImage(server, model, media, onRatio = { ratio = it })
                        }
                    }
                    GlassPill(
                        Modifier.align(Alignment.TopEnd).statusBarsPadding().padding(12.dp),
                        radius = 28,
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            if (saving)
                                CircularProgressIndicator(
                                    Modifier.padding(16.dp).size(24.dp),
                                    strokeWidth = 2.dp,
                                )
                            else
                                IconButton(
                                    enabled = !download.startsWith("Saved to "),
                                    onClick = {
                                        saving = true
                                        download = ""
                                        scope.launch {
                                            try {
                                                downloadViewerMedia(context, server, model, media)
                                                download =
                                                    ui(
                                                        R.string.saved_to_1_s,
                                                        downloadLocation(context),
                                                    )
                                            } catch (e: CancellationException) {
                                                throw e
                                            } catch (_: Exception) {
                                                download =
                                                    ui(R.string.couldn_t_save_this_media_try_again)
                                            } finally {
                                                saving = false
                                            }
                                        }
                                    },
                                ) {
                                    Icon(
                                        if (download.startsWith("Saved to ")) Icons.Outlined.Check
                                        else Icons.Outlined.Download,
                                        if (download.startsWith("Saved to ")) "Downloaded"
                                        else ui(R.string.download_media),
                                    )
                                }
                            media.gif?.let { GifStar(server, model, it, light = true) }
                            IconButton(onClick = onClose) {
                                Icon(
                                    Icons.Outlined.Close,
                                    if (media.kind in listOf("image", "gif"))
                                        ui(R.string.close_image)
                                    else ui(R.string.close_media),
                                )
                            }
                        }
                    }
                    if (download.isNotBlank())
                        GlassPill(
                            Modifier.align(Alignment.BottomCenter)
                                .navigationBarsPadding()
                                .padding(24.dp)
                        ) {
                            Text(
                                download,
                                modifier = Modifier.padding(16.dp),
                                style = MaterialTheme.typography.bodyMedium,
                            )
                        }
                    if (download.isBlank())
                        Text(
                            media.title,
                            Modifier.align(Alignment.BottomStart)
                                .navigationBarsPadding()
                                .padding(16.dp)
                                .widthIn(max = maxWidth - 32.dp),
                            color = Color.White.copy(alpha = .7f),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            style = MaterialTheme.typography.labelMedium,
                        )
                }
            }
        }
    }
}

@Composable
private fun BoxScope.ViewerImage(
    server: ServerState,
    model: PapoViewModel,
    media: ViewerMedia,
    onRatio: (Float) -> Unit,
) {
    val context = LocalContext.current
    var retry by remember(media.id) { mutableIntStateOf(0) }
    var failed by remember(media.id, retry) { mutableStateOf(false) }
    var loaded by remember(media.id, retry) { mutableStateOf(false) }
    val data by
        produceState<Any?>(null, media.id, retry) {
            value = null
            try {
                val gif = media.gif
                when {
                    gif != null &&
                        model.expressions.file(model.expressions.owner(server), gif.key).isFile ->
                        value = model.expressions.file(model.expressions.owner(server), gif.key)
                    media.embed != null ->
                        value = model.embedImage(server, media.embed.id, media.embed.revision)
                    gif?.embed?.isNotBlank() == true ->
                        value = model.embedImage(server, gif.embed, gif.revision)
                    else -> {
                        val source =
                            when {
                                gif?.attachment?.isNotBlank() == true ->
                                    model.mediaRequest(
                                        server.url,
                                        MediaResource.Attachment(gif.attachment, false),
                                    )
                                gif != null ->
                                    model.publicMediaRequest(
                                        server,
                                        (if (gif.url.isBlank() && gif.providerId.isNotBlank())
                                                model.gifItem(server, gif.providerId)
                                            else gif)
                                            .url,
                                        gif = true,
                                    )
                                media.url.isNotBlank() ->
                                    model.publicMediaRequest(server, media.url)
                                else ->
                                    model.mediaRequest(server.url, requireNotNull(media.resource))
                            }
                        MediaCache.get(context).images(source).collect { value = it }
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                failed = true
            }
        }
    val loader = remember(context) { MediaImages.loader(context) }
    if (data != null && !failed)
        ManagedAsyncImage(
            ImageRequest.Builder(context).data(data).build(),
            media.title,
            imageLoader = loader,
            modifier =
                Modifier.fillMaxSize().testTag("viewer-image").semantics {
                    stateDescription =
                        if (loaded) ui(R.string.loaded) else ui(R.string.loading_state)
                },
            contentScale = ContentScale.Fit,
            onError = { failed = true },
            onSuccess = { result ->
                loaded = true
                val image = result.result.image
                if (image.width > 0 && image.height > 0)
                    onRatio(image.width.toFloat() / image.height)
            },
        )
    else if (!failed)
        CircularProgressIndicator(Modifier.align(Alignment.Center).size(24.dp), strokeWidth = 2.dp)
    if (failed) TextButton(onClick = { retry++ }) { Text(ui(R.string.image_unavailable_retry)) }
}

private suspend fun downloadViewerMedia(
    context: Context,
    server: ServerState,
    model: PapoViewModel,
    media: ViewerMedia,
) {
    var temporary: File? = null
    try {
        val file =
            when {
                media.gif != null &&
                    model.expressions.file(model.expressions.owner(server), media.gif.key).isFile ->
                    model.expressions.file(model.expressions.owner(server), media.gif.key)
                media.gif?.embed?.isNotBlank() == true ->
                    withContext(Dispatchers.IO) {
                        File.createTempFile("download-image-", ".bin", context.cacheDir).also {
                            temporary = it
                            it.writeBytes(
                                model.embedImage(server, media.gif.embed, media.gif.revision)
                            )
                        }
                    }
                media.gif != null -> {
                    val gif = media.gif
                    val source =
                        if (gif.attachment.isNotBlank())
                            model.mediaRequest(
                                server.url,
                                MediaResource.Attachment(gif.attachment, false),
                            )
                        else
                            model.publicMediaRequest(
                                server,
                                (if (gif.url.isBlank() && gif.providerId.isNotBlank())
                                        model.gifItem(server, gif.providerId)
                                    else gif)
                                    .url,
                                gif = true,
                            )
                    MediaNetwork.download(context, source, media.title).also { temporary = it }
                }
                media.embed != null && media.kind in listOf("image", "gif") ->
                    withContext(Dispatchers.IO) {
                        val bytes = model.embedImage(server, media.embed.id, media.embed.revision)
                        File.createTempFile("download-image-", ".bin", context.cacheDir).also {
                            temporary = it
                            it.writeBytes(bytes)
                        }
                    }
                else -> {
                    val source =
                        if (media.url.isNotBlank()) model.publicMediaRequest(server, media.url)
                        else model.mediaRequest(server.url, requireNotNull(media.resource))
                    MediaNetwork.download(context, source, media.title).also { temporary = it }
                }
            }
        val imageMime =
            if (media.kind in listOf("image", "gif"))
                withContext(Dispatchers.IO) {
                    BitmapFactory.Options()
                        .apply {
                            inJustDecodeBounds = true
                            BitmapFactory.decodeFile(file.absolutePath, this)
                        }
                        .outMimeType
                }
            else null
        val mime =
            imageMime
                ?: (if (media.kind == "audio" && media.mime.startsWith("video/")) "audio/mp4"
                else media.mime.takeIf { it.isNotBlank() })
                ?: when (media.kind) {
                    "gif" -> "image/gif"
                    "image" -> "image/png"
                    "video" -> "video/mp4"
                    "audio" -> "audio/mp4"
                    else -> "application/octet-stream"
                }
        val extension = MimeTypeMap.getSingleton().getExtensionFromMimeType(mime) ?: "bin"
        val name =
            media.title.takeIf {
                it.substringAfterLast('.', "").lowercase() == extension ||
                    MimeTypeMap.getSingleton()
                        .getMimeTypeFromExtension(it.substringAfterLast('.', "").lowercase()) ==
                        mime
            } ?: "${media.title.ifBlank {ui(R.string.papo_media)}}.$extension"
        saveMediaDownload(context, file, name, mime)
    } finally {
        temporary?.delete()
    }
}
