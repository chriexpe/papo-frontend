@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)

package io.github.chriexpe.papo.compose

import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.FileProvider
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.PlaybackException
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.ui.PlayerView
import coil3.ImageLoader
import coil3.compose.AsyncImage
import coil3.gif.AnimatedImageDecoder
import coil3.network.okhttp.OkHttpNetworkFetcherFactory
import coil3.request.ImageRequest
import io.github.chriexpe.papo.core.AttachmentView
import io.github.chriexpe.papo.core.EmbedView
import io.github.chriexpe.papo.core.MediaRequest
import io.github.chriexpe.papo.core.MediaResource
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.launch
import kotlinx.coroutines.CancellationException
import okhttp3.OkHttpClient
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.Request
import java.io.File
import java.io.IOException

/** Shared network pool, never a global authenticated cookie jar. Each request
 * is pinned to its server origin; provider iframes get no Papo headers. */
internal object MediaNetwork {
    private val base = OkHttpClient.Builder().followRedirects(false).followSslRedirects(false).build()
    fun client(source: MediaRequest): OkHttpClient = base.newBuilder().addInterceptor { chain ->
        val request = chain.request()
        val allowed = source.origin.toHttpUrl()
        if (request.url.scheme != allowed.scheme || request.url.host != allowed.host || request.url.port != allowed.port) throw IOException("The media address changed")
        val builder = request.newBuilder()
        source.headers.forEach { (key, value) -> builder.header(key, value) }
        chain.proceed(builder.build())
    }.build()

    suspend fun download(context: Context, source: MediaRequest, name: String): File = withContext(Dispatchers.IO) {
        val directory = File(context.cacheDir, "media").apply { mkdirs() }
        val extension = name.substringAfterLast('.', "bin").filter(Char::isLetterOrDigit).take(12).ifBlank { "bin" }
        val key = java.security.MessageDigest.getInstance("SHA-256").digest(source.cacheKey.toByteArray()).joinToString("") { "%02x".format(it) }
        val file = File(directory, "$key.$extension")
        if (file.isFile) return@withContext file
        val partial = File.createTempFile("download-", ".part", directory)
        try {
            partial.outputStream().use { output ->
                if (source.url.startsWith("file:///android_asset/")) context.assets.open(source.url.removePrefix("file:///android_asset/")).use { it.copyTo(output) }
                else client(source).newCall(Request.Builder().url(source.url).build()).execute().use { response ->
                    if (!response.isSuccessful) throw IOException("Couldn’t download this file (${response.code})")
                    response.body?.byteStream()?.use { it.copyTo(output) } ?: throw IOException("The file is empty")
                }
            }
            if (!partial.renameTo(file)) throw IOException("Couldn’t save this file")
            file
        } finally { partial.delete() }
    }
}

@Composable
internal fun MessageMedia(message: ChatMessage, server: ServerState, model: PapoViewModel, activePlayer: String?, onPlayer: (String?) -> Unit) {
    message.attachments.forEach { attachment ->
        Attachment(attachment, server, model, activePlayer, onPlayer)
    }
    message.embeds.forEach { embed ->
        EmbedCard(embed, server, model, activePlayer, onPlayer)
    }
}

/** The metadata ratio is the layout source of truth. Decode and player events
 * never change it, including on error, retry, or switching to playback. */
@Composable
internal fun ReservedMedia(width: Int, height: Int, fallback: Float = 16f / 9f, tag: String = "", content: @Composable BoxScope.() -> Unit) {
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val ratio = if (width > 0 && height > 0) width.toFloat() / height else fallback
        val available = maxWidth.coerceAtMost(480.dp)
        val frameHeight = (available / ratio).coerceIn(72.dp, 360.dp)
        val frameWidth = (frameHeight * ratio).coerceAtMost(available)
        Box(Modifier.width(frameWidth).height(frameHeight).clip(RoundedCornerShape(14.dp))
            .background(MaterialTheme.colorScheme.surfaceContainer).testTag(tag), content = content)
    }
}

@Composable
private fun Attachment(a: AttachmentView, server: ServerState, model: PapoViewModel, activePlayer: String?, onPlayer: (String?) -> Unit) {
    var revealed by rememberSaveable(server.url, server.me, a.id) { mutableStateOf(false) }
    val waiting = a.status in listOf("pending", "processing")
    val blocked = a.status in listOf("blocked", "failed")
    val sensitive = a.status == "sensitive" && !revealed
    if (waiting || blocked || sensitive) {
        ReservedMedia(0, 0, tag = "attachment-${a.id}") {
            Column(Modifier.align(Alignment.Center).padding(20.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Icon(if (sensitive) Icons.Outlined.VisibilityOff else Icons.Outlined.Shield, null)
                Text(when { waiting -> "Media is being checked"; blocked -> "Media unavailable"; else -> "Sensitive media" }, modifier = Modifier.padding(top = 8.dp))
                if (sensitive) TextButton(onClick = { revealed = true }) { Text("Show media") }
            }
        }
        return
    }
    when (a.kind) {
        "image" -> ReservedMedia(0, 0, fallback = 4f / 3f, tag = "attachment-${a.id}") {
            MediaImage(server, model, "Photo: ${a.name}", resource = MediaResource.Attachment(a.id, false))
        }
        "video", "audio" -> {
            val id = "${server.url}/${server.me}/attachment/${a.id}"
            if (a.kind == "audio") Box(Modifier.fillMaxWidth().height(104.dp).clip(RoundedCornerShape(14.dp)).background(MaterialTheme.colorScheme.surfaceContainer)) {
                PlayableMedia(server, model, id, a.name, activePlayer == id, { onPlayer(id) }, MediaResource.Attachment(a.id, false), audio = true)
            } else ReservedMedia(0, 0, tag = "attachment-${a.id}") {
                PlayableMedia(server, model, id, a.name, activePlayer == id, { onPlayer(id) }, MediaResource.Attachment(a.id, false),
                    poster = if (a.thumbnail) MediaResource.Attachment(a.id, true) else null)
            }
        }
        else -> FileAttachment(a, server, model)
    }
}

@Composable
private fun FileAttachment(a: AttachmentView, server: ServerState, model: PapoViewModel) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(MaterialTheme.colorScheme.surfaceContainer).padding(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Outlined.InsertDriveFile, null, Modifier.padding(end = 12.dp))
            Column(Modifier.weight(1f)) {
                Text(a.name, style = MaterialTheme.typography.bodyMedium)
                Text(if (a.sizeBytes > 1024 * 1024) "${a.sizeBytes / (1024 * 1024)} MB" else "${a.sizeBytes / 1024} KB", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (loading) CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
            else IconButton(onClick = {
                loading = true; error = ""
                scope.launch {
                    try {
                        val request = model.mediaRequest(server.url, MediaResource.Attachment(a.id, false))
                        val file = MediaNetwork.download(context, request, a.name)
                        val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file, a.name)
                        val intent = Intent(Intent.ACTION_VIEW).setDataAndType(uri, a.mime).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                        context.startActivity(Intent.createChooser(intent, "Open ${a.name}"))
                    } catch (e: CancellationException) { throw e }
                    catch (e: Exception) { error = "Couldn’t open this file. Try again." }
                    finally { loading = false }
                }
            }) { Icon(Icons.Outlined.OpenInNew, "Open ${a.name}") }
        }
        if (error.isNotBlank()) Text(error, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
    }
}

@Composable
private fun EmbedCard(e: EmbedView, server: ServerState, model: PapoViewModel, activePlayer: String?, onPlayer: (String?) -> Unit) {
    val uriHandler = LocalUriHandler.current
    val publicUrl = remember(e.url) { e.url.takeIf(::isPublicLink) }
    val accent = remember(e.color) { runCatching { Color(android.graphics.Color.parseColor(e.color)) }.getOrNull() }
    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(MaterialTheme.colorScheme.surface)
        .padding(start = 3.dp).then(if (accent != null) Modifier.background(accent.copy(alpha = .08f)) else Modifier)) {
        if (e.kind != "gif") Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            if (e.provider.isNotBlank()) Text(e.provider, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (e.author.isNotBlank()) Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (e.hasAuthorImage) Box(Modifier.size(24.dp).clip(RoundedCornerShape(12.dp))) { MediaImage(server, model, "${e.author} avatar", embed = e, author = true, viewer = false) }
                Text(e.author, style = MaterialTheme.typography.labelMedium)
            }
            if (e.title.isNotBlank()) Text(e.title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.then(if (publicUrl != null) Modifier.clickable { runCatching { uriHandler.openUri(publicUrl) } } else Modifier))
            if (e.description.isNotBlank()) Text(e.description, style = MaterialTheme.typography.bodyMedium)
            if (e.fields.isNotEmpty()) FlowRow(horizontalArrangement = Arrangement.spacedBy(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                e.fields.forEach { field -> Column(if (field.inline) Modifier.widthIn(min = 100.dp, max = 200.dp) else Modifier.fillMaxWidth()) {
                    Text(field.name, style = MaterialTheme.typography.labelLarge)
                    Text(field.value, style = MaterialTheme.typography.bodyMedium)
                } }
            }
        }
        when (e.kind) {
            "image", "gif" -> ReservedMedia(e.width, e.height, fallback = 4f / 3f, tag = "embed-${e.id}") { MediaImage(server, model, e.title.ifBlank { "Image preview" }, embed = e) }
            "video" -> ReservedMedia(e.width, e.height, tag = "embed-${e.id}") {
                val id = "${server.url}/${server.me}/embed/${e.id}"
                PlayableMedia(server, model, id, e.title.ifBlank { "Video" }, activePlayer == id, { onPlayer(id) }, MediaResource.EmbedVideo(e.id), embedPoster = e.takeIf { it.hasImage })
            }
            "iframe" -> ReservedMedia(e.width, e.height, tag = "embed-${e.id}") {
                val id = "${server.url}/${server.me}/iframe/${e.id}"
                if (activePlayer == id) ProviderPlayer(e.iframeUrl, server, onPlayer)
                else {
                    if (e.hasImage) MediaImage(server, model, e.title, embed = e, viewer = false)
                    PlayButton(e.title.ifBlank { "YouTube video" }) { onPlayer(id) }
                }
            }
        }
        if (e.footer.isNotBlank()) Text(e.footer, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(14.dp))
        if (e.kind == "gif" && publicUrl != null) TextButton(onClick = { runCatching { uriHandler.openUri(publicUrl) } }) { Text(e.provider.ifBlank { "Open original" }) }
    }
}

@Composable
private fun BoxScope.MediaImage(server: ServerState, model: PapoViewModel, description: String, resource: MediaResource? = null, embed: EmbedView? = null, author: Boolean = false, viewer: Boolean = true) {
    val context = LocalContext.current
    var retry by remember { mutableStateOf(0) }
    var expanded by remember { mutableStateOf(false) }
    var decodeError by remember(retry, embed?.revision) { mutableStateOf(false) }
    var decoded by remember(retry, embed?.revision) { mutableStateOf(false) }
    val data by produceState<Pair<Any, MediaRequest?>?>(null, server.url, server.me, resource, embed?.id, embed?.revision, author, retry) {
        value = null
        try {
            value = if (embed != null) model.embedImage(server, embed.id, embed.revision, author) to null
            else model.mediaRequest(server.url, requireNotNull(resource)).let { it.url to it }
        } catch (e: CancellationException) { throw e }
        catch (e: Exception) { decodeError = true }
    }
    val source = data?.second
    val loader = remember(source) { ImageLoader.Builder(context).components {
        add(AnimatedImageDecoder.Factory())
        if (source != null) add(OkHttpNetworkFetcherFactory(callFactory = { MediaNetwork.client(source) }))
    }.memoryCache(null).build() }
    DisposableEffect(loader) { onDispose { loader.shutdown() } }
    val request = remember(data, server.url, server.me, embed?.revision) { ImageRequest.Builder(context)
        .data(data?.first).memoryCacheKey(source?.cacheKey ?: "${server.url}/${server.me}/${embed?.id}/${embed?.revision}/$author").build() }
    if (data != null && !decodeError) AsyncImage(request, description, imageLoader = loader,
        modifier = Modifier.fillMaxSize().semantics { stateDescription = if (decoded) "Loaded" else "Loading" }.then(if (viewer) Modifier.clickable { expanded = true } else Modifier), contentScale = ContentScale.Fit,
        onSuccess = { decoded = true },
        onError = { decodeError = true })
    else if (!decodeError) CircularProgressIndicator(Modifier.align(Alignment.Center).size(24.dp), strokeWidth = 2.dp)
    if (decodeError) Column(Modifier.align(Alignment.Center).padding(12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Icon(Icons.Outlined.BrokenImage, null)
        Text("Image unavailable", style = MaterialTheme.typography.bodySmall)
        TextButton(onClick = { decodeError = false; retry++ }) { Text("Retry") }
    }
    if (expanded) ImageViewer(request, loader, description) { expanded = false }
}

@Composable
private fun ImageViewer(request: ImageRequest, loader: ImageLoader, title: String, onClose: () -> Unit) {
    var scale by remember { mutableStateOf(1f) }
    var x by remember { mutableStateOf(0f) }; var y by remember { mutableStateOf(0f) }
    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        Box(Modifier.fillMaxSize().background(Color.Black)) {
            AsyncImage(request, title, loader, Modifier.fillMaxSize().pointerInput(Unit) {
                detectTransformGestures { _, pan, zoom, _ -> scale = (scale * zoom).coerceIn(1f, 5f); x = if (scale == 1f) 0f else x + pan.x; y = if (scale == 1f) 0f else y + pan.y }
            }.graphicsLayer { scaleX = scale; scaleY = scale; translationX = x; translationY = y }, contentScale = ContentScale.Fit)
            Row(Modifier.align(Alignment.TopCenter).statusBarsPadding().fillMaxWidth().padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(title, color = Color.White, maxLines = 1, modifier = Modifier.weight(1f).padding(12.dp))
                IconButton(onClick = onClose) { Icon(Icons.Outlined.Close, "Close image", tint = Color.White) }
            }
        }
    }
}

@Composable
private fun BoxScope.PlayButton(title: String, play: () -> Unit) {
    FilledIconButton(onClick = play, modifier = Modifier.align(Alignment.Center).size(56.dp)) { Icon(Icons.Outlined.PlayArrow, "Play $title", Modifier.size(30.dp)) }
}

@androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)
@Composable
private fun BoxScope.PlayableMedia(server: ServerState, model: PapoViewModel, id: String, title: String, active: Boolean, activate: () -> Unit, resource: MediaResource, audio: Boolean = false, poster: MediaResource? = null, embedPoster: EmbedView? = null) {
    if (!active) {
        if (poster != null || embedPoster != null) MediaImage(server, model, title, resource = poster, embed = embedPoster, viewer = false)
        if (audio) Text(title, Modifier.align(Alignment.CenterStart).padding(start = 16.dp, end = 72.dp), style = MaterialTheme.typography.bodyMedium)
        PlayButton(title, activate)
        return
    }
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var error by remember(id) { mutableStateOf("") }
    var ready by remember(id) { mutableStateOf(false) }
    var retry by remember(id) { mutableStateOf(0) }
    val source by produceState<MediaRequest?>(null, id, retry) {
        try { value = model.mediaRequest(server.url, resource) }
        catch (e: CancellationException) { throw e }
        catch (e: Exception) { error = "Media unavailable. Tap to retry." }
    }
    val player = remember(source) { source?.let { request ->
        val http = OkHttpDataSource.Factory(MediaNetwork.client(request))
        val factory = DefaultDataSource.Factory(context, http)
        ExoPlayer.Builder(context).setMediaSourceFactory(DefaultMediaSourceFactory(factory)).build().apply {
            setMediaItem(MediaItem.fromUri(request.url)); playWhenReady = true; prepare()
        }
    } }
    DisposableEffect(player, lifecycle) {
        val listener = object : Player.Listener {
            override fun onPlayerError(e: PlaybackException) { error = "Playback failed. Tap to retry." }
            override fun onPlaybackStateChanged(state: Int) { ready = state == Player.STATE_READY || state == Player.STATE_ENDED }
        }
        player?.addListener(listener)
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_PAUSE) player?.pause() }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer); player?.removeListener(listener); player?.release() }
    }
    if (player == null) CircularProgressIndicator(Modifier.align(Alignment.Center).size(24.dp), strokeWidth = 2.dp)
    else AndroidView(factory = { android.view.LayoutInflater.from(it).inflate(R.layout.compose_player, null) as PlayerView }, update = { it.player = player }, modifier = Modifier.fillMaxSize().testTag("player-$id").semantics { stateDescription = if (ready) "Ready" else "Loading" })
    if (error.isNotBlank()) Column(Modifier.align(Alignment.Center).background(MaterialTheme.colorScheme.surface).padding(12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Text(error, style = MaterialTheme.typography.bodySmall)
        TextButton(onClick = { error = ""; if (player == null) retry++ else { player.prepare(); player.play() } }) { Text("Retry") }
    }
}

@Suppress("SetJavaScriptEnabled")
@Composable
private fun ProviderPlayer(url: String, server: ServerState, onPlayer: (String?) -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val webView = remember(url) { WebView(context).apply {
        settings.javaScriptEnabled = true; settings.domStorageEnabled = true
        settings.allowFileAccess = false; settings.allowContentAccess = false
        settings.mixedContentMode = android.webkit.WebSettings.MIXED_CONTENT_NEVER_ALLOW
        settings.mediaPlaybackRequiresUserGesture = true
        webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView?, request: WebResourceRequest?): Boolean {
                val target = request?.url ?: return true
                if (!request.isForMainFrame) return false
                if (target.scheme == "https" && target.host in listOf("www.youtube.com", "www.youtube-nocookie.com")) return false
                if (isPublicLink(target.toString())) runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, target)) }
                return true
            }
        }
        // Never install the Papo session cookie in the provider browser.
        val referer = if (server.isDemo) "https://github.com/chriexpe/papo-frontend/" else server.url
        loadUrl(url, mapOf("Referer" to referer))
    } }
    DisposableEffect(webView, lifecycle) {
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_PAUSE) { webView.onPause(); onPlayer(null) } }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer); webView.stopLoading(); webView.onPause(); webView.destroy() }
    }
    AndroidView(factory = { webView }, modifier = Modifier.fillMaxSize())
}

internal fun isPublicLink(raw: String): Boolean = runCatching { Uri.parse(raw).let { it.scheme in listOf("http", "https") && !it.host.isNullOrBlank() && it.userInfo.isNullOrEmpty() } }.getOrDefault(false)
