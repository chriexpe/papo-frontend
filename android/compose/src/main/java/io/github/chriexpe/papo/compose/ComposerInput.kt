package io.github.chriexpe.papo.compose

import android.graphics.ImageDecoder
import android.graphics.drawable.Animatable
import android.graphics.drawable.Drawable
import android.os.Handler
import android.os.Looper
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import io.github.chriexpe.papo.core.EmojiView
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.nio.ByteBuffer
import kotlin.math.roundToInt

internal data class ComposerSticker(val range:IntRange,val id:String,val name:String)
internal fun composerStickers(text:String,emojis:List<EmojiView>):List<ComposerSticker> {
    val byName=emojis.associateBy {it.name}
    return Regex(":([A-Za-z0-9_+\\-]+):").findAll(text).mapNotNull {m -> byName[m.groupValues[1]]?.let {ComposerSticker(m.range,it.id,it.name)} }.toList()
}
internal data class StickerPosition(val offset:Int,val id:String,val name:String)
internal class ComposerProjection(draft:ComposerDraft, stickers:List<ComposerSticker>, loaded:Set<String>, color:Color) : VisualTransformation {
    private val forward=IntArray(draft.text.length+1)
    private val reverse=mutableListOf(0)
    val positions=mutableListOf<StickerPosition>()
    val annotated:AnnotatedString
    init {
        val starts=stickers.filter {it.id in loaded}.associateBy {it.range.first}
        val plain=StringBuilder()
        var index=0
        while(index<draft.text.length) {
            val sticker=starts[index]
            val offset=plain.length
            forward[index]=offset
            if(sticker!=null) {
                plain.append('\u3000') // one em-wide slot; the real drawable uses this glyph's measured bounds
                positions+=StickerPosition(offset,sticker.id,sticker.name)
                for(i in index+1..sticker.range.last+1) forward[i]=offset+1
                index=sticker.range.last+1
            } else { plain.append(draft.text[index]); index++; forward[index]=offset+1 }
            reverse+=index
        }
        annotated=buildAnnotatedString {
            append(plain.toString())
            positions.forEach {addStyle(SpanStyle(fontSize=28.sp),it.offset,it.offset+1)}
            draft.mentions.filter {it.valid(draft.text)}.forEach {addStyle(SpanStyle(color=color,fontWeight=FontWeight.Bold),forward[it.start],forward[it.end])}
        }
    }
    val mapping=object:OffsetMapping {
        override fun originalToTransformed(offset:Int)=forward[offset.coerceIn(forward.indices)]
        override fun transformedToOriginal(offset:Int)=reverse[offset.coerceIn(reverse.indices)]
    }
    override fun filter(text:AnnotatedString)=TransformedText(annotated,mapping)
}
private data class LoadedComposerStickers(val owner:String,val size:Int,val images:Map<String,Drawable>)

/** Keep Compose's native IME, selection and cursor. The visual transformation
 * reserves a measured glyph slot, and animated drawables paint in those exact
 * text-layout bounds, so wrapping and scrolling share the editor's layout. */
@Composable
internal fun ComposerInput(draft:ComposerDraft, server:ServerState, model:PapoViewModel, placeholder:String, onChange:(ComposerDraft)->Unit, modifier:Modifier=Modifier, onFocusChange:(Boolean)->Unit={},animationsAllowed:Boolean=true) {
    val stickers=remember(draft.text,server.emojis) {composerStickers(draft.text,server.emojis)}
    val ids=remember(stickers) {stickers.map {it.id}.toSet()}
    val owner="${server.url}/${server.me}"
    val pixels=with(LocalDensity.current) {28.sp.toPx().roundToInt().coerceAtLeast(1)}
    val loaded by produceState(LoadedComposerStickers(owner,pixels,emptyMap()),owner,ids,pixels) {
        val retained=if(value.owner==owner && value.size==pixels) value.images.filterKeys {it in ids} else emptyMap()
        value=LoadedComposerStickers(owner,pixels,retained)
        for(id in ids-retained.keys) {
            try {
                val bytes=model.emojiImage(server,id)
                val drawable=withContext(Dispatchers.IO) {
                    ImageDecoder.decodeDrawable(ImageDecoder.createSource(ByteBuffer.wrap(bytes))) {decoder,info,_ ->
                        decoder.allocator=ImageDecoder.ALLOCATOR_SOFTWARE
                        val scale=pixels.toFloat()/maxOf(info.size.width,info.size.height).coerceAtLeast(1)
                        decoder.setTargetSize((info.size.width*scale).roundToInt().coerceAtLeast(1),(info.size.height*scale).roundToInt().coerceAtLeast(1))
                    }
                }
                value=value.copy(images=value.images+(id to drawable))
            } catch(e:CancellationException) {throw e}
            catch(_:Exception) { /* Keep the editable :name: text when decoding is unavailable. */ }
        }
    }
    val images=if(loaded.owner==owner && loaded.size==pixels) loaded.images else emptyMap()
    val color=MaterialTheme.colorScheme.primary
    val projection=remember(draft.text,draft.mentions,stickers,images.keys,color) {ComposerProjection(draft,stickers,images.keys,color)}
    var layout by remember {mutableStateOf<TextLayoutResult?>(null)}
    var frame by remember {mutableIntStateOf(0)}
    val editorScroll=rememberScrollState()
    var editorHeight by remember {mutableIntStateOf(0)}
    val visibleStickers by remember(projection,layout) {derivedStateOf {
        val result=layout
        if(result==null || result.layoutInput.text.text!=projection.annotated.text) emptySet()
        else projection.positions.filter {position ->
            val bounds=result.getBoundingBox(position.offset)
            bounds.bottom>editorScroll.value && bounds.top<editorScroll.value+editorHeight
        }.map {it.id}.toSet()
    }}
    val lifecycle=LocalLifecycleOwner.current.lifecycle
    DisposableEffect(images,lifecycle,visibleStickers,animationsAllowed) {
        val handler=Handler(Looper.getMainLooper())
        val callback=object:Drawable.Callback {
            override fun invalidateDrawable(who:Drawable) {frame++}
            override fun scheduleDrawable(who:Drawable,what:Runnable,`when`:Long) {handler.postAtTime(what,who,`when`)}
            override fun unscheduleDrawable(who:Drawable,what:Runnable) {handler.removeCallbacks(what,who)}
        }
        fun active() {images.forEach {(id,d) -> d.callback=callback; (d as? Animatable)?.let {if(animationsAllowed && id in visibleStickers && lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) it.start() else it.stop()} }}
        val observer=LifecycleEventObserver {_,_ ->active()}
        lifecycle.addObserver(observer); active()
        onDispose {lifecycle.removeObserver(observer); images.values.forEach {d -> (d as? Animatable)?.stop(); d.callback=null; handler.removeCallbacksAndMessages(d)}}
    }
    Box(modifier.padding(start=14.dp,end=8.dp,top=12.dp,bottom=12.dp).heightIn(min=24.dp,max=160.dp).onSizeChanged {editorHeight=it.height}.verticalScroll(editorScroll)) {
        BasicTextField(draft.value,onValueChange={next ->onChange(draft.deleteSticker(next,stickers.filter {it.id in images}.map {it.range}))},
            modifier=Modifier.fillMaxWidth().onFocusChanged {onFocusChange(it.isFocused)}.testTag("composer-input").semantics {
                if(projection.positions.isNotEmpty()) stateDescription="Contains stickers: ${projection.positions.map {it.name}.distinct().joinToString()}"
            }.drawWithContent {
                frame // AnimatedImageDrawable invalidates this draw without recomposing the editor.
                drawContent()
                val result=layout
                if(result!=null && result.layoutInput.text.text==projection.annotated.text) {
                    projection.positions.forEach {position ->
                        val drawable=images[position.id] ?: return@forEach
                        val bounds=result.getBoundingBox(position.offset)
                        val side=minOf(bounds.width,bounds.height)
                        val ratio=drawable.intrinsicWidth.toFloat()/drawable.intrinsicHeight.coerceAtLeast(1)
                        val width=if(ratio>=1) side else side*ratio
                        val height=if(ratio>=1) side/ratio else side
                        val left=(bounds.center.x-width/2).roundToInt(); val top=(bounds.center.y-height/2).roundToInt()
                        drawable.setBounds(left,top,left+width.roundToInt(),top+height.roundToInt())
                        drawable.draw(drawContext.canvas.nativeCanvas)
                    }
                }
            },textStyle=MaterialTheme.typography.bodyLarge.copy(color=MaterialTheme.colorScheme.onSurface,lineHeight=if(projection.positions.isEmpty()) 24.sp else 32.sp),
            cursorBrush=SolidColor(color),visualTransformation=projection,onTextLayout={layout=it},
            keyboardOptions=KeyboardOptions(capitalization=KeyboardCapitalization.Sentences),
            decorationBox={inner ->Box {if(draft.text.isEmpty()) Text(placeholder,color=MaterialTheme.colorScheme.onSurfaceVariant,style=MaterialTheme.typography.bodyLarge,maxLines=1,softWrap=false,overflow=TextOverflow.Ellipsis); inner()}})
    }
}

@Composable
internal fun ComposerSuggestions(draft:ComposerDraft, server:ServerState, model:PapoViewModel, onPick:(ComposerDraft)->Unit) {
    val trigger=draft.trigger() ?: return
    val query=trigger.query
    val people=if(trigger.people) server.members.filter {it.name.contains(query,true) || it.username.contains(query,true)}.sortedWith(compareBy<Person> {!it.name.startsWith(query,true)}.thenBy {it.name.lowercase()}) else emptyList()
    val stickers=if(!trigger.people) server.emojis.filter {it.name.contains(query,true)}.sortedWith(compareBy<EmojiView> {!it.name.startsWith(query,true)}.thenBy {it.name.lowercase()}) else emptyList()
    if(people.isEmpty() && stickers.isEmpty()) return
    GlassPill(Modifier.fillMaxWidth().padding(bottom=8.dp),radius=24) {
        LazyColumn(Modifier.heightIn(max=216.dp).testTag("composer-suggestions"),contentPadding=PaddingValues(6.dp)) {
            items(people,key={"person/${it.id}"}) {person ->
                SuggestionRow(person.name,person.username,{UserAvatar(person.id,person.name)},"Mention ${person.name}") {onPick(draft.insert("@${person.name}",trigger,person))}
            }
            items(stickers,key={"sticker/${it.id}"}) {e ->
                SuggestionRow(":${e.name}:","Server sticker",{EmojiGlyph(server,EmojiChoice(e.id,true,e.name),36,model)},"Insert :${e.name}:") {onPick(draft.insert(":${e.name}:",trigger))}
            }
        }
    }
}

@Composable
private fun SuggestionRow(title:String,subtitle:String,leading:@Composable ()->Unit,label:String,onClick:()->Unit) {
    androidx.compose.material3.ListItem(headlineContent={Text(title,maxLines=1,overflow=TextOverflow.Ellipsis)},supportingContent={Text(subtitle,maxLines=1,overflow=TextOverflow.Ellipsis)},leadingContent=leading,
        colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.clickable(onClickLabel=label,onClick=onClick))
}
