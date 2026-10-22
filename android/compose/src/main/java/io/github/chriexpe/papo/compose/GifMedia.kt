@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)

package io.github.chriexpe.papo.compose

import android.content.Context
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Star
import androidx.compose.material.icons.outlined.StarBorder
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import io.github.chriexpe.papo.core.MediaRequest
import kotlinx.coroutines.CancellationException
import java.io.File
import java.security.MessageDigest

internal fun publicGifSource(url: String): MediaRequest {
    if(url.startsWith("file:///android_asset/demo/")) return MediaRequest(url=url,headers=emptyMap(),origin="demo",cacheKey=url)
    val uri=Uri.parse(url)
    require(uri.scheme in listOf("http","https") && !uri.host.isNullOrBlank() && uri.userInfo==null) { ui(R.string.invalid_gif_address) }
    val origin=uri.buildUpon().path("").query(null).fragment(null).build().toString().trimEnd('/')
    return MediaRequest(url=url,headers=emptyMap(),origin=origin,cacheKey="public/$url")
}

@Composable
internal fun GifStar(server: ServerState, model: PapoViewModel, gif: GifChoice, modifier: Modifier=Modifier, light: Boolean=false) {
    val states by model.expressions.states.collectAsState()
    val owner=model.expressions.owner(server)
    val state=states[owner] ?: model.expressions.state(owner)
    val selected=state.favorites.any {it.key==gif.key}
    IconButton(onClick={model.toggleGif(server,gif)},modifier=modifier.padding(4.dp).size(48.dp).semantics {stateDescription=if(selected) uiLabel("Favorite") else ui(R.string.not_favorite)}) {
        Icon(if(selected) Icons.Outlined.Star else Icons.Outlined.StarBorder,if(selected) ui(R.string.remove_gif_from_favorites) else ui(R.string.favorite_gif),tint=if(selected) MaterialTheme.colorScheme.primary else if(light) Color.White else MaterialTheme.colorScheme.onSurface)
    }
}

@Composable
internal fun BoxScope.GifArtwork(server: ServerState, model: PapoViewModel, gif: GifChoice, preview: Boolean=false,onLongClick:(()->Unit)?=null, onClick: ()->Unit={}) {
    val context=LocalContext.current
    var error by remember(gif.key) { mutableStateOf("") }
    var retry by remember {mutableIntStateOf(0)}
    val data by produceState<Any?>(null,gif.key,gif.path,retry) {
        error=""
        try {
            val saved=model.expressions.file(model.expressions.owner(server),gif.key)
            when {
                saved.isFile -> value=saved
                gif.attachment.isNotBlank() || gif.embed.isNotBlank() -> {
                    if(gif.embed.isNotBlank()) value=model.embedImage(server,gif.embed,gif.revision)
                    else MediaCache.get(context).images(model.mediaRequest(server.url,io.github.chriexpe.papo.core.MediaResource.Attachment(gif.attachment,false))).collect {value=it}
                }
                else -> {
                    val resolved=if(gif.providerId.isNotBlank() && gif.url.isBlank()) model.gifItem(server,gif.providerId) else gif
                    MediaCache.get(context).images(publicGifSource(if(preview) resolved.preview else resolved.url)).collect {value=it}
                }
            }
        } catch(e:CancellationException) {throw e}
        catch(e:Exception) {error=e.message ?: ui(R.string.gif_unavailable)}
    }
    val loader=remember(context) {MediaImages.loader(context)}
    if(data!=null && error.isBlank()) ManagedAsyncImage(data,gif.title,imageLoader=loader,modifier=Modifier.fillMaxSize().combinedClickable(onClick=onClick,onLongClick=onLongClick,onLongClickLabel=ui(R.string.message_actions)),contentScale=ContentScale.Fit,onError={error=ui(R.string.gif_unavailable)},retentionKey="gif/${gif.key}/$preview")
    else if(error.isBlank()) CircularProgressIndicator(Modifier.align(Alignment.Center).size(24.dp),strokeWidth=2.dp)
    if(error.isNotBlank()) Column(Modifier.align(Alignment.Center).padding(12.dp),horizontalAlignment=Alignment.CenterHorizontally) {
        Text(error,style=MaterialTheme.typography.bodySmall,maxLines=3)
        TextButton(onClick={retry++}) {Text(ui(R.string.retry))}
    }
}

@Composable
internal fun ProviderGif(server: ServerState, model: PapoViewModel, id: String,onActions:()->Unit) {
    val viewer=LocalMediaViewer.current
    var gif by remember(id) {mutableStateOf(GifChoice("giphy:$id","GIF",providerId=id))}
    LaunchedEffect(id) { try {gif=model.gifItem(server,id)} catch(e:CancellationException) {throw e} catch(_:Exception) { /* Artwork exposes a retry and the provider error. */ } }
    ReservedMedia(gif.width,gif.height,tag="provider-gif-$id") {
        GifArtwork(server,model,gif,onLongClick=onActions,onClick={viewer(ViewerMedia("giphy:$id",gif.title,"gif","image/gif",gif.width,gif.height,gif=gif))})
        GifStar(server,model,gif,Modifier.align(Alignment.TopEnd))
    }
}
