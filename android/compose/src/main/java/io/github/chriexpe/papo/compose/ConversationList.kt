package io.github.chriexpe.papo.compose

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.SearchAnalysis
import io.github.chriexpe.papo.core.SearchSuggestion
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay

/** The editor uses ranges returned by the core, including quoted token suffixes. */
internal fun applySearchSuggestion(value:TextFieldValue,hint:SearchSuggestion):TextFieldValue {
    val start=hint.start.toInt().coerceIn(0,value.text.length)
    val end=hint.end.toInt().coerceIn(start,value.text.length)
    val prefix=value.text.take(start)+hint.replacement
    val complete=!hint.replacement.endsWith(':')
    val text=prefix+(if(complete) " " else "")+value.text.drop(end).let {if(complete) it.trimStart() else it}
    return TextFieldValue(text,TextRange(prefix.length+if(complete) 1 else 0))
}

@Composable
internal fun ConversationList(server:ServerState,model:PapoViewModel,appError:String,onOpen:(String)->Unit,onProfile:(String)->Unit,onMessage:(String,String)->Unit) {
    var section by rememberSaveable(server.url) {mutableStateOf("Channels")}
    // A server owns its search; changing servers never leaks terms/results.
    var query by rememberSaveable(server.url,stateSaver=TextFieldValue.Saver) {mutableStateOf(TextFieldValue())}
    var analysis by remember(server.url) {mutableStateOf<SearchAnalysis?>(null)}
    var focused by remember {mutableStateOf(false)}
    var analyzing by remember {mutableStateOf(false)}
    val searching=query.text.isNotBlank()
    val keyboard=LocalSoftwareKeyboardController.current
    val focus=LocalFocusManager.current
    fun update(value:TextFieldValue) {if(value.text!=query.text) analysis=null;query=value}
    fun pick(hint:SearchSuggestion) {update(applySearchSuggestion(query,hint))}
    BackHandler(searching) {update(TextFieldValue());focus.clearFocus();keyboard?.hide()}
    LaunchedEffect(query.text,query.selection,server.url,server.members,server.channels) {
        analyzing=true
        try {analysis=model.analyzeSearch(server,query.text,query.selection.end)}
        catch(e:CancellationException) {throw e} catch(_:Exception) {analysis=null}
        finally {analyzing=false}
    }
    LaunchedEffect(query.text,analysis?.valid,server.url,server.connection) {
        val text=query.text
        if(!searching || analysis?.valid!=true) {model.command(Action.ClearSearch,server.url);return@LaunchedEffect}
        delay(300)
        if(server.connection=="online" && server.searchQuery!=text) model.command(Action.Search(text,false),server.url)
    }
    val hints=if(focused) analysis?.suggestions.orEmpty() else emptyList()
    Column(Modifier.fillMaxSize().statusBarsPadding().imePadding()) {
        ServerHeader(server,model,appError)
        if(server.connection!="online") ConnectionBanner(server.connection) {model.command(Action.Refresh,server.url)}
        OutlinedTextField(query,::update,placeholder={Text("Search channels, people, messages")},leadingIcon={Icon(Icons.Outlined.Search,null)},
            trailingIcon=if(query.text.isNotEmpty()) ( {IconButton(onClick={update(TextFieldValue())}) {Icon(Icons.Outlined.Close,"Clear search")}} ) else null,
            singleLine=true,shape=RoundedCornerShape(24.dp),keyboardOptions=KeyboardOptions(imeAction=ImeAction.Search),
            keyboardActions=KeyboardActions(onSearch={if(hints.isNotEmpty()) pick(hints.first()) else {focus.clearFocus();keyboard?.hide()}}),
            modifier=Modifier.fillMaxWidth().padding(horizontal=20.dp,vertical=8.dp).testTag("global-search").onFocusChanged {focused=it.isFocused})
        if(hints.isNotEmpty()) Surface(Modifier.padding(horizontal=20.dp).fillMaxWidth(),shape=RoundedCornerShape(18.dp),color=MaterialTheme.colorScheme.surfaceContainer) {
            LazyColumn(Modifier.heightIn(max=216.dp).testTag("search-suggestions")) {
                items(hints,key={it.replacement}) {hint -> ListItem(headlineContent={Text(hint.label,maxLines=2,overflow=TextOverflow.Ellipsis)},
                    leadingContent={Icon(Icons.Outlined.FilterAlt,null)},colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.clickable {pick(hint)})}
            }
        }
        if(!searching) Row(Modifier.padding(horizontal=20.dp,vertical=8.dp),horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            listOf("Channels","Direct","People").forEach {label ->FilterChip(section==label,{section=label},label={Text(label)})}
        }
        val needle=if(searching && ':' !in query.text) query.text.trim() else ""
        val channels=server.channels.filter {it.kind=="text" && (!searching || (needle.isNotBlank() && it.name.contains(needle,true)))}
        val people=server.members.filter {it.id!=server.me && (!searching || (needle.isNotBlank() && (it.name.contains(needle,true)||it.username.contains(needle,true))))}
        val exact=server.searchQuery==query.text
        val results=if(exact) server.searchResults else emptyList()
        LazyColumn(Modifier.weight(1f).hazeSource(LocalHaze.current).testTag("conversation-results"),contentPadding=PaddingValues(start=12.dp,end=12.dp,bottom=128.dp)) {
            if(searching) {
                if(channels.isNotEmpty()) item {SearchHeading("Channels")}
                items(channels,key={"channel/${it.id}"}) {ChannelItem(it,onOpen)}
                if(people.isNotEmpty()) item {SearchHeading("People")}
                items(people,key={"person/${it.id}"}) {PersonItem(it,onProfile)}
                item {SearchHeading("Messages")}
                if(server.connection!="online") item {SearchNotice("Connect to search messages across this server.")}
                else if(!analyzing && analysis?.valid==false) item {SearchNotice("Complete a filter above, or enter a search term.")}
                else if(analyzing || !exact || server.searching) item {Box(Modifier.fillMaxWidth().padding(24.dp),contentAlignment=Alignment.Center) {CircularProgressIndicator(Modifier.size(24.dp))}}
                if(exact && !server.searching && server.error.isNotBlank()) item {SearchNotice(server.error)}
                if(exact && !server.searching && server.error.isBlank() && results.isEmpty() && analysis?.valid==true && server.connection=="online") item {SearchNotice("No matching messages.")}
                items(results,key={"message/${it.id}"}) {hit ->ListItem(headlineContent={Text(hit.author.ifBlank {"Message"},fontWeight=FontWeight.SemiBold)},
                    overlineContent={Text("#${hit.channelName}")},supportingContent={Text(hit.content.ifBlank {"Attachment"},maxLines=3,overflow=TextOverflow.Ellipsis)},
                    leadingContent={UserAvatar(hit.authorId,hit.author)},trailingContent={Icon(Icons.Outlined.ChevronRight,null)},colors=ListItemDefaults.colors(containerColor=Color.Transparent),
                    modifier=Modifier.testTag("search-hit-${hit.id}").clip(RoundedCornerShape(18.dp)).clickable {focus.clearFocus();keyboard?.hide();onMessage(hit.channelId,hit.id)})}
                if(exact && server.searchHasMore) item {TextButton(onClick={model.command(Action.Search(query.text,true),server.url)},enabled=!server.searching,modifier=Modifier.fillMaxWidth()) {Text("Load more messages")}}
            } else if(section=="People") {
                if(people.isEmpty()) item {EmptyState("No people here yet","People in your community will appear here.")}
                items(people,key={it.id}) {PersonItem(it,onProfile)}
            } else {
                val conversations=if(section=="Direct") server.dms else server.channels.filter {it.kind!="direct" && it.kind!="voice"}
                if(conversations.isEmpty()) item {EmptyState("A little quiet here.",if(section=="Direct") "Find someone in People to start a direct message." else "Your text channels will appear here.")}
                items(conversations,key={it.id}) {channel ->
                    if(channel.kind=="category") SearchHeading(channel.name.uppercase())
                    else if(section=="Direct") ListItem(headlineContent={Text(channel.name,fontWeight=FontWeight.Medium)},leadingContent={UserAvatar(channel.userId,channel.name,Modifier.clickable {onProfile(channel.userId)})},
                        trailingContent={if(channel.mentions>0) Badge {Text(channel.mentions.toString())} else Icon(Icons.Outlined.ChevronRight,null)},
                        colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.clip(RoundedCornerShape(18.dp)).clickable {onOpen(channel.id)})
                    else ChannelItem(channel,onOpen)
                }
            }
        }
    }
}

@Composable private fun SearchHeading(text:String) {Text(text,Modifier.padding(start=20.dp,top=20.dp,bottom=8.dp),style=MaterialTheme.typography.labelLarge,color=MaterialTheme.colorScheme.onSurfaceVariant)}
@Composable private fun SearchNotice(text:String) {Text(text,Modifier.padding(20.dp),style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)}
@Composable private fun PersonItem(person:io.github.chriexpe.papo.core.MemberView,onProfile:(String)->Unit) {
    ListItem(headlineContent={Text(person.name,fontWeight=FontWeight.Medium)},supportingContent={Text("@${person.username}")},leadingContent={UserAvatar(person.id,person.name)},
        trailingContent={Icon(Icons.Outlined.PersonOutline,null)},colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.clip(RoundedCornerShape(18.dp)).clickable {onProfile(person.id)})
}
@Composable private fun ChannelItem(channel:io.github.chriexpe.papo.core.ChannelView,onOpen:(String)->Unit) {
    ListItem(headlineContent={Text(channel.name,fontWeight=if(channel.unread) FontWeight.Bold else FontWeight.Medium)},
        supportingContent=if(channel.topic.isNotBlank()) ({Text(channel.topic,maxLines=1,overflow=TextOverflow.Ellipsis)}) else null,
        leadingContent={Box(Modifier.size(44.dp).clip(RoundedCornerShape(14.dp)).background(MaterialTheme.colorScheme.surfaceContainer),contentAlignment=Alignment.Center) {Text("#",fontSize=23.sp,color=MaterialTheme.colorScheme.onSurfaceVariant)}},
        trailingContent={if(channel.mentions>0) Badge {Text(channel.mentions.toString())} else if(channel.unread) Box(Modifier.size(8.dp).background(MaterialTheme.colorScheme.primary,CircleShape)) else Icon(Icons.Outlined.ChevronRight,null,tint=MaterialTheme.colorScheme.onSurfaceVariant)},
        colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.clip(RoundedCornerShape(18.dp)).clickable {onOpen(channel.id)})
}
