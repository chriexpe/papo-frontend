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
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.FileProvider
import androidx.core.view.doOnLayout
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.PlaybackException
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.ui.PlayerView
import coil3.compose.AsyncImage
import coil3.gif.AnimatedImageDecoder
import coil3.request.ImageRequest
import io.github.chriexpe.papo.core.AttachmentView
import io.github.chriexpe.papo.core.EmbedView
import io.github.chriexpe.papo.core.MediaRequest
import io.github.chriexpe.papo.core.MediaResource
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
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
    private val base = OkHttpClient.Builder().followRedirects(false).followSslRedirects(false).connectTimeout(5,java.util.concurrent.TimeUnit.SECONDS).readTimeout(10,java.util.concurrent.TimeUnit.SECONDS).build()
    fun client(source: MediaRequest): OkHttpClient = base.newBuilder().addInterceptor { chain ->
        val request = chain.request()
        val allowed = source.origin.toHttpUrl()
        if (request.url.scheme != allowed.scheme || request.url.host != allowed.host || request.url.port != allowed.port) throw IOException("The media address changed")
        val builder = request.newBuilder()
        source.headers.forEach { (key, value) -> builder.header(key, value) }
        chain.proceed(builder.build())
    }.build()

    suspend fun download(context:Context,source:MediaRequest,name:String):File=withContext(Dispatchers.IO) {
        val directory=File(context.cacheDir,"media-exports").apply {mkdirs()}
        directory.listFiles().orEmpty().filter {System.currentTimeMillis()-it.lastModified()>24*60*60*1000L}.forEach {it.delete()}
        val extension=name.substringAfterLast('.',"bin").filter(Char::isLetterOrDigit).take(12).ifBlank {"bin"}
        val temporary=File.createTempFile("media-",".$extension",directory)
        try {MediaCache.get(context).export(source,temporary);temporary}
        catch(e:Exception) {temporary.delete();throw e}
    }

}

@Composable
internal fun MessageMedia(message: ChatMessage, server: ServerState, model: PapoViewModel, activePlayer: String?, onPlayer: (String?) -> Unit, onActions: ()->Unit) {
    GifChoice.marker(message.content)?.let { ProviderGif(server,model,it,onActions) }
    message.attachments.forEach { attachment ->
        Attachment(attachment, server, model, activePlayer, onPlayer,onActions)
    }
    message.embeds.forEach { embed ->
        EmbedCard(embed, server, model, activePlayer, onPlayer,onActions)
    }
    message.nativeLinks().distinctBy {it.url}.filter {link -> message.embeds.none {it.url==link.url}}.forEach {link ->
        val kind=directMediaKind(link.url) ?: return@forEach
        val title=Uri.parse(link.url).lastPathSegment?.takeIf {it.isNotBlank()} ?: "Media"
        val gif=if(kind=="gif") GifChoice("url:${link.url}",title,url=link.url) else null
        val media=ViewerMedia("${server.url}/${server.me}/url/${link.url}",title,kind,gif=gif,url=link.url)
        if(kind=="audio") {
            AudioPreview(server,model,media,activePlayer,onPlayer)
        } else ReservedMedia(0,0,tag="direct-media-${message.id}") {
            if(kind=="video") VideoPreview(server,model,media,onPlayer=onPlayer,onActions=onActions)
            else MediaImage(server,model,title,publicUrl=link.url,gif=gif,onActions=onActions)
        }
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
private fun Attachment(a: AttachmentView, server: ServerState, model: PapoViewModel, activePlayer: String?, onPlayer: (String?) -> Unit,onActions: ()->Unit) {
    var revealed by rememberSaveable(server.url, server.me, a.id) { mutableStateOf(false) }
    val blocked = a.status == "blocked"
    val sensitive = a.status == "sensitive" && !revealed
    if (blocked || sensitive) {
        ReservedMedia(0, 0, tag = "attachment-${a.id}") {
            Column(Modifier.align(Alignment.Center).padding(20.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Icon(if (sensitive) Icons.Outlined.VisibilityOff else Icons.Outlined.Shield, null)
                Text(if (blocked) "Media unavailable" else "Sensitive media", modifier = Modifier.padding(top = 8.dp))
                if (sensitive) TextButton(onClick = { revealed = true }) { Text("Show media") }
            }
        }
        return
    }
    when (a.kind) {
        "image" -> ReservedMedia(0, 0, fallback = 4f / 3f, tag = "attachment-${a.id}") {
            MediaImage(server, model, "Photo: ${a.name}", resource = MediaResource.Attachment(a.id, false), gif = if(a.mime.substringBefore(';').equals("image/gif",true) || a.name.endsWith(".gif",true)) GifChoice("attachment:${a.id}",a.name,attachment=a.id) else null,onActions=onActions)
        }
        "video", "audio" -> {
            val id = "${server.url}/${server.me}/attachment/${a.id}"
            val media=ViewerMedia(id,a.name,a.kind,a.mime,resource=MediaResource.Attachment(a.id,false))
            if (a.kind == "audio") {
                AudioPreview(server,model,media,activePlayer,onPlayer,tag="audio-attachment-${a.id}")
            } else ReservedMedia(0, 0, tag = "attachment-${a.id}") {
                VideoPreview(server,model,media,poster=if(a.thumbnail) MediaResource.Attachment(a.id,true) else null,onPlayer=onPlayer,onActions=onActions)
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
private fun EmbedCard(e: EmbedView, server: ServerState, model: PapoViewModel, activePlayer: String?, onPlayer: (String?) -> Unit,onActions: ()->Unit) {
    if(e.nativeMedia) {
        ReservedMedia(e.width,e.height,fallback=if(e.kind=="video") 16f/9f else 4f/3f,tag="embed-${e.id}") {
            EmbedVisual(e,server,model,onPlayer,onActions)
        }
        return
    }
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
            "image", "gif" -> ReservedMedia(e.width, e.height, fallback = 4f / 3f, tag = "embed-${e.id}") { EmbedVisual(e,server,model,onPlayer,onActions) }
            "video" -> ReservedMedia(e.width, e.height, tag = "embed-${e.id}") {
                EmbedVisual(e,server,model,onPlayer,onActions)
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
private fun BoxScope.EmbedVisual(e:EmbedView,server:ServerState,model:PapoViewModel,onPlayer:(String?)->Unit,onActions:()->Unit) {
    if(e.kind=="video") VideoPreview(server,model,ViewerMedia("${server.url}/${server.me}/embed/${e.id}",e.title.ifBlank {"Video"},"video",e.mime,e.width,e.height,resource=MediaResource.EmbedVideo(e.id)),embedPoster=e.takeIf {it.hasImage},onPlayer=onPlayer,onActions=onActions)
    else MediaImage(server,model,e.title.ifBlank {"Image preview"},embed=e,gif=if(e.kind=="gif") GifChoice("embed:${e.id}",e.title.ifBlank {"GIF"},url=e.url,embed=e.id,revision=e.revision,width=e.width,height=e.height) else null,onActions=onActions)
}

@Composable
private fun BoxScope.VideoPreview(server:ServerState,model:PapoViewModel,media:ViewerMedia,poster:MediaResource?=null,embedPoster:EmbedView?=null,onPlayer:(String?)->Unit,onActions:()->Unit) {
    val viewer=LocalMediaViewer.current
    val active=LocalInlinePlayer.current==media.id
    Box(Modifier.fillMaxSize().combinedClickable(onClick={onPlayer(media.id)},onLongClick=onActions,onLongClickLabel="Message actions")) {
        PlayableMedia(server,model,media.id,media.title,active,{onPlayer(media.id)},media.resource,poster=poster,embedPoster=embedPoster,publicUrl=media.url,onFullscreen={viewer(media.copy(playback=it))})
    }
}

@Composable
private fun AudioPreview(server:ServerState,model:PapoViewModel,media:ViewerMedia,activePlayer:String?,onPlayer:(String?)->Unit,tag:String="") {
    val viewer=LocalMediaViewer.current
    val waveform by produceState(WaveformState(),server.url,server.me,media.id) {
        try {value=WaveformState(if(media.resource!=null) model.audioWaveform(server,media.resource) else model.publicAudioWaveform(media.url))}
        catch(e:CancellationException) {throw e} catch(_:Exception) {value=WaveformState(failed=true)}
    }
    Box(Modifier.fillMaxWidth().height(104.dp).testTag(tag).clip(RoundedCornerShape(14.dp)).background(MaterialTheme.colorScheme.surfaceContainer)) {
        PlayableMedia(server,model,media.id,media.title,activePlayer==media.id,{onPlayer(media.id)},media.resource,audio=true,waveform=waveform,publicUrl=media.url,onExpand={onPlayer(null);viewer(media)},onFullscreen={viewer(media.copy(playback=it))})
    }
}

@Composable
private fun BoxScope.MediaImage(server: ServerState, model: PapoViewModel, description: String, resource: MediaResource? = null, embed: EmbedView? = null, author: Boolean = false, viewer: Boolean = true, gif: GifChoice? = null,publicUrl:String="",onActions:(()->Unit)?=null) {
    val context = LocalContext.current
    var retry by remember { mutableStateOf(0) }
    val openViewer=LocalMediaViewer.current
    val media=ViewerMedia("${server.url}/${server.me}/image/${embed?.id ?: resource ?: publicUrl}",description.removePrefix("Photo: "),if(gif!=null) "gif" else "image",embed?.mime.orEmpty(),embed?.width ?: 0,embed?.height ?: 0,resource,embed,gif,publicUrl)
    var decodeError by remember(retry, embed?.revision) { mutableStateOf(false) }
    var decoded by remember(retry, embed?.revision) { mutableStateOf(false) }
    val data by produceState<ByteArray?>(null,server.url,server.me,resource,embed?.id,embed?.revision,author,retry,publicUrl) {
        value=null
        try {
            if(embed!=null) value=model.embedImage(server,embed.id,embed.revision,author)
            else {
                val source=if(publicUrl.isNotBlank()) publicMediaSource(publicUrl) else model.mediaRequest(server.url,requireNotNull(resource))
                MediaCache.get(context).images(source).collect {value=it}
            }
        } catch(e:CancellationException) {throw e} catch(_:Exception) {decodeError=true}
    }
    val loader=remember(context) {MediaImages.loader(context)}
    val request=remember(data) {ImageRequest.Builder(context).data(data).build()}
    if (data != null && !decodeError) ManagedAsyncImage(request, description, imageLoader = loader,
        modifier = Modifier.fillMaxSize().semantics { stateDescription = if (decoded) "Loaded" else "Loading" }.then(if (viewer) Modifier.combinedClickable(onClick={openViewer(media)},onLongClick=onActions,onLongClickLabel="Message actions") else Modifier), contentScale = ContentScale.Fit,
        onSuccess = { decoded = true },
        onError = { decodeError = true },retentionKey="image/${embed?.id}/${embed?.revision}/$resource/$publicUrl/$author")
    else if (!decodeError) CircularProgressIndicator(Modifier.align(Alignment.Center).size(24.dp), strokeWidth = 2.dp)
    if (decodeError) Column(Modifier.align(Alignment.Center).padding(12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Icon(Icons.Outlined.BrokenImage, null)
        Text("Image unavailable", style = MaterialTheme.typography.bodySmall)
        TextButton(onClick = { decodeError = false; retry++ }) { Text("Retry") }
    }
    if (gif!=null) GifStar(server,model,gif,Modifier.align(Alignment.TopEnd))
}

@Composable
private fun BoxScope.PlayButton(title: String, play: () -> Unit) {
    FilledIconButton(onClick = play, modifier = Modifier.align(Alignment.Center).size(56.dp)) { Icon(Icons.Outlined.PlayArrow, "Play $title", Modifier.size(30.dp)) }
}

@androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)
internal class PlaybackHandle(val player:ExoPlayer,val source:MediaRequest,initialPlayIntent:Boolean=true,private val save:(PlaybackBookmark)->Unit={}) {
    private val owners=mutableMapOf<Any,Boolean>()
    private var visibilityPaused=true
    private var changingVisibility=false
    private var playIntent=initialPlayIntent
    private val intentListener=object:Player.Listener {
        override fun onPlayWhenReadyChanged(ready:Boolean,reason:Int) {if(!changingVisibility && !visibilityPaused) playIntent=ready}
    }
    init {player.addListener(intentListener)}
    private var view:PlayerView?=null
    private data class Host(val container:android.widget.FrameLayout,val fullscreen:Boolean,val toggle:(()->Unit)?,val scale:Float,val pan:androidx.compose.ui.geometry.Offset)
    private var inlineHost:Host?=null
    private var fullscreenHost:Host?=null
    fun acquire(owner:Any) {owners[owner]=false;applyVisibility()}
    fun setVisible(owner:Any,visible:Boolean) {if(owner in owners) {owners[owner]=visible;applyVisibility()}}
    private fun applyVisibility() {
        val visible=owners.values.any {it}
        if(visible && visibilityPaused) {
            visibilityPaused=false;changingVisibility=true
            try {player.playWhenReady=playIntent} finally {changingVisibility=false}
        } else if(!visible && !visibilityPaused) {
            playIntent=player.playWhenReady;visibilityPaused=true;changingVisibility=true
            try {player.pause()} finally {changingVisibility=false}
        }
    }
    fun release(owner:Any) {if(owners.remove(owner)==null) return
        if(owners.isEmpty()) {
        save(PlaybackBookmark(player.currentPosition.coerceAtLeast(0),if(visibilityPaused) playIntent else player.playWhenReady))
        view?.let {native ->(native.parent as? android.view.ViewGroup)?.removeView(native);native.player=null}
        view=null;inlineHost=null;fullscreenHost=null;player.removeListener(intentListener);player.release()
    } else applyVisibility()}
    fun bind(container:android.widget.FrameLayout,fullscreen:Boolean,toggle:(()->Unit)?,scale:Float,pan:androidx.compose.ui.geometry.Offset) {
        val host=Host(container,fullscreen,toggle,scale,pan)
        if(fullscreen) fullscreenHost=host else inlineHost=host
        if(view==null) view=(android.view.LayoutInflater.from(container.context).inflate(R.layout.compose_player,null) as PlayerView).apply {this.player=this@PlaybackHandle.player}
        attach()
    }
    fun unbind(container:android.widget.FrameLayout) {
        if(inlineHost?.container===container) inlineHost=null
        if(fullscreenHost?.container===container) fullscreenHost=null
        attach()
    }
    private fun attach() {
        val native=view ?: return
        val host=fullscreenHost ?: inlineHost
        if(host==null) {(native.parent as? android.view.ViewGroup)?.removeView(native);return}
        val moved=native.parent!==host.container
        if(moved) {
            (native.parent as? android.view.ViewGroup)?.removeView(native)
            host.container.addView(native,android.widget.FrameLayout.LayoutParams(android.view.ViewGroup.LayoutParams.MATCH_PARENT,android.view.ViewGroup.LayoutParams.MATCH_PARENT))
        }
        // Media3 invokes the listener during icon synchronization too. Only
        // changing modes is a user action; initialization must not exit again.
        native.setFullscreenButtonClickListener(if(host.toggle!=null) PlayerView.FullscreenButtonClickListener {requested ->if(requested!=host.fullscreen) host.toggle()} else null)
        native.setFullscreenButtonState(host.fullscreen)
        native.videoSurfaceView?.doOnLayout {surface -> (surface as? android.view.TextureView)?.setTransform(android.graphics.Matrix().apply {
            postScale(host.scale,host.scale,surface.width/2f,surface.height/2f);postTranslate(host.pan.x,host.pan.y)
        })}
        if(moved) native.showController()
    }
}

@androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)
@Composable
internal fun BoxScope.PlayableMedia(server: ServerState, model: PapoViewModel, id: String, title: String, active: Boolean, activate: () -> Unit, resource: MediaResource?, audio: Boolean = false, poster: MediaResource? = null, embedPoster: EmbedView? = null, waveform: WaveformState = WaveformState(),publicUrl:String="",onVideoRatio:(Float)->Unit={},onExpand:(()->Unit)?=null,videoScale:Float=1f,videoPan:androidx.compose.ui.geometry.Offset=androidx.compose.ui.geometry.Offset.Zero,handoff:PlaybackHandle?=null,fullscreen:Boolean=false,onFullscreen:((PlaybackHandle)->Unit)?=null) {
    val activity=rememberMediaActivity()
    Box(Modifier.fillMaxSize().then(activity.modifier)) {
        PlayableMediaContent(server,model,id,title,active && (activity.retained || handoff!=null),activate,resource,audio,poster,embedPoster,waveform,publicUrl,onVideoRatio,onExpand,videoScale,videoPan,handoff,fullscreen,onFullscreen,activity.visible)
    }
}

@androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)
@Composable
private fun BoxScope.PlayableMediaContent(server: ServerState, model: PapoViewModel, id: String, title: String, active: Boolean, activate: () -> Unit, resource: MediaResource?, audio: Boolean, poster: MediaResource?, embedPoster: EmbedView?, waveform: WaveformState,publicUrl:String,onVideoRatio:(Float)->Unit,onExpand:(()->Unit)?,videoScale:Float,videoPan:androidx.compose.ui.geometry.Offset,handoff:PlaybackHandle?,fullscreen:Boolean,onFullscreen:((PlaybackHandle)->Unit)?,mediaVisible:Boolean) {
    var initialSeek by remember(id) { mutableStateOf<Float?>(null) }
    if (!active) {
        if (audio) Row(Modifier.fillMaxSize().padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            FilledIconButton(onClick = activate, modifier = Modifier.size(48.dp)) { Icon(Icons.Outlined.PlayArrow, "Play $title") }
            Column(Modifier.weight(1f)) {
                Text(title, modifier=Modifier.then(if(onExpand!=null) Modifier.clickable(onClickLabel="Open $title",onClick=onExpand) else Modifier),maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium)
                AudioWaveformSeek(waveform, 0f, waveform.data != null, id, { initialSeek = it; activate() }, Modifier.fillMaxWidth())
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    Text("0:00", style = MaterialTheme.typography.labelSmall)
                    Text(waveform.data?.durationMs?.let(::audioTime) ?: "—:—", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        } else {
            if (poster != null || embedPoster != null) MediaImage(server, model, title, resource = poster, embed = embedPoster, viewer = false)
            PlayButton(title, activate)
        }
        return
    }
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var error by remember(id) { mutableStateOf("") }
    var ready by remember(id) { mutableStateOf(handoff?.player?.playbackState in listOf(Player.STATE_READY,Player.STATE_ENDED)) }
    var retry by remember(id) { mutableStateOf(0) }
    val registry=LocalMediaVisibilityRegistry.current
    val row=LocalMediaRowId.current
    val retainedPlayback=remember(id,retry) {if(retry==0) registry?.player(id) else null}
    val existing=handoff ?: retainedPlayback
    val source by produceState<MediaRequest?>(null, id, retry) {
        try { value = existing?.source ?: if(publicUrl.isNotBlank()) publicMediaSource(publicUrl) else model.mediaRequest(server.url, requireNotNull(resource)) }
        catch (e: CancellationException) { throw e }
        catch (e: Exception) { error = "Media unavailable. Tap to retry." }
    }
    val bookmarks=LocalPlaybackBookmarks.current
    val playback = remember(source,existing) { existing ?: source?.let { request ->
        val bookmark=bookmarks?.get(id)
        val factory = MediaCache.get(context).factory(request)
        val player=ExoPlayer.Builder(context).setMediaSourceFactory(DefaultMediaSourceFactory(factory)).build().apply {
            setMediaItem(MediaItem.fromUri(request.url))
            bookmark?.let {seekTo(it.position)}
            playWhenReady = false; prepare()
        }
        PlaybackHandle(player,request,bookmark?.playWhenVisible ?: true) {bookmarks?.save(id,it)}
    } }
    val player=playback?.player
    val owner=remember {Any()}
    DisposableEffect(player, lifecycle) {
        playback?.let {registry?.retainPlayer(id,row,it)}
        playback?.acquire(owner)
        val listener = object : Player.Listener {
            override fun onVideoSizeChanged(videoSize: androidx.media3.common.VideoSize) {if(videoSize.width>0 && videoSize.height>0) onVideoRatio(videoSize.width.toFloat()*videoSize.pixelWidthHeightRatio/videoSize.height)}
            override fun onPlayerError(e: PlaybackException) { error = "Playback failed. Tap to retry." }
            override fun onPlaybackStateChanged(state: Int) {
                ready = state == Player.STATE_READY || state == Player.STATE_ENDED
                if (state == Player.STATE_READY && initialSeek != null && player != null && player.duration > 0) {
                    val fraction = initialSeek!!; initialSeek = null
                    player.seekTo((fraction * player.duration).toLong())
                }
            }
        }
        player?.addListener(listener)
        player?.videoSize?.let {size ->if(size.width>0 && size.height>0) onVideoRatio(size.width.toFloat()*size.pixelWidthHeightRatio/size.height)}
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_PAUSE) player?.pause() }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer); player?.removeListener(listener); playback?.release(owner) }
    }
    DisposableEffect(playback,registry,row) {
        registry?.register(owner,row) {playback?.setVisible(owner,false)}
        onDispose {registry?.unregister(owner)}
    }
    SideEffect {playback?.setVisible(owner,mediaVisible && registry?.isVisible(row)!=false)}
    if (player == null) CircularProgressIndicator(Modifier.align(Alignment.Center).size(24.dp), strokeWidth = 2.dp)
    else Box(Modifier.fillMaxSize().testTag(if(!fullscreen && !audio && LocalViewedMedia.current?.id==id) "inline-player-$id" else "player-$id").semantics { stateDescription = if (ready) "Ready" else "Loading" }) {
        if (audio) AudioControls(player, title, waveform, id,if(onExpand!=null) {{if(onFullscreen!=null) onFullscreen(requireNotNull(playback)) else onExpand()}} else null)
        else AndroidView(factory={android.widget.FrameLayout(it)},onRelease={playback?.unbind(it)},update={host ->
            playback?.bind(host,fullscreen,if(onFullscreen!=null) {{onFullscreen(playback)}} else null,videoScale,videoPan)
        },modifier=Modifier.fillMaxSize())
    }
    if (error.isNotBlank()) Column(Modifier.align(Alignment.Center).background(MaterialTheme.colorScheme.surface).padding(12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Text(error, style = MaterialTheme.typography.bodySmall)
        TextButton(onClick = { error = ""; if (player == null) retry++ else { player.prepare(); player.play() } }) { Text("Retry") }
    }
}

/** Audio has its own controls and never creates a video surface. */
@Composable
private fun AudioControls(player: Player, title: String, waveform: WaveformState, id: String,onExpand:(()->Unit)?=null) {
    var playing by remember(player) { mutableStateOf(player.isPlaying) }
    var position by remember(player) { mutableLongStateOf(0) }
    var duration by remember(player) { mutableLongStateOf(waveform.data?.durationMs ?: 0) }
    DisposableEffect(player) {
        val listener = object : Player.Listener {
            override fun onIsPlayingChanged(isPlaying: Boolean) { playing = isPlaying }
        }
        player.addListener(listener)
        onDispose { player.removeListener(listener) }
    }
    LaunchedEffect(player) {
        while (true) {
            position = player.currentPosition.coerceAtLeast(0)
            duration = player.duration.takeIf { it > 0 } ?: waveform.data?.durationMs ?: 0
            delay(250)
        }
    }
    Row(Modifier.fillMaxSize().testTag("audio-controls").padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        FilledIconButton(onClick = {
            if (player.isPlaying) player.pause()
            else { if (player.playbackState == Player.STATE_ENDED) player.seekTo(0); player.play() }
        }, modifier = Modifier.size(48.dp)) {
            Icon(if (playing) Icons.Outlined.Pause else Icons.Outlined.PlayArrow, if (playing) "Pause $title" else "Play $title")
        }
        Column(Modifier.weight(1f)) {
            Text(title, modifier=Modifier.then(if(onExpand!=null) Modifier.clickable(onClickLabel="Open $title",onClick=onExpand) else Modifier),maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis, style = MaterialTheme.typography.labelMedium)
            AudioWaveformSeek(waveform, if (duration > 0) position.toFloat() / duration else 0f, duration > 0, id,
                onSeek = { fraction -> val target = (fraction * duration).toLong(); player.seekTo(target); position = target }, modifier = Modifier.fillMaxWidth())
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text(audioTime(position), style = MaterialTheme.typography.labelSmall)
                Text(audioTime(duration), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

private fun audioTime(milliseconds: Long): String {
    val seconds = milliseconds.coerceAtLeast(0) / 1000
    return "%d:%02d".format(seconds / 60, seconds % 60)
}

@Suppress("SetJavaScriptEnabled")
@Composable
private fun ProviderPlayer(url: String, server: ServerState, onPlayer: (String?) -> Unit) {
    val activity=rememberMediaActivity()
    Box(Modifier.fillMaxSize().then(activity.modifier)) {
        if(activity.retained) ProviderPlayerContent(url,server,onPlayer,activity.visible)
    }
}

@Suppress("SetJavaScriptEnabled")
@Composable
private fun ProviderPlayerContent(url: String, server: ServerState, onPlayer: (String?) -> Unit,visible:Boolean) {
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
    LaunchedEffect(webView,visible) {
        if(visible) webView.onResume()
        else {
            webView.evaluateJavascript("document.querySelectorAll('video,audio').forEach(function(media){media.pause();});",null)
            webView.onPause()
        }
    }
    AndroidView(factory = { webView }, modifier = Modifier.fillMaxSize())
}

internal fun isPublicLink(raw: String): Boolean = runCatching { Uri.parse(raw).let { it.scheme in listOf("http", "https") && !it.host.isNullOrBlank() && it.userInfo.isNullOrEmpty() } }.getOrDefault(false)
