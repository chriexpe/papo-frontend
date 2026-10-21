package io.github.chriexpe.papo.compose

import android.graphics.drawable.Animatable
import androidx.compose.foundation.Image
import androidx.compose.foundation.lazy.LazyListLayoutInfo
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.SemanticsPropertyKey
import androidx.compose.ui.semantics.semantics
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import coil3.ImageLoader
import coil3.asDrawable
import coil3.compose.AsyncImage
import coil3.compose.AsyncImagePainter

internal val LocalMediaOverlayBlocking=compositionLocalOf {false}
internal val LocalMediaViewport=compositionLocalOf<Rect?> {null}
internal val LocalTimelineMediaVisible=compositionLocalOf {true}
internal val LocalPlaybackBookmarks=compositionLocalOf<PlaybackBookmarks?> {null}
internal val LocalMediaVisibilityRegistry=compositionLocalOf<MediaVisibilityRegistry?> {null}
internal val LocalMediaRowId=compositionLocalOf<String?> {null}

/** Cached lazy compositions can be inactive. Enforce their pause from the
 * conversation itself, rather than waiting for an offscreen recomposition. */
internal class MediaVisibilityRegistry {
    private var rows=emptySet<String>()
    private var foreground=true
    private data class Entry(val row:String,val stop:()->Unit)
    private val entries=mutableMapOf<Any,Entry>()
    fun isVisible(row:String?)=row==null || (foreground && row in rows)
    fun register(owner:Any,row:String?,stop:()->Unit) {
        if(row==null) return
        entries[owner]=Entry(row,stop)
        if(!isVisible(row)) stop()
    }
    fun unregister(owner:Any) {entries.remove(owner)}
    private data class RetainedPlayer(val row:String,val handle:PlaybackHandle,val owner:Any=Any())
    private data class RetainedImage(val row:String,val state:AsyncImagePainter.State.Success)
    private val players=mutableMapOf<String,RetainedPlayer>()
    private val images=mutableMapOf<String,RetainedImage>()
    private val sizes=mutableMapOf<String,Int>()
    fun player(id:String)=players[id]?.handle
    fun retainPlayer(id:String,row:String?,handle:PlaybackHandle) {
        if(row==null || players[id]?.handle===handle) return
        players.remove(id)?.let {it.handle.release(it.owner)}
        val entry=RetainedPlayer(row,handle);players[id]=entry;handle.acquire(entry.owner)
        while(players.size>3) players.remove(players.keys.first())?.let {it.handle.release(it.owner)}
    }
    fun image(key:String)=images[key]?.state
    fun retainImage(key:String,row:String?,state:AsyncImagePainter.State.Success) {
        if(row!=null) {images[key]=RetainedImage(row,state);while(images.size>96) images.remove(images.keys.first())}
    }
    fun trim(layout:LazyListLayoutInfo,order:List<String>) {
        val placed=layout.visibleItemsInfo.filter {it.key is String && it.key in order}
        placed.forEach {sizes[it.key as String]=it.size}
        val anchor=placed.firstOrNull() ?: return
        val anchorIndex=order.indexOf(anchor.key)
        val height=(layout.viewportSize.height-layout.beforeContentPadding-layout.afterContentPadding).coerceAtLeast(1)
        val fallback=if(sizes.isEmpty()) height else sizes.values.average().toInt().coerceAtLeast(1)
        fun far(row:String):Boolean {
            val index=order.indexOf(row)
            if(index<0) return true
            val top=anchor.offset + if(index>=anchorIndex) (anchorIndex until index).sumOf {sizes[order[it]] ?: fallback}
                else -(index until anchorIndex).sumOf {sizes[order[it]] ?: fallback}
            val bottom=top+(sizes[row] ?: fallback)
            return maxOf(-bottom,top-height)>height*2
        }
        players.keys.filter {far(players.getValue(it).row)}.forEach {key ->players.remove(key)?.let {it.handle.release(it.owner)}}
        images.keys.filter {far(images.getValue(it).row)}.forEach {images.remove(it)}
        // Channel timelines are bounded separately by core; don't retain old row measurements indefinitely.
        sizes.keys.retainAll(order.toSet())
    }
    fun close() {players.values.forEach {it.handle.release(it.owner)};players.clear();images.clear();entries.clear()}
    fun update(visibleRows:Set<String>,enabled:Boolean) {
        if(rows==visibleRows && foreground==enabled) return
        rows=visibleRows;foreground=enabled
        entries.values.toList().forEach {if(!isVisible(it.row)) it.stop()}
    }
}

internal data class PlaybackBookmark(val position:Long,val playWhenVisible:Boolean)
internal class PlaybackBookmarks {
    private val entries=LinkedHashMap<String,PlaybackBookmark>()
    operator fun get(id:String)=entries[id]
    fun save(id:String,bookmark:PlaybackBookmark) {
        entries.remove(id);entries[id]=bookmark
        while(entries.size>128) entries.remove(entries.keys.first())
    }
}

/** Bounds alone are insufficient for retained lazy rows: unplaced items can
 * keep their last coordinates. The row's placement gate closes that gap. */
internal class MediaActivity(private val permitted:()->Boolean,private val viewport:()->Rect?) {
    private var bounds by mutableStateOf<Rect?>(null)
    var retained by mutableStateOf(false)
        private set
    val visible by derivedStateOf {
        permitted() && bounds?.let {rect -> !rect.isEmpty && (viewport()?.overlaps(rect) ?: true)}==true
    }
    val modifier=Modifier.onGloballyPositioned {bounds=it.boundsInWindow()}
    fun retainIfVisible() {if(visible) retained=true}
}

@Composable
internal fun rememberMediaActivity():MediaActivity {
    val lifecycle=LocalLifecycleOwner.current.lifecycle
    val state by lifecycle.currentStateFlow.collectAsState()
    val allowed by rememberUpdatedState(LocalTimelineMediaVisible.current && state.isAtLeast(Lifecycle.State.RESUMED))
    val viewport by rememberUpdatedState(LocalMediaViewport.current)
    val activity=remember {MediaActivity({allowed},{viewport})}
    activity.visible // Observe visibility without recomposing for every scroll pixel.
    SideEffect {activity.retainIfVisible()}
    return activity
}

// These custom semantics expose actual decoder state to device regressions;
// they don't add an announcement or implementation detail to TalkBack labels.
internal val MediaAnimationRunning=SemanticsPropertyKey<()->Boolean>("MediaAnimationRunning")

/** Coil owns decoding/drawing; visibility owns the animation clock. Decoded
 * images are per occurrence so stopping one sticker cannot stop another. */
@Composable
internal fun ManagedAsyncImage(model:Any?,description:String?,imageLoader:ImageLoader,modifier:Modifier=Modifier,
    contentScale:ContentScale=ContentScale.Fit,onSuccess:(AsyncImagePainter.State.Success)->Unit={},onError:(AsyncImagePainter.State.Error)->Unit={},retentionKey:String="") {
    val context=LocalContext.current
    val activity=rememberMediaActivity()
    val registry=LocalMediaVisibilityRegistry.current
    val row=LocalMediaRowId.current
    val owner=remember {Any()}
    val occurrence=currentCompositeKeyHash
    val imageKey=if(row!=null && retentionKey.isNotBlank()) "$row/$retentionKey/$occurrence" else ""
    val retained=remember(imageKey,registry) {if(imageKey.isNotBlank()) registry?.image(imageKey) else null}
    var animation by remember(imageKey) {mutableStateOf(retained?.result?.image?.asDrawable(context.resources) as? Animatable)}
    val visible=activity.visible
    DisposableEffect(animation,registry,row) {
        val old=animation
        registry?.register(owner,row) {old?.stop()}
        onDispose {registry?.unregister(owner);old?.stop()}
    }
    LaunchedEffect(retained) {retained?.let(onSuccess)}
    SideEffect {animation?.let {if(visible && registry?.isVisible(row)!=false) {if(!it.isRunning) it.start()} else it.stop()}}
    Box(modifier.then(activity.modifier).semantics {this[MediaAnimationRunning]={animation?.isRunning==true}}) {
        if(activity.retained && retained!=null) Image(retained.painter,description,Modifier.fillMaxSize(),contentScale=contentScale)
        else if(activity.retained) AsyncImage(model,description,imageLoader=imageLoader,modifier=Modifier.fillMaxSize(),contentScale=contentScale,
            onSuccess={result -> animation=result.result.image.asDrawable(context.resources) as? Animatable;if(imageKey.isNotBlank()) registry?.retainImage(imageKey,row,result);onSuccess(result)},onError={animation=null;onError(it)})
    }
}
