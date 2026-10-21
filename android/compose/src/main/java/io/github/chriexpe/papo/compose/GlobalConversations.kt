package io.github.chriexpe.papo.compose

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.InboxItem
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

@Composable
internal fun GlobalAction(title:String,icon:ImageVector,count:Int,onClick:()->Unit,modifier:Modifier=Modifier) {
    GlassPill(modifier,radius=24) {
        Column(Modifier.fillMaxWidth().heightIn(min=88.dp).clickable(onClick=onClick).padding(14.dp),horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(8.dp)) {
            BadgedBox(badge={if(count>0) Badge {Text(if(count>99) "99+" else count.toString())}}) {Icon(icon,null,Modifier.size(24.dp))}
            Text(title,style=MaterialTheme.typography.labelLarge,maxLines=2,overflow=TextOverflow.Ellipsis)
        }
    }
}

/** Shared by server-scoped and global lists; only the scope label changes. */
@Composable
internal fun DirectMessageItem(server:ServerState,model:PapoViewModel,channel:Conversation,onOpen:()->Unit,onProfile:()->Unit,showServer:Boolean=false) {
    CompositionLocalProvider(LocalProfileContext provides ProfileContext(server,model)) {
        ListItem(headlineContent={Text(channel.name,fontWeight=if(channel.unread) FontWeight.SemiBold else FontWeight.Medium,maxLines=1,overflow=TextOverflow.Ellipsis)},
            supportingContent=if(showServer) ({Text(server.name,maxLines=1,overflow=TextOverflow.Ellipsis)}) else null,
            leadingContent={UserAvatar(channel.userId,channel.name,Modifier.clickable(onClick=onProfile))},
            trailingContent={if(channel.mentions>0) Badge {Text(channel.mentions.toString())} else if(channel.unread) Box(Modifier.size(8.dp).background(MaterialTheme.colorScheme.primary,CircleShape)) else Icon(Icons.Outlined.ChevronRight,null)},
            colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.testTag("dm-${server.key}-${channel.id}").clip(RoundedCornerShape(18.dp)).clickable(onClick=onOpen))
    }
}
@Composable
private fun GlobalHeader(title:String,onBack:()->Unit,action:@Composable ()->Unit={}) {
    Row(Modifier.fillMaxWidth().statusBarsPadding().padding(12.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
        GlassPill {IconButton(onClick=onBack) {Icon(Icons.Outlined.ArrowBack,"Back to servers")}}
        Text(title,Modifier.weight(1f),style=MaterialTheme.typography.headlineSmall,fontWeight=FontWeight.SemiBold)
        action()
    }
}
internal data class ScopedDirect(val server:ServerState,val channel:Conversation)
internal fun globalDirects(state:PapoState,query:String)=state.servers.filter {it.screen=="chat" && it.me.isNotBlank()}.flatMap {server ->
    server.dms.filter {dm ->listOf(dm.name,server.name,server.members.firstOrNull {it.id==dm.userId}?.username.orEmpty()).any {it.contains(query.trim(),true)}}.map {ScopedDirect(server,it)}
}.sortedWith(compareByDescending<ScopedDirect> {it.channel.unread}.thenBy(String.CASE_INSENSITIVE_ORDER) {it.channel.name}.thenBy {it.server.url})

@Composable
internal fun GlobalDirectMessages(state:PapoState,model:PapoViewModel,onBack:()->Unit,onProfile:(ServerState,String)->Unit,onOpen:(ServerState,Conversation)->Unit) {
    var query by rememberSaveable {mutableStateOf("")}
    BackHandler(onBack=onBack)
    Column(Modifier.fillMaxSize().testTag("global-direct-messages")) {
        GlobalHeader("Direct messages",onBack)
        OutlinedTextField(query,{query=it},Modifier.fillMaxWidth().padding(horizontal=20.dp,vertical=8.dp).testTag("direct-search"),placeholder={Text("Search names or servers")},singleLine=true,shape=RoundedCornerShape(24.dp),leadingIcon={Icon(Icons.Outlined.Search,null)},trailingIcon={if(query.isNotBlank()) IconButton(onClick={query=""}) {Icon(Icons.Outlined.Close,"Clear direct search")}})
        val rows=globalDirects(state,query)
        LazyColumn(Modifier.weight(1f).hazeSource(LocalHaze.current).testTag("global-direct-list"),contentPadding=PaddingValues(12.dp,8.dp,12.dp,140.dp)) {
            if(rows.isEmpty()) item {EmptyState(if(query.isBlank()) "No direct messages yet" else "No matching conversations",if(query.isBlank()) "Start a conversation with someone from a server’s search or profile." else "Search by nickname, username or server name.")}
            items(rows,key={"${it.server.url}/${it.server.me}/${it.channel.id}"}) {row ->DirectMessageItem(row.server,model,row.channel,{onOpen(row.server,row.channel)},{onProfile(row.server,row.channel.userId)},showServer=true)}
        }
    }
}

private data class InboxPage(val url:String,val owner:String,val items:List<InboxItem> = emptyList(),val loading:Boolean=true,val error:String="")
private data class ScopedNotice(val server:ServerState,val item:InboxItem)
@Composable
internal fun GlobalNotifications(state:PapoState,model:PapoViewModel,onBack:()->Unit,onOpen:(ServerState,InboxItem)->Unit) {
    val servers=state.servers.filter {it.screen=="chat" && it.me.isNotBlank()}
    val owners=servers.map {Triple(it.url,it.me,it.notificationCount)}
    var refresh by remember {mutableIntStateOf(0)}
    var pages by remember {mutableStateOf<List<InboxPage>>(emptyList())}
    var reading by remember {mutableStateOf<Set<String>>(emptySet())}
    val scope=rememberCoroutineScope()
    val snackbar=remember {SnackbarHostState()}
    BackHandler(onBack=onBack)
    LaunchedEffect(owners,refresh) {
        pages=servers.map {s ->pages.firstOrNull {it.url==s.url && it.owner==s.me}?.copy(loading=true,error="") ?: InboxPage(s.url,s.me)}
        coroutineScope {servers.forEach {server ->launch {
            val page=try {InboxPage(server.url,server.me,model.inbox(server),loading=false)}
            catch(e:CancellationException) {throw e}
            catch(e:Exception) {InboxPage(server.url,server.me,items=pages.firstOrNull {it.url==server.url && it.owner==server.me}?.items.orEmpty(),loading=false,error=e.message ?: "Couldn’t load notifications")}
            pages=pages.map {if(it.url==page.url && it.owner==page.owner) page else it}
        }}}
    }
    val activePages=pages.filter {p ->servers.any {it.url==p.url && it.me==p.owner}}
    val rows=activePages.flatMap {page ->val server=servers.first {it.url==page.url && it.me==page.owner};page.items.map {ScopedNotice(server,it)}}.sortedWith(compareByDescending<ScopedNotice> {it.item.at}.thenBy {it.server.url}.thenBy {it.item.id})
    Box(Modifier.fillMaxSize().testTag("global-notifications")) {
        Column(Modifier.fillMaxSize()) {
            GlobalHeader("Notifications",onBack) {IconButton(onClick={refresh++},enabled=activePages.none {it.loading}) {Icon(Icons.Outlined.Refresh,"Refresh notifications")}}
            if(activePages.any {it.loading}) LinearProgressIndicator(Modifier.fillMaxWidth().padding(horizontal=20.dp))
            LazyColumn(Modifier.weight(1f).hazeSource(LocalHaze.current).testTag("notification-list"),contentPadding=PaddingValues(12.dp,8.dp,12.dp,140.dp),verticalArrangement=Arrangement.spacedBy(6.dp)) {
                items(activePages.filter {it.error.isNotBlank()},key={"error/${it.url}"}) {page ->Surface(shape=RoundedCornerShape(20.dp),color=MaterialTheme.colorScheme.errorContainer,modifier=Modifier.padding(8.dp)) {
                    Column(Modifier.padding(16.dp)) {Text(servers.first {it.url==page.url}.name,fontWeight=FontWeight.SemiBold);Text(page.error,style=MaterialTheme.typography.bodyMedium);TextButton(onClick={refresh++}) {Text("Retry")}}
                }}
                if(rows.isEmpty() && activePages.none {it.loading || it.error.isNotBlank()}) item {EmptyState("All quiet for now","Notifications from your signed-in servers will appear here.")}
                items(rows,key={"${it.server.url}/${it.server.me}/${it.item.id}"}) {notice ->
                    val server=notice.server;val item=notice.item
                    val key="${server.url}/${server.me}/${item.id}"
                    val canOpen=(server.channels+server.dms).any {it.id==item.channelId && it.kind!="category"}
                    CompositionLocalProvider(LocalProfileContext provides ProfileContext(server,model)) {
                        ListItem(overlineContent={Text(server.name,maxLines=1,overflow=TextOverflow.Ellipsis)},headlineContent={Text(item.author.ifBlank {"Message"}+(if(item.channelName.isNotBlank()) " · ${item.channelName}" else ""),fontWeight=if(item.read) FontWeight.Normal else FontWeight.SemiBold,maxLines=2,overflow=TextOverflow.Ellipsis)},
                            supportingContent={Column(verticalArrangement=Arrangement.spacedBy(4.dp)) {Text(item.content.ifBlank {"New message"},maxLines=3,overflow=TextOverflow.Ellipsis);if(item.at>0) Text(Instant.ofEpochMilli(item.at).atZone(ZoneId.systemDefault()).format(DateTimeFormatter.ofLocalizedDateTime(FormatStyle.SHORT)),style=MaterialTheme.typography.labelSmall);if(!canOpen) Text("Conversation unavailable",style=MaterialTheme.typography.labelSmall)}},
                            leadingContent={if(item.authorId.isNotBlank()) UserAvatar(item.authorId,item.author) else Icon(Icons.Outlined.NotificationsNone,null)},
                            trailingContent={if(!item.read) IconButton(enabled=key !in reading,onClick={
                                reading=reading+key
                                scope.launch {try {
                                    model.readInbox(server,listOf(item.id))
                                    pages=pages.map {page ->if(page.url==server.url && page.owner==server.me) page.copy(items=page.items.map {if(it.id==item.id) it.copy(read=true) else it}) else page}
                                } catch(e:CancellationException) {throw e} catch(e:Exception) {snackbar.showSnackbar(e.message ?: "Couldn’t mark notification as read")} finally {reading=reading-key}}
                            }) {Icon(Icons.Outlined.Done,"Mark notification as read")} else Icon(Icons.Outlined.ChevronRight,null)},
                            colors=ListItemDefaults.colors(containerColor=if(item.read) MaterialTheme.colorScheme.surfaceContainerLow else MaterialTheme.colorScheme.primaryContainer.copy(alpha=.4f)),
                            modifier=Modifier.testTag("notification-${server.key}-${item.id}").clip(RoundedCornerShape(20.dp)).semantics {stateDescription=if(item.read) "Read" else "Unread"}.clickable(enabled=canOpen,onClickLabel="Open notification message") {onOpen(server,item)})
                    }
                }
            }
        }
        SnackbarHost(snackbar,Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(12.dp))
    }
}
