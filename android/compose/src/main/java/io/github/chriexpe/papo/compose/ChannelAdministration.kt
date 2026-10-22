package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

@Composable
internal fun AdministrationEmpty(text:String) {Text(text,Modifier.padding(20.dp),color=MaterialTheme.colorScheme.onSurfaceVariant)}

@Composable
internal fun AdministrationDelete(title:String,text:String,busy:Boolean,onDismiss:()->Unit,onDelete:()->Unit) {
    AlertDialog(onDismissRequest={if(!busy) onDismiss()},title={Text(title)},text={Text(text)},confirmButton={TextButton(enabled=!busy,onClick=onDelete) {Text("Delete",color=MaterialTheme.colorScheme.error)}},dismissButton={TextButton(enabled=!busy,onClick=onDismiss) {Text("Cancel")}})
}

@Composable
internal fun ChannelAdministrationList(server:ServerState,ui:AdministrationUi,open:(String,String)->Unit) {
    val channels=ui.data?.channels.orEmpty().sortedBy {it.position}
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-channels"),contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        item {FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {Button(enabled=!ui.busy,onClick={open("New channel","")}) {Icon(Icons.Outlined.Add,null);Text("Create channel")};OutlinedButton(enabled=!ui.busy,onClick={open("New category","")}) {Text("Create category")}}}
        if(channels.isEmpty() && !ui.loading) item {AdministrationEmpty("No channels yet. Create a text channel to get started.")}
        items(channels,key={it.id}) {channel ->
            val siblings=channels.filter {it.parentId==channel.parentId}
            val index=siblings.indexOfFirst {it.id==channel.id}
            SettingsCard {
                SettingsLink(channel.name,if(channel.kind=="category") "Category" else listOfNotNull(channels.find {it.id==channel.parentId}?.name,if(channel.kind=="voice") "Voice channel" else "Text channel").joinToString(" · "),when(channel.kind) {"category"->Icons.Outlined.Folder;"voice"->Icons.Outlined.VolumeUp;else->Icons.Outlined.Tag},{open("Channel/${channel.id}","")})
                Row(Modifier.fillMaxWidth().padding(horizontal=8.dp),horizontalArrangement=Arrangement.End) {
                    IconButton(enabled=!ui.busy && index>0,onClick={ui.mutate(AdministrationChange.MoveChannel(channel.id,siblings[index-1].id,null),"Channel moved")}) {Icon(Icons.Outlined.ArrowUpward,"Move ${channel.name} up")}
                    IconButton(enabled=!ui.busy && index>=0 && index<siblings.lastIndex,onClick={ui.mutate(AdministrationChange.MoveChannel(channel.id,siblings[index+1].id,null),"Channel moved")}) {Icon(Icons.Outlined.ArrowDownward,"Move ${channel.name} down")}
                }
            }
        }
    }
}

@Composable
internal fun ChannelAdministrationEditor(server:ServerState,ui:AdministrationUi,id:String?,category:Boolean,onDirty:(Boolean)->Unit,onSaved:()->Unit,open:(String,String)->Unit={_,_->}) {
    val existing=ui.data?.channels?.find {it.id==id}
    if(id!=null && existing==null) {if(!ui.loading) AdministrationEmpty("This channel no longer exists.");return}
    var name by rememberSaveable(id) {mutableStateOf(existing?.name.orEmpty())}
    var topic by rememberSaveable(id) {mutableStateOf(existing?.topic.orEmpty())}
    var kind by rememberSaveable(id) {mutableStateOf(existing?.kind ?: if(category) "category" else "text")}
    var delete by remember {mutableStateOf(false)}
    val changed=if(id==null) name.isNotEmpty() || topic.isNotEmpty() || kind!=(if(category) "category" else "text") else name!=existing?.name || topic!=existing.topic
    SideEffect {onDirty(changed)}
    val valid=name.trim().isNotEmpty() && name.codePointCount(0,name.length)<=32 && topic.codePointCount(0,topic.length)<=512
    val target=LocalSettingTarget.current
    val list=rememberLazyListState()
    LaunchedEffect(target) {when(target) {"channel-topic"->list.scrollToItem(2);"channel-delete"->list.scrollToItem(if(kind=="category") 3 else 5)}}
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-channel-editor"),state=list,contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        item {OutlinedTextField(name,{name=it},Modifier.fillMaxWidth().then(settingTarget("channel-name")).testTag("admin-channel-name"),label={Text(if(kind=="category") "Category name" else "Channel name")},singleLine=true,enabled=!ui.busy,isError=name.codePointCount(0,name.length)>32,supportingText={Text("Up to 32 characters")})}
        item {SettingsLabel("Type");FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {(if(category || existing?.kind=="category") listOf("category" to "Category") else listOf("text" to "Text","voice" to "Voice")).forEach {(value,label)->FilterChip(kind==value,{kind=value},label={Text(label)},enabled=id==null && !ui.busy && !category)}};if(id!=null) Text("The channel type is fixed after creation.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}
        if(kind!="category") {
            item {OutlinedTextField(topic,{topic=it},Modifier.fillMaxWidth().then(settingTarget("channel-topic")),label={Text("Description")},minLines=3,maxLines=8,enabled=!ui.busy,isError=topic.codePointCount(0,topic.length)>512,supportingText={Text("${topic.codePointCount(0,topic.length)} / 512")})}
            if(id!=null) item {SettingsCard {SettingsLink("Channel access","Read, send, moderate and connect by role",Icons.Outlined.Lock,{open("Access/$id","channel-access")},target="channel-access")}}
        }
        item {Button(enabled=valid && !ui.busy && (id==null || changed),onClick={
            val change=if(id==null) AdministrationChange.CreateChannel(name,kind,topic,null,emptyList()) else AdministrationChange.UpdateChannel(id,name,topic)
            ui.mutate(change,if(id==null) "${if(kind=="category") "Category" else "Channel"} created" else "Channel saved") {onSaved()}
        },modifier=Modifier.fillMaxWidth().heightIn(min=48.dp).testTag("save-channel")) {Text(if(id==null) "Create ${if(kind=="category") "category" else "channel"}" else "Save channel")}}
        if(id!=null) item {OutlinedButton(enabled=!ui.busy,onClick={delete=true},modifier=Modifier.fillMaxWidth().heightIn(min=48.dp).then(settingTarget("channel-delete"))) {Icon(Icons.Outlined.DeleteOutline,null);Spacer(Modifier.width(8.dp));Text("Delete ${if(kind=="category") "category" else "channel"}",color=MaterialTheme.colorScheme.error)}}
    }
    if(delete && id!=null) AdministrationDelete("Delete ${name.ifBlank {"channel"}}?",if(kind=="category") "This removes the category. Other channels remain available." else "This permanently deletes the channel and its messages.",ui.busy,{delete=false}) {ui.mutate(AdministrationChange.DeleteChannel(id),"Channel deleted") {delete=false;onSaved()}}
}

@Composable
internal fun ChannelAccessAdministration(server:ServerState,ui:AdministrationUi,id:String) {
    var rules by remember {mutableStateOf<List<ChannelRule>>(emptyList())}
    var loading by remember {mutableStateOf(true)}
    var error by remember {mutableStateOf("")}
    var editing by remember {mutableStateOf<ChannelRule?>(null)}
    val scope=rememberCoroutineScope()
    suspend fun load() {loading=true;error="";try {rules=ui.model.channelRules(server,id)} catch(e:CancellationException) {throw e} catch(e:Exception) {error=e.message.orEmpty()} finally {loading=false}}
    LaunchedEffect(server.url,server.me,id) {load()}
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-channel-access"),contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        item {Text("Access is saved on the server and applies to every device. With no role rules, the channel is open to everyone.",style=MaterialTheme.typography.bodyMedium)}
        if(loading) item {LinearProgressIndicator(Modifier.fillMaxWidth())}
        if(error.isNotBlank()) item {SettingsError(error) {scope.launch {load()}}}
        if(!loading && rules.isEmpty()) item {AdministrationEmpty("Everyone can access this channel.")}
        items(rules,key={it.roleId}) {rule ->SettingsCard {SettingsLink(rule.roleName,listOfNotNull(if(rule.access.read) "Read" else null,if(rule.access.send) "Send" else null,if(rule.access.deleteMessages) "Moderate" else null,if(rule.access.connectVoice) "Voice" else null).joinToString(" · ").ifEmpty {"No permissions"},Icons.Outlined.Shield,{editing=rule})}}
        item {SettingsLabel("Add a role");FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {server.roles.filter {r ->rules.none {it.roleId==r.id}}.forEach {role ->AssistChip(onClick={editing=ChannelRule(role.id,role.name,ChannelAccess(true,true,false,true))},label={Text(role.name)},leadingIcon={Icon(Icons.Outlined.Add,null)},enabled=!ui.busy && !loading)}}}
    }
    editing?.let {rule ->
        var read by remember(rule.roleId) {mutableStateOf(rule.access.read)}
        var send by remember(rule.roleId) {mutableStateOf(rule.access.send)}
        var moderate by remember(rule.roleId) {mutableStateOf(rule.access.deleteMessages)}
        var voice by remember(rule.roleId) {mutableStateOf(rule.access.connectVoice)}
        AlertDialog(onDismissRequest={if(!ui.busy) editing=null},title={Text(rule.roleName)},text={Column {
            AdministrationToggle("Read channel",read,!ui.busy) {read=it};AdministrationToggle("Send messages",send,!ui.busy) {send=it};AdministrationToggle("Delete messages",moderate,!ui.busy) {moderate=it};AdministrationToggle("Connect to voice",voice,!ui.busy) {voice=it}
            if(rules.any {it.roleId==rule.roleId}) TextButton(enabled=!ui.busy,onClick={scope.launch {try {ui.apply(AdministrationChange.RemoveChannelRule(id,rule.roleId),"Role rule removed");editing=null;load()} catch(e:CancellationException) {throw e} catch(_:Exception) {}}}) {Text("Remove role rule",color=MaterialTheme.colorScheme.error)}
            if(ui.error.isNotBlank()) Text(ui.error,color=MaterialTheme.colorScheme.error)
        }},confirmButton={TextButton(enabled=!ui.busy,onClick={scope.launch {try {ui.apply(AdministrationChange.SetChannelRule(id,rule.roleId,ChannelAccess(read,send,moderate,voice)),"Channel access saved");editing=null;load()} catch(e:CancellationException) {throw e} catch(_:Exception) {}}}) {Text("Save")}},dismissButton={TextButton(enabled=!ui.busy,onClick={editing=null}) {Text("Cancel")}})
    }
}

@Composable
internal fun AdministrationToggle(title:String,value:Boolean,enabled:Boolean,target:String="",onChange:(Boolean)->Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min=56.dp).then(settingTarget(target)),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {Text(title,Modifier.weight(1f));Switch(value,onChange,Modifier.semantics {contentDescription=title},enabled=enabled)}
}
