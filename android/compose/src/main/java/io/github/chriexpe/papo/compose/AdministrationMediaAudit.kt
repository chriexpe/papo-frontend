package io.github.chriexpe.papo.compose

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@Composable
internal fun ServerStickerAdministration(server:ServerState,model:PapoViewModel,ui:AdministrationUi) {
    var source by remember {mutableStateOf<MediaEditorSource?>(null)}
    var delete by remember {mutableStateOf<String?>(null)}
    var opening by remember {mutableStateOf(false)}
    val context=LocalContext.current
    val scope=rememberCoroutineScope()
    val picker=rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) {uri ->if(uri!=null) scope.launch {
        opening=true
        try {source=MediaEditorSource.read(context,uri)} catch(e:CancellationException) {throw e} catch(e:Exception) {ui.error=e.message ?: ui(R.string.couldn_t_open_this_image)} finally {opening=false}
    }}
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-stickers"),contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        item {Button(enabled=!ui.busy && !opening,onClick={picker.launch(arrayOf("image/png","image/jpeg","image/gif","image/webp"))},modifier=Modifier.fillMaxWidth().heightIn(min=52.dp)) {Icon(Icons.Outlined.AddPhotoAlternate,null);Spacer(Modifier.width(8.dp));Text(ui(R.string.add_sticker))};Text(ui(R.string.png_jpeg_webp_or_gif_crop_and_preview_before_uploading_animated_stickers_ke),Modifier.padding(top=8.dp),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}
        if(opening) item {LinearProgressIndicator(Modifier.fillMaxWidth())}
        if(server.emojis.isEmpty()) item {AdministrationEmpty(ui(R.string.no_server_stickers_yet))}
        items(server.emojis.chunked(3),key={it.first().id}) {row ->Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(12.dp)) {row.forEach {sticker ->
            Card(Modifier.weight(1f)) {Column(Modifier.fillMaxWidth().padding(8.dp),horizontalAlignment=Alignment.CenterHorizontally) {
                Box(Modifier.height(80.dp).fillMaxWidth(),contentAlignment=Alignment.Center) {EmojiGlyph(server,EmojiChoice(sticker.id,true,sticker.name),64,model)}
                Text(":${sticker.name}:",maxLines=2,overflow=TextOverflow.Ellipsis,style=MaterialTheme.typography.labelMedium)
                IconButton(enabled=!ui.busy,onClick={delete=sticker.id}) {Icon(Icons.Outlined.DeleteOutline,ui(R.string.delete_sticker_1_s, sticker.name))}
            }}
        };repeat(3-row.size) {Spacer(Modifier.weight(1f))}}}
        item {Text(ui(R.string.the_server_supports_adding_and_deleting_stickers_to_change_a_name_upload_a),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}
    }
    source?.let {image ->MediaEditor(image,MediaEditPurpose.STICKER,server,model,onDismiss={source=null}) {prepared,name ->ui.apply(AdministrationChange.CreateSticker(name,prepared.blob,prepared.format),ui(R.string.sticker_added))}}
    delete?.let {id ->val sticker=server.emojis.find {it.id==id};AdministrationDelete(ui(R.string.delete_named,":${sticker?.name.orEmpty()}:"),ui(R.string.this_removes_the_sticker_from_the_server_picker_existing_messages_may_no_lo),ui.busy,{delete=null}) {ui.mutate(AdministrationChange.DeleteSticker(id),ui(R.string.sticker_deleted)) {delete=null}}}
}

@Composable
private fun AuditFilterChip(value:String,label:String,choices:List<Pair<String,String>>,enabled:Boolean=true,onChange:(String)->Unit) {
    var expanded by remember {mutableStateOf(false)}
    Box {
        AssistChip(onClick={expanded=true},label={Text(ui(R.string.filter_label_value,label,choices.find {it.first==value}?.second ?: ui(R.string.unknown)),maxLines=1)},trailingIcon={Icon(Icons.Outlined.ExpandMore,null,Modifier.size(18.dp))},enabled=enabled)
        DropdownMenu(expanded,{expanded=false}) {choices.forEach {(id,text)->DropdownMenuItem(text={Text(text)},leadingIcon={if(value==id) Icon(Icons.Outlined.Check,null)},onClick={expanded=false;onChange(id)})}}
    }
}

@Composable
internal fun AuditAdministration(server:ServerState,model:PapoViewModel) {
    val portuguese=androidx.compose.ui.platform.LocalConfiguration.current.locales[0].language=="pt"
    var person by rememberSaveable {mutableStateOf("")}
    var kind by rememberSaveable {mutableStateOf("all")}
    var channel by rememberSaveable {mutableStateOf("")}
    var period by rememberSaveable {mutableStateOf("all")}
    var rows by remember {mutableStateOf<List<AuditEntryView>>(emptyList())}
    var cursor by remember {mutableStateOf("")}
    var more by remember {mutableStateOf(false)}
    var loading by remember {mutableStateOf(false)}
    var error by remember {mutableStateOf("")}
    var generation by remember {mutableIntStateOf(0)}
    val scope=rememberCoroutineScope()
    suspend fun fetch(reset:Boolean) {
        val requestGeneration=generation
        loading=true;error=""
        try {
            val result=model.administrationAudit(server,AuditFilter(person,kind,channel,period,if(reset) "" else cursor,portuguese))
            if(generation==requestGeneration) {
                rows=if(reset) result.entries else (rows+result.entries).distinctBy {it.id}
                more=result.hasMore && result.cursor.isNotBlank() && (reset || result.cursor!=cursor)
                cursor=result.cursor
            }
        } catch(e:CancellationException) {throw e} catch(e:Exception) {if(generation==requestGeneration) error=e.message ?: ui(R.string.couldn_t_load_the_audit_log)} finally {if(generation==requestGeneration) loading=false}
    }
    LaunchedEffect(person,kind,channel,period,portuguese) {generation++;rows=emptyList();cursor="";more=false;loading=true;delay(250);fetch(true)}
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-audit"),contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        item {OutlinedTextField(person,{person=it},Modifier.fillMaxWidth().testTag("audit-person-search"),label={Text(ui(R.string.person))},placeholder={Text(ui(R.string.name_username_or_id),maxLines=1,overflow=TextOverflow.Ellipsis)},singleLine=true,leadingIcon={Icon(Icons.Outlined.Search,null)},trailingIcon={if(person.isNotEmpty()) IconButton(onClick={person=""}) {Icon(Icons.Outlined.Close,ui(R.string.clear_person_filter))}})}
        item {FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            FilterChip(kind=="deleted",{kind=if(kind=="deleted") "all" else "deleted"},label={Text(ui(R.string.deleted_messages))},leadingIcon={Icon(Icons.Outlined.DeleteOutline,null,Modifier.size(18.dp))})
            AuditFilterChip(kind,ui(R.string.type),listOf("all" to uiLabel("All"),"deleted" to ui(R.string.deleted_messages),"messages" to ui(R.string.messages),"channels" to uiLabel("Channels"),"roles" to uiLabel("Roles"),"members" to uiLabel("People"),"server" to ui(R.string.server))) {kind=it}
            AuditFilterChip(channel,ui(R.string.channel),listOf("" to uiLabel("All"))+server.channels.filter {it.kind!="category" && it.kind!="direct"}.map {it.id to it.name}) {channel=it}
            AuditFilterChip(period,ui(R.string.date),listOf("all" to ui(R.string.any_date),"today" to uiLabel("Today"),"week" to ui(R.string.last_7_days),"month" to ui(R.string.last_30_days))) {period=it}
        }}
        if(loading) item {LinearProgressIndicator(Modifier.fillMaxWidth())}
        if(error.isNotBlank()) item {SettingsError(error) {if(!loading) scope.launch {fetch(cursor.isEmpty())}}}
        if(!loading && error.isBlank() && rows.isEmpty()) item {AdministrationEmpty(if(person.isBlank() && kind=="all" && channel.isEmpty() && period=="all") ui(R.string.no_server_activity_yet) else ui(R.string.no_matching_activity_in_these_entries))}
        rows.forEachIndexed {index,entry ->
            if(index==0 || rows[index-1].day!=entry.day) item(key="day/${entry.id}") {Text(entry.day,Modifier.padding(top=20.dp,bottom=8.dp),style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)}
            item(key=entry.id) {AuditTimelineEntry(entry)}
        }
        if(more) item {OutlinedButton(enabled=!loading,onClick={scope.launch {fetch(false)}},modifier=Modifier.fillMaxWidth().heightIn(min=48.dp).testTag("audit-load-more")) {Text(if(loading) ui(R.string.loading) else ui(R.string.load_older_activity))}}
    }
}

@Composable
private fun AuditTimelineEntry(entry:AuditEntryView) {
    val primary=MaterialTheme.colorScheme.primary
    val icon=when(entry.action.substringBefore('.')) {"channel"->Icons.Outlined.Tag;"role","user_role"->Icons.Outlined.Shield;"user"->Icons.Outlined.PersonOutline;"emoji"->Icons.Outlined.EmojiEmotions;"server"->Icons.Outlined.Dns;else->Icons.Outlined.ChatBubbleOutline}
    Row(Modifier.fillMaxWidth().padding(vertical=10.dp).testTag("audit-entry-${entry.id}"),horizontalArrangement=Arrangement.spacedBy(12.dp)) {
        Surface(shape=MaterialTheme.shapes.large,color=MaterialTheme.colorScheme.surfaceContainerHigh) {Icon(icon,null,Modifier.padding(10.dp).size(20.dp),tint=if(entry.danger) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant)}
        Column(Modifier.weight(1f),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            val sentence=buildAnnotatedString {entry.parts.forEach {part ->withStyle(when(part.kind) {"person"->SpanStyle(fontWeight=FontWeight.SemiBold);"channel"->SpanStyle(color=primary,fontWeight=FontWeight.Medium);else->SpanStyle()}) {append(part.text)}}}
            SelectionContainer {Text(sentence,style=MaterialTheme.typography.bodyMedium)}
            if(entry.deletedText.isNotBlank()) Surface(shape=MaterialTheme.shapes.medium,color=MaterialTheme.colorScheme.surfaceContainerHigh) {SelectionContainer {Text(entry.deletedText,Modifier.padding(12.dp),style=MaterialTheme.typography.bodySmall)}}
        }
        Text(entry.time,style=MaterialTheme.typography.labelSmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
    }
}
