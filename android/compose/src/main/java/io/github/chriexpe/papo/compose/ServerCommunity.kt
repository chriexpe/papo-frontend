@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
package io.github.chriexpe.papo.compose

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.Capabilities
import kotlinx.coroutines.CancellationException

internal val Capabilities.canAdmin get()=manageServer || manageChannels || manageRoles

@Composable
internal fun ServerIcon(server:ServerState,model:PapoViewModel,modifier:Modifier=Modifier) {
    val context=LocalContext.current
    val revision=server.serverIconRevision
    val bytes by produceState<ByteArray?>(null,server.url,server.me,revision) {
        value=null
        if(revision.isNotEmpty()) try {value=model.serverIcon(server)} catch(e:CancellationException) {throw e} catch(_:Exception) {}
    }
    var loaded by remember(server.url,revision) {mutableStateOf(false)}
    Box(modifier.size(48.dp).clip(RoundedCornerShape(16.dp)).testTag("server-icon").semantics {
        contentDescription="${server.name} server icon";stateDescription=if(loaded) "Icon loaded" else "Initials"
    }) {
        if(!loaded) Avatar(server.name,Modifier.fillMaxSize())
        bytes?.let {ManagedAsyncImage(it,null,MediaImages.loader(context),Modifier.fillMaxSize(),ContentScale.Crop,onSuccess={loaded=true},onError={loaded=false},retentionKey="server/${server.url}/$revision")}
    }
}

@Composable
internal fun ServerHeader(server:ServerState,model:PapoViewModel,appError:String) {
    var panel by rememberSaveable(server.url) {mutableStateOf("")}
    var settingsFromInfo by remember {mutableStateOf(false)}
    val canAdmin=server.capabilities.canAdmin
    GlassPill(Modifier.padding(start=20.dp,end=20.dp,top=16.dp,bottom=8.dp).fillMaxWidth(),radius=24) {
        Row(Modifier.fillMaxWidth().heightIn(min=76.dp).testTag("server-header").clickable(onClickLabel="Server information") {panel="info"}.padding(start=12.dp,end=8.dp,top=10.dp,bottom=10.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
            ServerIcon(server,model)
            Text(server.name,Modifier.weight(1f),style=MaterialTheme.typography.titleLarge,fontWeight=FontWeight.SemiBold,maxLines=2,overflow=TextOverflow.Ellipsis)
            if(canAdmin) IconButton(onClick={settingsFromInfo=false;panel="settings"}) {Icon(Icons.Outlined.Settings,"Server settings")}
            else Icon(Icons.Outlined.ExpandMore,null,Modifier.padding(12.dp),tint=MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
    if(panel=="info") ServerInfoSheet(server,model,appError,onDismiss={panel=""},onAdmin={settingsFromInfo=true;panel="settings"})
    if(panel=="settings" && canAdmin) ServerSettingsSheet(server,model,appError) {panel=if(settingsFromInfo) "info" else ""}
    LaunchedEffect(canAdmin) {if(!canAdmin && panel=="settings") panel=""}
}

@Composable
private fun ServerInfoSheet(server:ServerState,model:PapoViewModel,appError:String,onDismiss:()->Unit,onAdmin:()->Unit) {
    BackHandler(onBack=onDismiss)
    val clipboard=LocalClipboardManager.current
    var copied by remember {mutableStateOf(false)}
    ModalBottomSheet(onDismissRequest=onDismiss,sheetState=rememberModalBottomSheetState(skipPartiallyExpanded=true)) {
        LazyColumn(Modifier.fillMaxWidth().heightIn(max=640.dp).navigationBarsPadding().testTag("server-info"),contentPadding=PaddingValues(bottom=24.dp)) {
            item {Row(Modifier.fillMaxWidth().padding(horizontal=20.dp,vertical=8.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                ServerIcon(server,model)
                Column(Modifier.weight(1f)) {
                    Text(server.name,style=MaterialTheme.typography.titleLarge,fontWeight=FontWeight.SemiBold)
                    Text("${server.members.count {it.presence=="online"}} online · ${server.members.size} people",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if(server.capabilities.canAdmin) IconButton(onClick=onAdmin) {Icon(Icons.Outlined.Settings,"Server settings")}
                IconButton(onClick=onDismiss) {Icon(Icons.Outlined.Close,"Close server information")}
            }}
            item {CommunitySection("ADDRESS")}
            item {ListItem(headlineContent={Text(server.url.removePrefix("https://").removePrefix("http://"),style=MaterialTheme.typography.bodyMedium,maxLines=2,overflow=TextOverflow.Ellipsis)},
                supportingContent=if(copied) ({Text("Copied address")}) else null,trailingContent={Icon(Icons.Outlined.ContentCopy,"Copy server address")},
                colors=ListItemDefaults.colors(containerColor=MaterialTheme.colorScheme.surfaceContainerLow),modifier=Modifier.clickable {clipboard.setText(AnnotatedString(server.url));copied=true})}
            if(server.channels.any {it.kind!="category" && it.kind!="direct"}) item {CommunitySection("NOTIFICATIONS")}
            items(server.channels.filter {it.kind!="category" && it.kind!="direct"},key={it.id}) {channel ->
                var expanded by remember {mutableStateOf(false)}
                val label=when(channel.notificationSettings) {"all"->"All messages";"off"->"Off";else->"Mentions only"}
                ListItem(headlineContent={Text(channel.name,maxLines=2,overflow=TextOverflow.Ellipsis)},leadingContent={Icon(if(channel.kind=="voice") Icons.Outlined.VolumeUp else Icons.Outlined.Tag,null)},
                    trailingContent={Box {
                        Row(verticalAlignment=Alignment.CenterVertically) {Text(label,style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant);Icon(Icons.Outlined.ChevronRight,null,Modifier.size(20.dp))}
                        DropdownMenu(expanded,{expanded=false}) {
                            listOf("all" to "All messages","only_mentions" to "Mentions only","off" to "Off").forEach {(value,title) ->
                                DropdownMenuItem(text={Text(title)},leadingIcon={if(channel.notificationSettings==value) Icon(Icons.Outlined.Check,null)},onClick={expanded=false;model.command(Action.ChannelNotifications(channel.id,value),server.url)})
                            }
                        }
                    }},colors=ListItemDefaults.colors(containerColor=MaterialTheme.colorScheme.surfaceContainerLow),modifier=Modifier.testTag("channel-notifications-${channel.id}").clickable {expanded=true})
            }
            item {HorizontalDivider(Modifier.padding(vertical=12.dp));TextButton(onClick={model.command(Action.MarkServerRead,server.url)},modifier=Modifier.fillMaxWidth()) {Icon(Icons.Outlined.DoneAll,null);Spacer(Modifier.width(8.dp));Text("Mark server as read")}}
            if(appError.isNotBlank() || server.error.isNotBlank()) item {CommunityError(appError.ifBlank {server.error})}
        }
    }
}

@Composable private fun CommunitySection(title:String) {HorizontalDivider(Modifier.padding(top=16.dp,bottom=12.dp));Text(title,Modifier.padding(horizontal=20.dp,vertical=6.dp),style=MaterialTheme.typography.labelSmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}
@Composable private fun CommunityError(error:String) {Text(error,Modifier.padding(20.dp),color=MaterialTheme.colorScheme.error,style=MaterialTheme.typography.bodyMedium)}

@Composable
private fun ServerSettingsSheet(server:ServerState,model:PapoViewModel,appError:String,onDismiss:()->Unit) {
    BackHandler(onBack=onDismiss)
    val tabs=buildList {if(server.capabilities.manageServer) add("General");if(server.capabilities.manageChannels) add("Channels");if(server.capabilities.manageRoles) add("Roles")}
    var tab by rememberSaveable(server.url) {mutableStateOf(tabs.firstOrNull().orEmpty())}
    val selected=tab.takeIf {it in tabs} ?: tabs.firstOrNull().orEmpty()
    var channelId by rememberSaveable {mutableStateOf<String?>(null)}
    var roleId by rememberSaveable {mutableStateOf<String?>(null)}
    ModalBottomSheet(onDismissRequest=onDismiss,sheetState=rememberModalBottomSheetState(skipPartiallyExpanded=true)) {
        Column(Modifier.fillMaxWidth().fillMaxHeight(.9f).imePadding().navigationBarsPadding().testTag("server-settings")) {
            Row(Modifier.fillMaxWidth().padding(start=24.dp,end=12.dp),verticalAlignment=Alignment.CenterVertically) {
                Text("Server settings",Modifier.weight(1f),style=MaterialTheme.typography.headlineSmall,fontWeight=FontWeight.SemiBold)
                IconButton(onClick=onDismiss) {Icon(Icons.Outlined.Close,"Close server settings")}
            }
            Row(Modifier.padding(horizontal=20.dp,vertical=8.dp),horizontalArrangement=Arrangement.spacedBy(8.dp)) {tabs.forEach {title ->FilterChip(selected==title,{tab=title;channelId=null;roleId=null},label={Text(title)})}}
            if(appError.isNotBlank() || server.error.isNotBlank()) CommunityError(appError.ifBlank {server.error})
            when(selected) {
                "General" -> GeneralServerSettings(server,model)
                "Channels" -> {
                    val channel=server.channels.firstOrNull {it.id==channelId}
                    if(channel!=null) key(channel.id) {ChannelSettings(server,channel,model) {channelId=null}}
                    else LazyColumn(Modifier.weight(1f),contentPadding=PaddingValues(bottom=24.dp)) {items(server.channels.filter {it.kind!="direct"},key={it.id}) {c ->
                        ListItem(headlineContent={Text(c.name)},supportingContent={Text(c.topic, maxLines=1,overflow=TextOverflow.Ellipsis)},trailingContent={Icon(Icons.Outlined.Edit,null)},
                            modifier=Modifier.clickable {channelId=c.id})
                    }}
                }
                "Roles" -> {
                    val role=server.roles.firstOrNull {it.id==roleId}
                    if(role!=null) key(role.id) {RoleSettings(server,role,model) {roleId=null}}
                    else LazyColumn(Modifier.weight(1f),contentPadding=PaddingValues(bottom=24.dp)) {
                        if(server.roles.isEmpty()) item {EmptyState("No roles yet","This server has no roles to edit.")}
                        items(server.roles,key={it.id}) {r ->ListItem(headlineContent={Text(r.name)},trailingContent={Icon(Icons.Outlined.Edit,null)},modifier=Modifier.clickable {roleId=r.id})}
                    }
                }
            }
        }
    }
}

@Composable
private fun GeneralServerSettings(server:ServerState,model:PapoViewModel) {
    var name by rememberSaveable(server.url) {mutableStateOf(server.name)}
    var public by rememberSaveable(server.url) {mutableStateOf(server.serverPublic)}
    var password by remember {mutableStateOf("")}
    var saving by remember {mutableStateOf(false)}
    var saved by remember {mutableStateOf(false)}
    val error=(!public && server.serverPublic || password.isNotEmpty()) && password.codePointCount(0,password.length)<8
    val dirty=name.trim()!=server.name || public!=server.serverPublic || password.isNotEmpty()
    LaunchedEffect(server.busy,server.name,server.serverPublic,server.error) {
        if(saving && !server.busy) {saving=false;if(server.error.isBlank() && name.trim()==server.name && public==server.serverPublic) {password="";saved=true}}
    }
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal=24.dp,vertical=12.dp),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        ServerIcon(server,model,Modifier.size(72.dp))
        OutlinedTextField(name,{name=it;saved=false},label={Text("Server name")},singleLine=true,modifier=Modifier.fillMaxWidth().testTag("server-name-field"))
        ListItem(headlineContent={Text("Public server")},supportingContent={Text("Private servers require a server password.")},trailingContent={Switch(public,{public=it;saved=false})},modifier=Modifier.clip(RoundedCornerShape(18.dp)))
        if(!public) OutlinedTextField(password,{password=it;saved=false},label={Text(if(server.serverPublic) "Server password" else "New server password (optional)")},visualTransformation=PasswordVisualTransformation(),singleLine=true,isError=error,
            supportingText=if(error) ({Text("Use at least 8 characters.")}) else null,modifier=Modifier.fillMaxWidth())
        Button(onClick={saving=true;saved=false;model.command(Action.SaveServer(name,public,password.takeIf {it.isNotEmpty()}),server.url)},enabled=dirty && name.trim().isNotEmpty() && name.codePointCount(0,name.length)<=64 && !error && !saving && !server.busy,modifier=Modifier.align(Alignment.End)) {Text(if(saving) "Saving…" else "Save server")}
        if(saved) Text("Server saved",color=MaterialTheme.colorScheme.primary)
    }
}

@Composable
private fun ChannelSettings(server:ServerState,channel:io.github.chriexpe.papo.core.ChannelView,model:PapoViewModel,onDone:()->Unit) {
    var name by rememberSaveable {mutableStateOf(channel.name)}
    var topic by rememberSaveable {mutableStateOf(channel.topic)}
    var saving by remember {mutableStateOf(false)}
    BackHandler(onBack=onDone)
    LaunchedEffect(channel.name,channel.topic,server.error) {if(saving) {saving=false;if(name==channel.name && topic==channel.topic) onDone()}}
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(24.dp),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        TextButton(onClick=onDone) {Icon(Icons.Outlined.ArrowBack,null);Spacer(Modifier.width(8.dp));Text("Channels")}
        OutlinedTextField(name,{name=it},label={Text("Channel name")},singleLine=true,modifier=Modifier.fillMaxWidth())
        OutlinedTextField(topic,{topic=it},label={Text("Description")},minLines=3,maxLines=6,modifier=Modifier.fillMaxWidth())
        Button(onClick={saving=true;model.command(Action.SaveChannel(channel.id,name,topic),server.url)},enabled=!saving && name.trim().isNotEmpty() && name.codePointCount(0,name.length)<=64 && (name!=channel.name || topic!=channel.topic),modifier=Modifier.align(Alignment.End)) {Text(if(saving) "Saving…" else "Save channel")}
    }
}

@Composable
private fun RoleSettings(server:ServerState,role:io.github.chriexpe.papo.core.RoleView,model:PapoViewModel,onDone:()->Unit) {
    var name by rememberSaveable {mutableStateOf(role.name)}
    var color by rememberSaveable {mutableStateOf(role.color)}
    var permissions by remember {mutableStateOf(role.permissions)}
    var saving by remember {mutableStateOf(false)}
    BackHandler(onBack=onDone)
    LaunchedEffect(role.name,role.color,role.permissions,server.error) {if(saving) {saving=false;if(name==role.name && color==role.color && permissions==role.permissions) onDone()}}
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(24.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        TextButton(onClick=onDone) {Icon(Icons.Outlined.ArrowBack,null);Spacer(Modifier.width(8.dp));Text("Roles")}
        OutlinedTextField(name,{name=it},label={Text("Role name")},singleLine=true,modifier=Modifier.fillMaxWidth())
        OutlinedTextField(color,{color=it},label={Text("Color (optional)")},placeholder={Text("#0088FF")},singleLine=true,modifier=Modifier.fillMaxWidth())
        listOf("Manage server" to permissions.manageServer,"Manage channels" to permissions.manageChannels,"Manage roles" to permissions.manageRoles,"Ban members" to permissions.banMembers,"Pin messages" to permissions.pinMessage,"Mention everyone" to permissions.everyoneMessage,"Send attachments" to permissions.sendAttachment).forEachIndexed {index,(label,value) ->
            ListItem(headlineContent={Text(label)},trailingContent={Switch(value,{enabled ->permissions=when(index) {
                0->permissions.copy(manageServer=enabled);1->permissions.copy(manageChannels=enabled);2->permissions.copy(manageRoles=enabled);3->permissions.copy(banMembers=enabled);4->permissions.copy(pinMessage=enabled);5->permissions.copy(everyoneMessage=enabled);else->permissions.copy(sendAttachment=enabled)
            }})})
        }
        Button(onClick={saving=true;model.command(Action.SaveRole(role.id,name,color,permissions),server.url)},enabled=!saving && name.trim().isNotEmpty() && name.codePointCount(0,name.length)<=64 && (color.isEmpty() || Regex("#[a-fA-F0-9]{6}").matches(color)) && (name!=role.name || color!=role.color || permissions!=role.permissions),modifier=Modifier.align(Alignment.End)) {Text(if(saving) "Saving…" else "Save role")}
    }
}
