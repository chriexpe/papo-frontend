@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
package io.github.chriexpe.papo.compose

import android.util.Base64
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil3.ImageLoader
import coil3.compose.AsyncImage
import coil3.gif.AnimatedImageDecoder
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONArray

@Composable
internal fun EmojiGlyph(server: ServerState, emoji: EmojiChoice, size: Int = 28, model: PapoViewModel) {
    if (!emoji.custom) Text(emoji.value, fontSize=size.sp, maxLines=1)
    else {
        val entry=server.emojis.firstOrNull { it.id==emoji.value }
        val context=LocalContext.current
        val bytes by produceState<ByteArray?>(null,server.url,server.me,emoji.value,entry?.revision) {
            try { value=model.emojiImage(server,emoji.value) }
            catch(e:CancellationException) {throw e}
            catch(_:Exception) {value=null}
        }
        val loader=remember(context) { MediaImages.loader(context) }
        if(bytes!=null) ManagedAsyncImage(bytes,":${entry?.name ?: emoji.name}:",imageLoader=loader,modifier=Modifier.size(size.dp),contentScale=ContentScale.Fit,retentionKey="emoji/${emoji.value}/${entry?.revision}/$size")
        else Text(":${entry?.name ?: emoji.name}:",style=MaterialTheme.typography.labelSmall,maxLines=1)
    }
}

@Composable
internal fun EmojiPicker(server: ServerState, model: PapoViewModel, onDismiss: ()->Unit, onPick: (EmojiChoice)->Unit) {
    var query by remember { mutableStateOf("") }
    val context=LocalContext.current
    val standard=remember { val a=JSONArray(context.assets.open("emoji.json").bufferedReader().use { it.readText() }); (0 until a.length()).map { a.getJSONObject(it).let { e -> e.getString("glyph") to e.getString("keywords") } } }
    val custom=remember(server.emojis,query) { server.emojis.filter { it.name.contains(query.trim(),true) } }
    val unicode=remember(standard,query) {
        val typed=messageEmoji(query,server).filter {!it.custom}.takeIf {it.size==1 && it[0].value==query.trim()}.orEmpty().map {it.value to it.value}
        (typed+standard.filter { (g,k)->query.isBlank() || k.contains(query.trim(),true) || g==query.trim() }).distinctBy {it.first}
    }
    ModalBottomSheet(onDismissRequest=onDismiss) {
        Column(Modifier.fillMaxWidth().fillMaxHeight(.74f).imePadding().padding(horizontal=16.dp).testTag("emoji-picker")) {
            Text("Emoji & stickers",style=MaterialTheme.typography.titleLarge,modifier=Modifier.padding(bottom=12.dp))
            OutlinedTextField(query,{query=it},label={Text("Find an emoji or sticker")},singleLine=true,modifier=Modifier.fillMaxWidth(),leadingIcon={Icon(Icons.Outlined.Search,null)})
            LazyVerticalGrid(GridCells.Adaptive(56.dp),modifier=Modifier.weight(1f).padding(top=12.dp),contentPadding=PaddingValues(bottom=24.dp)) {
                item(span={GridItemSpan(maxLineSpan)}) { Text(server.name,style=MaterialTheme.typography.titleSmall,color=MaterialTheme.colorScheme.primary,modifier=Modifier.padding(vertical=12.dp)) }
                if(custom.isEmpty()) item(span={GridItemSpan(maxLineSpan)}) { Text(if(server.emojis.isEmpty()) "This server hasn’t added emoji yet." else "No matching server emoji.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant,modifier=Modifier.padding(bottom=12.dp)) }
                items(custom,key={"c/${it.id}"}) { e ->
                    val choice=EmojiChoice(e.id,true,e.name)
                    Column(Modifier.clip(RoundedCornerShape(16.dp)).clickable(onClickLabel="Insert :${e.name}:") { onPick(choice) }.padding(vertical=8.dp).semantics { contentDescription="Sticker :${e.name}:" },horizontalAlignment=Alignment.CenterHorizontally) {
                        EmojiGlyph(server,choice,40,model)
                        Text(e.name,style=MaterialTheme.typography.labelSmall,maxLines=1,modifier=Modifier.padding(top=4.dp))
                    }
                }
                item(span={GridItemSpan(maxLineSpan)}) { Text("Emoji",style=MaterialTheme.typography.titleSmall,color=MaterialTheme.colorScheme.onSurfaceVariant,modifier=Modifier.padding(vertical=12.dp)) }
                items(unicode,key={"u/${it.first}"}) { (glyph,_) -> Box(Modifier.size(52.dp).clip(RoundedCornerShape(16.dp)).clickable { onPick(EmojiChoice(glyph)) }.semantics { contentDescription="Emoji $glyph" },contentAlignment=Alignment.Center) { Text(glyph,fontSize=28.sp) } }
                if(custom.isEmpty() && unicode.isEmpty()) item(span={GridItemSpan(maxLineSpan)}) { Text("No emoji found. Try another name.",modifier=Modifier.padding(vertical=24.dp)) }
            }
        }
    }
}

@Composable
internal fun GifPicker(server: ServerState, model: PapoViewModel, onDismiss: ()->Unit, onPick: (GifChoice)->Unit) {
    var tab by remember { mutableIntStateOf(2) }
    var query by remember { mutableStateOf("") }
    var results by remember { mutableStateOf<List<GifChoice>>(emptyList()) }
    var more by remember { mutableStateOf(false) }
    var offset by remember { mutableIntStateOf(0) }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var retry by remember { mutableIntStateOf(0) }
    var requestEpoch by remember { mutableIntStateOf(0) }
    val all by model.expressions.states.collectAsState()
    val owner=model.expressions.owner(server)
    val library=all[owner] ?: model.expressions.state(owner)
    val displayed=if(query.isNotBlank() || tab==2) results else (if(tab==0) library.favorites else library.recent)
    LaunchedEffect(query,tab,retry) {
        requestEpoch++
        error=""; loading=false; more=false
        if(tab!=2 && query.isBlank()) return@LaunchedEffect
        results=emptyList(); offset=0; loading=true
        try { if(query.isNotBlank()) delay(300); val page=model.gifPage(server,query,0); results=page.items.map(GifChoice::from); offset=24; more=page.more }
        catch(e:CancellationException) { throw e }
        catch(e:Exception) { error=e.message ?: "Couldn’t load GIFs" }
        finally { loading=false }
    }
    val scope=rememberCoroutineScope()
    ModalBottomSheet(onDismissRequest=onDismiss) {
        Column(Modifier.fillMaxWidth().fillMaxHeight(.78f).imePadding().padding(horizontal=16.dp).testTag("gif-picker")) {
            Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically) {
                Text("GIFs",style=MaterialTheme.typography.titleLarge,modifier=Modifier.weight(1f))
                Text("Powered by GIPHY",style=MaterialTheme.typography.labelSmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
            }
            OutlinedTextField(query,{query=it},label={Text("Search GIFs")},singleLine=true,modifier=Modifier.fillMaxWidth().padding(vertical=12.dp),leadingIcon={Icon(Icons.Outlined.Search,null)})
            Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(8.dp)) { listOf("Favorites","Recent","Trending").forEachIndexed { i,label -> FilterChip(query.isBlank() && tab==i,{tab=i; query=""},label={Text(label)}) } }
            if(error.isNotBlank()) Column(Modifier.padding(vertical=16.dp)) { Text(error,style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant); TextButton(onClick={retry++}) { Text("Retry") } }
            if(loading && displayed.isEmpty()) Box(Modifier.fillMaxWidth().padding(24.dp),contentAlignment=Alignment.Center) { CircularProgressIndicator(Modifier.size(28.dp)) }
            if(displayed.isEmpty() && !loading && error.isBlank()) Text(when(tab) { 0->"Star a GIF in chat to keep it here."; 1->"GIFs you send will appear here."; else->"No GIFs found. Try another search." },modifier=Modifier.padding(vertical=24.dp),color=MaterialTheme.colorScheme.onSurfaceVariant)
            LazyVerticalGrid(GridCells.Fixed(2),modifier=Modifier.weight(1f),horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(8.dp),contentPadding=PaddingValues(bottom=24.dp)) {
                items(displayed,key={it.key}) { gif -> Box(Modifier.fillMaxWidth().height(132.dp).clip(RoundedCornerShape(16.dp)).background(MaterialTheme.colorScheme.surfaceContainer)) {
                    GifArtwork(server,model,gif,preview=true,onClick={onPick(gif)})
                    GifStar(server,model,gif,Modifier.align(Alignment.TopEnd))
                } }
                if(more && error.isBlank() && (tab==2 || query.isNotBlank())) item(span={GridItemSpan(maxLineSpan)}) {
                    TextButton(onClick={scope.launch {
                        val epoch=requestEpoch
                        loading=true
                        try { val page=model.gifPage(server,query,offset); if(epoch==requestEpoch) {results=(results+page.items.map(GifChoice::from)).distinctBy { it.key }; offset+=24; more=page.more} }
                        catch(e:CancellationException) { throw e }
                        catch(e:Exception) {if(epoch==requestEpoch) error=e.message ?: "Couldn’t load more GIFs"}
                        finally {if(epoch==requestEpoch) loading=false}
                    }},enabled=!loading,modifier=Modifier.fillMaxWidth()) { Text(if(loading) "Loading…" else "More GIFs") }
                }
            }
        }
    }
}

@Composable
internal fun ReactionPeople(server: ServerState, model: PapoViewModel, messageId: String, initial: EmojiChoice, onDismiss: ()->Unit) {
    val message=server.messages.firstOrNull {it.id==messageId}
    val openProfile=LocalOpenProfile.current
    var selected by remember(messageId) { mutableStateOf(initial) }
    val details=server.reactionDetails.firstOrNull {it.messageId==messageId}
    val loading=messageId in server.reactionLoading
    LaunchedEffect(messageId) { model.command(io.github.chriexpe.papo.core.Action.ReactionDetails(server.selected,messageId,false),server.url) }
    ModalBottomSheet(onDismissRequest=onDismiss) {
        Column(Modifier.fillMaxWidth().heightIn(max=520.dp).navigationBarsPadding().padding(20.dp).testTag("reaction-people")) {
            Text("Reactions",style=MaterialTheme.typography.titleLarge)
            LazyRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) { items(message?.reactions.orEmpty(),key={"${it.custom}/${it.emoji}"}) { r ->
                val e=EmojiChoice(r.emoji,r.custom,server.emojis.firstOrNull {it.id==r.emoji}?.name ?: r.emoji)
                FilterChip(selected.key==e.key,{selected=e},label={Row(verticalAlignment=Alignment.CenterVertically) {EmojiGlyph(server,e,24,model); Text(" ${r.count}")}})
            } }
            message?.let { m -> val mine=m.reactions.any {it.emoji==selected.value && it.custom==selected.custom && it.mine}
                TextButton(onClick={model.react(server,m,selected); model.command(io.github.chriexpe.papo.core.Action.ReactionDetails(server.selected,messageId,false),server.url)}) { Text(if(mine) "Remove my reaction" else "Add my reaction") }
            }
            val users=details?.groups?.firstOrNull {it.emoji==selected.value && it.custom==selected.custom}?.users.orEmpty().distinctBy {it.id}
            LazyColumn(Modifier.weight(1f,fill=false)) { items(users,key={it.id}) { person -> Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).clickable {onDismiss();openProfile(person.id)}.padding(vertical=12.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) { UserAvatar(person.id,person.name); Text(person.name,color=userNameColor(person.id)); if(person.id==server.me) Text("You",color=MaterialTheme.colorScheme.onSurfaceVariant,style=MaterialTheme.typography.labelSmall) } } }
            if(loading) CircularProgressIndicator(Modifier.padding(16.dp).size(24.dp),strokeWidth=2.dp)
            else if(details==null) TextButton(onClick={model.command(io.github.chriexpe.papo.core.Action.ReactionDetails(server.selected,messageId,false),server.url)}) { Text("Retry loading reactions") }
            else if(details.more) TextButton(onClick={model.command(io.github.chriexpe.papo.core.Action.ReactionDetails(server.selected,messageId,true),server.url)}) { Text("Load more people") }
            else if(users.isEmpty()) Text("No reactions yet.",modifier=Modifier.padding(vertical=16.dp),color=MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}
