package io.github.chriexpe.papo.compose

import android.content.Context
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import coil3.ImageLoader
import coil3.compose.AsyncImage
import coil3.gif.AnimatedImageDecoder
import coil3.network.okhttp.OkHttpNetworkFetcherFactory
import io.github.chriexpe.papo.core.MediaRequest
import kotlinx.coroutines.CancellationException
import java.io.File
import java.security.MessageDigest
import okhttp3.OkHttpClient

internal fun publicGifSource(url: String): MediaRequest {
    if(url.startsWith("file:///android_asset/demo/")) return MediaRequest(url,emptyMap(),"demo",url)
    val uri=Uri.parse(url)
    require(uri.scheme=="https" && !uri.host.isNullOrBlank() && uri.userInfo==null) { "Invalid GIF address" }
    val origin=uri.buildUpon().path("").query(null).fragment(null).build().toString().trimEnd('/')
    return MediaRequest(url,emptyMap(),origin,"public/$url")
}
internal object ExpressionImages {
    @Volatile private var shared: ImageLoader? = null
    @Synchronized fun loader(context: Context): ImageLoader = shared ?: ImageLoader.Builder(context.applicationContext).components {
        add(AnimatedImageDecoder.Factory())
        add(OkHttpNetworkFetcherFactory(callFactory={ OkHttpClient.Builder().followRedirects(false).followSslRedirects(false).build() }))
    }.build().also {shared=it}
}

@Composable
internal fun GifStar(server: ServerState, model: PapoViewModel, gif: GifChoice, modifier: Modifier=Modifier, light: Boolean=false) {
    val states by model.expressions.states.collectAsState()
    val owner=model.expressions.owner(server)
    val state=states[owner] ?: model.expressions.state(owner)
    val selected=state.favorites.any {it.key==gif.key}
    IconButton(onClick={model.toggleGif(server,gif)},modifier=modifier.padding(4.dp).size(48.dp).semantics {stateDescription=if(selected) "Favorite" else "Not a favorite"}) {
        Icon(if(selected) Icons.Outlined.Star else Icons.Outlined.StarBorder,if(selected) "Remove GIF from favorites" else "Favorite GIF",tint=if(selected) MaterialTheme.colorScheme.primary else if(light) Color.White else MaterialTheme.colorScheme.onSurface)
    }
}

@Composable
internal fun BoxScope.GifArtwork(server: ServerState, model: PapoViewModel, gif: GifChoice, preview: Boolean=false, onClick: ()->Unit={}) {
    val context=LocalContext.current
    var error by remember(gif.key) { mutableStateOf("") }
    var retry by remember {mutableIntStateOf(0)}
    val data by produceState<Any?>(null,gif.key,gif.path,retry) {
        error=""
        try {
            val saved=model.expressions.file(model.expressions.owner(server),gif.key)
            value=when {
                saved.isFile -> saved
                gif.attachment.isNotBlank() || gif.embed.isNotBlank() -> model.gifCopy(server,gif)
                gif.providerId.isNotBlank() && gif.url.isBlank() -> model.gifItem(server,gif.providerId).let { publicGifSource(if(preview) it.preview else it.url).url }
                else -> publicGifSource(if(preview) gif.preview else gif.url).url
            }
        } catch(e:CancellationException) {throw e}
        catch(e:Exception) {error=e.message ?: "GIF unavailable"}
    }
    val loader=remember(context) {ExpressionImages.loader(context)}
    if(data!=null && error.isBlank()) AsyncImage(data,gif.title,imageLoader=loader,modifier=Modifier.fillMaxSize().clickable(onClick=onClick),contentScale=ContentScale.Fit,onError={error="GIF unavailable"})
    else if(error.isBlank()) CircularProgressIndicator(Modifier.align(Alignment.Center).size(24.dp),strokeWidth=2.dp)
    if(error.isNotBlank()) Column(Modifier.align(Alignment.Center).padding(12.dp),horizontalAlignment=Alignment.CenterHorizontally) {
        Text(error,style=MaterialTheme.typography.bodySmall,maxLines=3)
        TextButton(onClick={retry++}) {Text("Retry")}
    }
}

@Composable
internal fun ProviderGif(server: ServerState, model: PapoViewModel, id: String) {
    var expanded by remember(id) { mutableStateOf(false) }
    var gif by remember(id) {mutableStateOf(GifChoice("giphy:$id","GIF",providerId=id))}
    LaunchedEffect(id) { try {gif=model.gifItem(server,id)} catch(e:CancellationException) {throw e} catch(_:Exception) { /* Artwork exposes a retry and the provider error. */ } }
    ReservedMedia(gif.width,gif.height,tag="provider-gif-$id") {
        GifArtwork(server,model,gif,onClick={expanded=true})
        GifStar(server,model,gif,Modifier.align(Alignment.TopEnd))
    }
    if(expanded) Dialog(onDismissRequest={expanded=false},properties=DialogProperties(usePlatformDefaultWidth=false,decorFitsSystemWindows=false)) {
        Box(Modifier.fillMaxSize().background(Color.Black)) {
            GifArtwork(server,model,gif)
            Row(Modifier.align(Alignment.TopEnd).statusBarsPadding().padding(12.dp),verticalAlignment=Alignment.CenterVertically) {
                GifStar(server,model,gif,light=true)
                IconButton(onClick={expanded=false}) {Icon(Icons.Outlined.Close,"Close image",tint=Color.White)}
            }
        }
    }
}
