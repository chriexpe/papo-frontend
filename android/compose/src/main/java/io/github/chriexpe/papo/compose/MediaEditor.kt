package io.github.chriexpe.papo.compose

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.RestartAlt
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.chrisbanes.haze.HazeState
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.MediaCrop
import io.github.chriexpe.papo.core.MediaEditPurpose
import io.github.chriexpe.papo.core.PreparedMediaImage
import io.github.chriexpe.papo.core.inspectMediaImage
import io.github.chriexpe.papo.core.prepareMediaImage
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import kotlin.math.min
import kotlin.math.roundToInt

/** A local edit source. Picked sources are temporary; an existing attachment
 * stays untouched until its edited replacement has been saved to the draft. */
internal data class MediaEditorSource(val path:String,val name:String,val width:Int,val height:Int,val temporary:Boolean=false) {
    companion object {
        suspend fun read(context:Context,uri:Uri):MediaEditorSource = withContext(Dispatchers.IO) {
            val directory=context.cacheDir.resolve("image-editor").apply {mkdirs()}
            directory.listFiles().orEmpty().filter {System.currentTimeMillis()-it.lastModified()>24*60*60*1000L}.forEach(File::delete)
            val name=context.contentResolver.query(uri,arrayOf(OpenableColumns.DISPLAY_NAME),null,null,null)?.use {if(it.moveToFirst()) it.getString(0) else null}.orEmpty().ifBlank {"image"}
            val copy=File.createTempFile("source-",".image",directory)
            try {
                requireNotNull(context.contentResolver.openInputStream(uri)) {ui(R.string.couldn_t_open_this_image)}.use {input ->copy.outputStream().use {output ->
                    val buffer=ByteArray(65536);var total=0L
                    while(true) {val count=input.read(buffer);if(count<0) break;total+=count;require(total<=32L*1024*1024) {ui(R.string.choose_an_image_up_to_32_mb)};output.write(buffer,0,count)}
                }}
                val info=inspectMediaImage(copy.absolutePath)
                MediaEditorSource(copy.absolutePath,name,info.width.toInt(),info.height.toInt(),true)
            } catch(error:Throwable) {copy.delete();throw error}
        }
        suspend fun attachment(file:PendingAttachment):MediaEditorSource=withContext(Dispatchers.IO) {
            val info=inspectMediaImage(file.path)
            MediaEditorSource(file.path,file.name,info.width.toInt(),info.height.toInt())
        }
    }
}

/** Fractions of the oriented source, shared by the preview and native export. */
internal fun mediaCrop(width:Int,height:Int,aspect:Float,zoom:Float,centerX:Float,centerY:Float):MediaCrop {
    require(width>0 && height>0 && aspect.isFinite() && aspect>0 && zoom.isFinite() && centerX.isFinite() && centerY.isFinite())
    val cropWidth=min(1f,height.toFloat()*aspect/width)/zoom.coerceIn(1f,6f)
    val cropHeight=(cropWidth*width/(aspect*height)).coerceAtMost(1f)
    val x=centerX.coerceIn(cropWidth/2,1-cropWidth/2)-cropWidth/2
    val y=centerY.coerceIn(cropHeight/2,1-cropHeight/2)-cropHeight/2
    return MediaCrop(x,y,cropWidth,cropHeight)
}
internal val PreparedMediaImage.mime get()=when(format) {"GIF"->"image/gif";"JPEG"->"image/jpeg";"PNG"->"image/png";else->"image/webp"}
internal val PreparedMediaImage.extension get()=if(format=="JPEG") "jpg" else format.lowercase()

@Composable
internal fun MediaEditor(source:MediaEditorSource,purpose:MediaEditPurpose,server:ServerState,model:PapoViewModel,onDismiss:()->Unit,onApply:suspend (PreparedMediaImage,String)->Unit) {
    val scope=rememberCoroutineScope()
    val context=LocalContext.current
    var busy by remember {mutableStateOf(false)}
    var error by remember {mutableStateOf("")}
    var zoom by rememberSaveable(source.path) {mutableFloatStateOf(1f)}
    var centerX by rememberSaveable(source.path) {mutableFloatStateOf(.5f)}
    var centerY by rememberSaveable(source.path) {mutableFloatStateOf(.5f)}
    var mode by rememberSaveable(source.path) {mutableStateOf(if(purpose==MediaEditPurpose.STICKER) "square" else "original")}
    var name by rememberSaveable(source.path) {mutableStateOf(source.name.substringBeforeLast('.').lowercase().map {if(it.isLetterOrDigit() && it.code<128 || it=='_' || it=='-') it else '-'}.joinToString("").trim('-').take(32).ifBlank {"sticker"})}
    val aspect=when(purpose) {MediaEditPurpose.AVATAR,MediaEditPurpose.SERVER_ICON->1f;MediaEditPurpose.BANNER->3f;else->when(mode) {"square"->1f;"wide"->16f/9;else->source.width.toFloat()/source.height}}
    val crop=mediaCrop(source.width,source.height,aspect,zoom,centerX,centerY)
    var prepared by remember {mutableStateOf<Pair<MediaCrop,PreparedMediaImage>?>(null)}
    val title=when(purpose) {MediaEditPurpose.STICKER->ui(R.string.new_sticker);MediaEditPurpose.AVATAR->ui(R.string.adjust_profile_photo);MediaEditPurpose.SERVER_ICON->ui(R.string.adjust_server_picture);MediaEditPurpose.BANNER->ui(R.string.adjust_banner);else->ui(R.string.edit_image)}
    fun update(value:Float,x:Float=centerX,y:Float=centerY) {
        val next=mediaCrop(source.width,source.height,aspect,value,x,y)
        zoom=value.coerceIn(1f,6f);centerX=next.x+next.width/2;centerY=next.y+next.height/2
    }
    DisposableEffect(source.path) {onDispose {if(source.temporary) File(source.path).delete()}}
    Dialog(onDismissRequest={if(!busy) onDismiss()},properties=DialogProperties(usePlatformDefaultWidth=false,decorFitsSystemWindows=false)) {
        val haze=remember {HazeState()}
        CompositionLocalProvider(LocalHaze provides haze,LocalProfileContext provides ProfileContext(server,model)) {
            Surface(Modifier.fillMaxSize(),color=MaterialTheme.colorScheme.background) {
                Column(Modifier.fillMaxSize().navigationBarsPadding().imePadding().testTag("media-editor")) {
                    SettingsHeader(title,{if(!busy) onDismiss()},enabled=!busy)
                    Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).hazeSource(haze).padding(horizontal=16.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                        Text("${source.name} · ${source.width} × ${source.height}",maxLines=1,overflow=TextOverflow.Ellipsis,style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
                        var size by remember {mutableStateOf(IntSize.Zero)}
                        val frame=remember(size,aspect) {
                            val width=min(size.width-32f,size.height*aspect-32f*aspect).coerceAtLeast(1f)
                            val height=width/aspect
                            Rect((size.width-width)/2,(size.height-height)/2,(size.width+width)/2,(size.height+height)/2)
                        }
                        val radius=when(purpose) {MediaEditPurpose.AVATAR->min(frame.width,frame.height)/2;MediaEditPurpose.SERVER_ICON->frame.width*.28f;else->16f}
                        Box(Modifier.fillMaxWidth().height(340.dp).clip(RoundedCornerShape(20.dp)).background(Color(0xFF111114)).onSizeChanged {size=it}
                            .testTag("media-crop-area").semantics {
                                contentDescription=ui(R.string.crop_preview_drag_to_reposition_pinch_to_zoom)
                                customActions=listOf(CustomAccessibilityAction(ui(R.string.zoom_in)) {update(zoom+.25f);true},CustomAccessibilityAction(ui(R.string.zoom_out)) {update(zoom-.25f);true},CustomAccessibilityAction(ui(R.string.move_crop_left)) {update(zoom,centerX-.05f);true},CustomAccessibilityAction(ui(R.string.move_crop_right)) {update(zoom,centerX+.05f);true},CustomAccessibilityAction(ui(R.string.move_crop_up)) {update(zoom,centerX,centerY-.05f);true},CustomAccessibilityAction(ui(R.string.move_crop_down)) {update(zoom,centerX,centerY+.05f);true})
                            }.pointerInput(source.path,aspect,size,busy) {
                                if(!busy) detectTransformGestures {centroid,pan,scale,_ ->
                                    val before=mediaCrop(source.width,source.height,aspect,zoom,centerX,centerY)
                                    val oldScale=frame.width/(source.width*before.width)
                                    val newZoom=(zoom*scale).coerceIn(1f,6f)
                                    val after=mediaCrop(source.width,source.height,aspect,newZoom,centerX,centerY)
                                    val newScale=frame.width/(source.width*after.width)
                                    update(newZoom,centerX+((centroid.x-frame.center.x)*(1/oldScale-1/newScale)-pan.x/newScale)/source.width,centerY+((centroid.y-frame.center.y)*(1/oldScale-1/newScale)-pan.y/newScale)/source.height)
                                }
                            }) {
                            if(size.width>0) CroppedEditorImage(source,crop,Modifier.fillMaxSize(),frame)
                            Canvas(Modifier.fillMaxSize()) {
                                val outside=Path().apply {fillType=PathFillType.EvenOdd;addRect(Rect(Offset.Zero,this@Canvas.size));addRoundRect(RoundRect(frame,CornerRadius(radius)))}
                                drawPath(outside,Color.Black.copy(alpha=.58f))
                                drawRoundRect(Color.White,frame.topLeft,frame.size,CornerRadius(radius),style=Stroke(2.dp.toPx()))
                            }
                            if(purpose==MediaEditPurpose.BANNER && size.width>0) {
                                val density=LocalDensity.current
                                UserAvatar(server.me,server.myName,Modifier.offset {IntOffset((frame.left+frame.width*16/300).roundToInt(),(frame.bottom-frame.width*44/300).roundToInt())}.size(with(density) {(frame.width*80/300).toDp()}))
                            }
                        }
                        Text(ui(R.string.drag_to_reposition_pinch_or_use_the_slider_to_zoom),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                            Text("${(zoom*100).roundToInt()}%",style=MaterialTheme.typography.labelLarge)
                            Slider(zoom,{update(it)},Modifier.weight(1f).testTag("crop-zoom").semantics {contentDescription=ui(R.string.crop_zoom);stateDescription="${(zoom*100).roundToInt()} percent"},enabled=!busy,valueRange=1f..6f)
                            IconButton(enabled=!busy,onClick={zoom=1f;centerX=.5f;centerY=.5f}) {Icon(Icons.Outlined.RestartAlt,ui(R.string.reset_crop))}
                        }
                        if(purpose==MediaEditPurpose.STICKER || purpose==MediaEditPurpose.ATTACHMENT) FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                            val modes=if(purpose==MediaEditPurpose.STICKER) listOf("square" to uiLabel("Square"),"original" to ui(R.string.original_shape)) else listOf("original" to ui(R.string.original_shape),"square" to uiLabel("Square"),"wide" to "16:9")
                            modes.forEach {(value,label)->FilterChip(mode==value,{mode=value;zoom=1f;centerX=.5f;centerY=.5f},label={Text(label)},enabled=!busy)}
                        }
                        if(purpose==MediaEditPurpose.STICKER) {
                            OutlinedTextField(name,{name=it},Modifier.fillMaxWidth().testTag("sticker-name"),label={Text(ui(R.string.sticker_name))},prefix={Text(":")},suffix={Text(":")},singleLine=true,enabled=!busy,isError=!Regex("[A-Za-z0-9_+\\-]{1,32}").matches(name),supportingText={Text(ui(R.string.up_to_32_letters_numbers_or))})
                            SettingsCard {
                                Column(Modifier.padding(16.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                                    Text(ui(R.string.in_conversation),style=MaterialTheme.typography.labelLarge)
                                    Row(horizontalArrangement=Arrangement.spacedBy(12.dp)) {UserAvatar(server.me,server.myName);Column {Text(server.myName,style=MaterialTheme.typography.titleSmall);CroppedEditorImage(source,crop,Modifier.width(72.dp).aspectRatio(aspect));Row(verticalAlignment=Alignment.CenterVertically) {Text(ui(R.string.like_this));CroppedEditorImage(source,crop,Modifier.height(22.dp).aspectRatio(aspect))}}}
                                    Text(ui(R.string.in_the_picker),style=MaterialTheme.typography.labelLarge)
                                    Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {CroppedEditorImage(source,crop,Modifier.width(48.dp).aspectRatio(aspect).clip(RoundedCornerShape(10.dp)));Text(":$name:",Modifier.align(Alignment.CenterVertically),style=MaterialTheme.typography.bodySmall)}
                                }
                            }
                        }
                        val maxWidth=if(purpose==MediaEditPurpose.BANNER) 1536 else if(purpose==MediaEditPurpose.ATTACHMENT) 65535 else 512
                        val maxHeight=if(purpose==MediaEditPurpose.ATTACHMENT) 65535 else 512
                        val scale=min(1f,min(maxWidth/(source.width*crop.width),maxHeight/(source.height*crop.height)))
                        Text(ui(R.string.label_1_s_2_s_3_s_animated_images_stay_animated, (source.width*crop.width*scale).roundToInt().coerceAtLeast(1), (source.height*crop.height*scale).roundToInt().coerceAtLeast(1), when(purpose) {MediaEditPurpose.STICKER->ui(R.string.up_to_256_kb);MediaEditPurpose.ATTACHMENT->ui(R.string.up_to_32_mb);else->ui(R.string.up_to_2_mb)}),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                        if(error.isNotBlank()) SettingsError(error)
                        Spacer(Modifier.height(8.dp))
                    }
                    if(busy) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("media-editor-saving"))
                    Row(Modifier.fillMaxWidth().padding(16.dp),horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                        OutlinedButton(enabled=!busy,onClick=onDismiss,modifier=Modifier.heightIn(min=48.dp)) {Text(ui(R.string.cancel))}
                        Button(enabled=!busy && (purpose!=MediaEditPurpose.STICKER || Regex("[A-Za-z0-9_+\\-]{1,32}").matches(name)),modifier=Modifier.weight(1f).heightIn(min=48.dp).testTag("save-media-edit"),onClick={
                            busy=true;error=""
                            scope.launch {
                                try {
                                    val output=prepared?.takeIf {it.first==crop}?.second ?: withContext(Dispatchers.IO) {prepareMediaImage(source.path,crop,purpose)}.also {prepared=crop to it}
                                    onApply(output,name);onDismiss()
                                } catch(e:CancellationException) {throw e} catch(e:Exception) {error=e.message ?: ui(R.string.couldn_t_save_this_image)} finally {busy=false}
                            }
                        }) {Text(if(busy) ui(R.string.saving) else if(purpose==MediaEditPurpose.STICKER) ui(R.string.save_sticker) else "Save")}
                    }
                }
            }
        }
    }
}

@Composable
private fun CroppedEditorImage(source:MediaEditorSource,crop:MediaCrop,modifier:Modifier=Modifier,frame:Rect?=null) {
    val context=LocalContext.current
    var size by remember {mutableStateOf(IntSize.Zero)}
    Box(modifier.onSizeChanged {size=it}.clip(RoundedCornerShape(1.dp))) {
        val target=frame ?: Rect(0f,0f,size.width.toFloat(),size.height.toFloat())
        if(size.width>0 && size.height>0) ManagedAsyncImage(File(source.path),null,MediaImages.loader(context),Modifier.fillMaxSize().graphicsLayer {
            scaleX=target.width/size.width/crop.width
            scaleY=target.height/size.height/crop.height
            translationX=(.5f-crop.x-crop.width/2)*target.width/crop.width+target.center.x-size.width/2
            translationY=(.5f-crop.y-crop.height/2)*target.height/crop.height+target.center.y-size.height/2
        },ContentScale.FillBounds,retentionKey="editor/${source.path}")
    }
}
