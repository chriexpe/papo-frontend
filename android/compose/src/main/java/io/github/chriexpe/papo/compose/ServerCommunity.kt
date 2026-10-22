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

internal val Capabilities.canAdmin get()=manageServer || manageChannels || manageRoles || banMembers

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
internal fun ServerHeader(server:ServerState,model:PapoViewModel,appError:String,modifier:Modifier=Modifier) {
    var panel by rememberSaveable(server.url) {mutableStateOf("")}
    var settingsFromInfo by remember {mutableStateOf(false)}
    val canAdmin=server.capabilities.canAdmin
    GlassPill(modifier.fillMaxWidth(),radius=24) {
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
