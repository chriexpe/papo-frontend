@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
package io.github.chriexpe.papo.compose

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.chrisbanes.haze.HazeState
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

internal fun administrationPeople(count:Int)=if(count==1) "1 person" else "$count people"

@Stable
internal class AdministrationUi(val server:ServerState,val model:PapoViewModel,private val scope:CoroutineScope,val snackbar:SnackbarHostState) {
    var data by mutableStateOf<AdministrationState?>(null);private set
    var loading by mutableStateOf(false);private set
    var busy by mutableStateOf(false);private set
    var error by mutableStateOf("")
    fun refresh() {if(loading || busy) return;scope.launch {
        loading=true;error=""
        try {data=model.administration(server)} catch(e:CancellationException) {throw e} catch(e:Exception) {error=e.message ?: "Couldn’t load server settings"} finally {loading=false}
    }}
    suspend fun apply(change:AdministrationChange,message:String):AdministrationOutcome {
        check(!busy) {"Wait for the current change to finish"}
        busy=true;error=""
        try {
            val result=model.administer(server,change)
            var refreshError=result.refreshError
            try {data=model.administration(server,false)} catch(e:CancellationException) {throw e} catch(e:Exception) {refreshError=refreshError.ifBlank {e.message ?: "Refresh failed"}}
            if(refreshError.isNotBlank()) error="Saved, but couldn’t refresh the server. $refreshError"
            scope.launch {snackbar.showSnackbar(message)}
            return result
        } catch(e:CancellationException) {throw e} catch(e:Exception) {error=e.message ?: "Couldn’t save changes";throw e}
        finally {busy=false}
    }
    fun mutate(change:AdministrationChange,message:String,onSaved:(AdministrationOutcome)->Unit={}) {
        if(busy) return
        scope.launch {try {onSaved(apply(change,message))} catch(e:CancellationException) {throw e} catch(_:Exception) {}}
    }
}

internal fun serverSettingEntries(server:ServerState,data:AdministrationState?):List<SettingEntry> = buildList {
    val p=server.capabilities
    if(p.manageServer) {
        add(SettingEntry("server-name","General","Server name",server.name,"rename general"))
        add(SettingEntry("server-icon","General","Server picture","Crop and update the server icon","photo avatar image"))
        add(SettingEntry("server-access","General","Server access","Public or password protected","private password"))
        if(!server.serverPublic) add(SettingEntry("server-password","General","Server password","Change the server’s access password","private security"))
        add(SettingEntry("stickers","Stickers","Server stickers","Add animated or static stickers","emoji GIF images delete"))
        add(SettingEntry("audit","Audit","Audit log","Who changed what","activity history deleted messages actor channel date"))
    }
    if(p.manageChannels) {
        add(SettingEntry("create-channel","Channels","Create channel","Text and voice channels","new"))
        add(SettingEntry("create-category","Channels","Create category","Group server channels","new"))
        add(SettingEntry("reorder","Channels","Channel order","Move channels and categories","reorder organize"))
        data?.channels?.forEach {channel ->
            add(SettingEntry("channel-name","Channel/${channel.id}","${channel.name}: name","Rename this channel","channel"))
            if(channel.kind!="category") add(SettingEntry("channel-topic","Channel/${channel.id}","${channel.name}: description",channel.topic.ifBlank {"Channel topic"},"topic channel"))
            add(SettingEntry("channel-delete","Channel/${channel.id}","Delete ${channel.name}","Permanently delete this channel","remove channel"))
            if(channel.kind!="category") add(SettingEntry("channel-access","Access/${channel.id}","Access to ${channel.name}","Role permissions","read send voice channel"))
        }
    }
    if(p.manageRoles) {
        add(SettingEntry("create-role","Roles","Create role","Name, color and permissions","new tag"))
        server.roles.forEach {role ->
            add(SettingEntry("role-name","Role/${role.id}","${role.name}: name","Rename this role","tag"))
            add(SettingEntry("role-color","Role/${role.id}","${role.name}: color","Set a nickname color","hex tag"))
            listOf("manage-server" to "Manage server","manage-channels" to "Manage channels","manage-roles" to "Manage roles","ban-members" to "Ban people","pin-message" to "Pin messages","everyone-message" to "Mention everyone","send-attachment" to "Send attachments").forEach {(id,label) ->add(SettingEntry("role-$id","Role/${role.id}","${role.name}: $label","Role permission","permissions"))}
            add(SettingEntry("role-delete","Role/${role.id}","Delete ${role.name}","Remove this role and its channel rules","delete tag"))
            add(SettingEntry("role-members","RolePeople/${role.id}","People with ${role.name}","Assign or remove this role","member tag"))
        }
    }
    if(p.banMembers) add(SettingEntry("people","People","People and moderation","Ban or restore server access","members unban"))
}

@Composable
internal fun ServerSettingsSheet(server:ServerState,model:PapoViewModel,appError:String,onDismiss:()->Unit) {
    val scope=rememberCoroutineScope()
    val snackbar=remember {SnackbarHostState()}
    val ui=remember(server.url,server.me) {AdministrationUi(server,model,scope,snackbar)}
    var pages by rememberSaveable(server.url,server.me) {mutableStateOf(listOf("Settings"))}
    var dirtyPages by rememberSaveable(server.url,server.me) {mutableStateOf(emptyList<String>())}
    var query by rememberSaveable {mutableStateOf("")}
    var target by rememberSaveable {mutableStateOf("")}
    var discard by remember {mutableStateOf(false)}
    val holder=rememberSaveableStateHolder()
    val route=pages.last()
    val focus=LocalFocusManager.current
    val keyboard=LocalSoftwareKeyboardController.current
    fun pop() {holder.removeState(route);dirtyPages=dirtyPages-route;if(pages.size>1) pages=pages.dropLast(1) else onDismiss()}
    fun back() {if(!ui.busy) {if(route in dirtyPages) discard=true else pop()}}
    fun open(page:String,id:String="") {focus.clearFocus();keyboard?.hide();target=id;pages=pages+page}
    val dirty:(Boolean)->Unit={value ->if(value && route !in dirtyPages) dirtyPages=dirtyPages+route else if(!value && route in dirtyPages) dirtyPages=dirtyPages-route}
    val saved:()->Unit={dirtyPages=dirtyPages-route;pop()}
    LaunchedEffect(ui) {ui.refresh()}
    Dialog(onDismissRequest=::back,properties=DialogProperties(usePlatformDefaultWidth=false,decorFitsSystemWindows=false)) {
        val haze=remember {HazeState()}
        CompositionLocalProvider(LocalSettingTarget provides target,LocalHaze provides haze,LocalProfileContext provides ProfileContext(server,model)) {
            Surface(Modifier.fillMaxSize(),color=MaterialTheme.colorScheme.background) {
                Box(Modifier.fillMaxSize().navigationBarsPadding().imePadding().testTag("server-settings")) {
                    Column(Modifier.fillMaxSize()) {
                        val title=when {route=="Settings"->"Server settings";route.startsWith("Channel/")->"Edit channel";route.startsWith("Access/")->"Channel access";route.startsWith("RolePeople/")->"Role members";route.startsWith("Role/")->"Edit role";else->route}
                        SettingsHeader(title,::back,close=route=="Settings",enabled=!ui.busy) {
                            if(ui.busy) CircularProgressIndicator(Modifier.padding(12.dp).size(24.dp),strokeWidth=2.dp)
                        }
                        if(ui.error.isNotBlank()) SettingsError(ui.error,if(!ui.busy && !ui.loading) ({ui.refresh()}) else null)
                        if(ui.loading) LinearProgressIndicator(Modifier.fillMaxWidth().testTag("administration-loading"))
                        holder.SaveableStateProvider(route) {
                            when {
                                route=="Settings" -> ServerSettingsOverview(server,model,ui,query,{query=it},::open)
                                route=="General" -> GeneralAdministration(server,model,ui,dirty,saved)
                                route=="Channels" -> ChannelAdministrationList(server,ui,::open)
                                route=="New channel" -> ChannelAdministrationEditor(server,ui,null,false,dirty,saved)
                                route=="New category" -> ChannelAdministrationEditor(server,ui,null,true,dirty,saved)
                                route.startsWith("Channel/") -> ChannelAdministrationEditor(server,ui,route.substringAfter('/'),false,dirty,saved,::open)
                                route.startsWith("Access/") -> ChannelAccessAdministration(server,ui,route.substringAfter('/'))
                                route=="Roles" -> RoleAdministrationList(server,ui,::open)
                                route=="New role" -> RoleAdministrationEditor(server,ui,null,dirty,saved,::open)
                                route.startsWith("Role/") -> RoleAdministrationEditor(server,ui,route.substringAfter('/'),dirty,saved,::open)
                                route.startsWith("RolePeople/") -> RoleMembershipAdministration(server,ui,route.substringAfter('/'))
                                route=="People" -> PeopleAdministration(server,ui)
                                route=="Stickers" -> ServerStickerAdministration(server,model,ui)
                                route=="Audit" -> AuditAdministration(server,model)
                            }
                        }
                    }
                    SnackbarHost(snackbar,Modifier.align(Alignment.BottomCenter).padding(12.dp))
                }
                if(discard) AlertDialog(onDismissRequest={discard=false},title={Text("Discard changes?")},text={Text("Your unsaved edits will be lost.")},confirmButton={TextButton(onClick={discard=false;pop()}) {Text("Discard")}},dismissButton={TextButton(onClick={discard=false}) {Text("Keep editing")}})
            }
        }
    }
}

@Composable
private fun ServerSettingsOverview(server:ServerState,model:PapoViewModel,ui:AdministrationUi,query:String,onQuery:(String)->Unit,open:(String,String)->Unit) {
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("server-settings-overview"),contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(14.dp)) {
        item {OutlinedTextField(query,onQuery,Modifier.fillMaxWidth().testTag("server-settings-search"),singleLine=true,placeholder={Text("Search server settings")},leadingIcon={Icon(Icons.Outlined.Search,null)},trailingIcon={if(query.isNotBlank()) IconButton(onClick={onQuery("")}) {Icon(Icons.Outlined.Close,"Clear server settings search")}},shape=RoundedCornerShape(28.dp))}
        if(query.isNotBlank()) {
            val words=searchFold(query).trim().split(Regex("\\s+")).filter {it.isNotBlank()}
            val matches=serverSettingEntries(server,ui.data).filter {entry ->val text=searchFold("${entry.title} ${entry.page} ${entry.summary} ${entry.keywords}");words.all(text::contains)}
            items(matches,key={"${it.page}/${it.id}"}) {entry ->SettingsCard {SettingsLink(entry.title,entry.summary,Icons.Outlined.Tune,{open(entry.page,entry.id)})}}
            if(matches.isEmpty()) item {Text("No matching settings",Modifier.padding(16.dp),color=MaterialTheme.colorScheme.onSurfaceVariant)}
        } else {
            item {SettingsCard {Row(Modifier.fillMaxWidth().padding(16.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(16.dp)) {ServerIcon(server,model);Column {Text(server.name,style=MaterialTheme.typography.titleMedium);Text("${if(server.serverPublic) "Public" else "Private"} · ${administrationPeople(ui.data?.people?.size ?: server.members.size)}",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}}}}
            val p=server.capabilities
            if(p.manageServer) item {SettingsCard {SettingsLink("General","Name, picture and server access",Icons.Outlined.Settings,{open("General","")})}}
            if(p.manageChannels) item {SettingsCard {SettingsLink("Channels","${ui.data?.channels?.size ?: server.channels.count {it.kind!="direct"}} channels and categories",Icons.Outlined.Tag,{open("Channels","")})}}
            if(p.manageRoles) item {SettingsCard {SettingsLink("Roles","${server.roles.size} ${if(server.roles.size==1) "role" else "roles"} · colors, permissions and people",Icons.Outlined.Badge,{open("Roles","")})}}
            if(p.banMembers) item {SettingsCard {SettingsLink("People","Server access and moderation",Icons.Outlined.PeopleOutline,{open("People","")})}}
            if(p.manageServer) {
                item {SettingsCard {SettingsLink("Stickers","${server.emojis.size} server ${if(server.emojis.size==1) "sticker" else "stickers"}",Icons.Outlined.EmojiEmotions,{open("Stickers","")})}}
                item {SettingsCard {SettingsLink("Audit","Who changed what",Icons.Outlined.History,{open("Audit","")})}}
            }
        }
    }
}

@Composable
private fun GeneralAdministration(server:ServerState,model:PapoViewModel,ui:AdministrationUi,onDirty:(Boolean)->Unit,onSaved:()->Unit) {
    var name by rememberSaveable {mutableStateOf(server.name)}
    var public by rememberSaveable {mutableStateOf(server.serverPublic)}
    var password by remember {mutableStateOf("")}
    var editing by remember {mutableStateOf<MediaEditorSource?>(null)}
    val context=LocalContext.current
    val scope=rememberCoroutineScope()
    val picker=rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) {uri ->if(uri!=null) scope.launch {try {editing=MediaEditorSource.read(context,uri)} catch(e:CancellationException) {throw e} catch(e:Exception) {ui.error=e.message ?: "Couldn’t open this picture"}}}
    val dirty=name.trim()!=server.name || public!=server.serverPublic || password.isNotEmpty()
    SideEffect {onDirty(dirty)}
    val invalid=(!public && server.serverPublic || password.isNotEmpty()) && password.codePointCount(0,password.length)<8
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("server-general-settings"),contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        item {SettingsCard {SettingsLink("Server picture","Crop and preview before saving",Icons.Outlined.AddPhotoAlternate,{picker.launch(arrayOf("image/png","image/jpeg","image/gif","image/webp"))},leading={ServerIcon(server,model,Modifier.size(64.dp))},target="server-icon")}}
        item {OutlinedTextField(name,{name=it},Modifier.fillMaxWidth().then(settingTarget("server-name")).testTag("server-name-field"),label={Text("Server name")},singleLine=true,enabled=!ui.busy,isError=name.trim().isEmpty() || name.codePointCount(0,name.length)>32)}
        item {SettingsCard {Column(Modifier.padding(horizontal=16.dp).then(settingTarget("server-access"))) {AdministrationToggle("Public server",public,!ui.busy) {public=it};Text("Private servers require a server password",Modifier.padding(bottom=16.dp),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}}}
        if(!public) item {OutlinedTextField(password,{password=it},Modifier.fillMaxWidth().then(settingTarget("server-password")),singleLine=true,enabled=!ui.busy,visualTransformation=PasswordVisualTransformation(),label={Text(if(server.serverPublic) "Server password" else "New password (optional)")},isError=invalid,supportingText={Text(if(invalid) "Use at least eight characters" else "Leave empty to keep the current password")})}
        item {Button(enabled=dirty && name.trim().isNotEmpty() && name.codePointCount(0,name.length)<=32 && !invalid && !ui.busy,onClick={ui.mutate(AdministrationChange.Server(name,public,password.takeIf {it.isNotEmpty()}),"Server saved") {password="";onSaved()}},modifier=Modifier.fillMaxWidth().heightIn(min=48.dp)) {Text("Save server")}}
    }
    editing?.let {source ->MediaEditor(source,MediaEditPurpose.SERVER_ICON,server,model,onDismiss={editing=null}) {prepared,_ ->ui.apply(AdministrationChange.Icon(prepared.blob,prepared.format),"Server picture updated")}}
}
