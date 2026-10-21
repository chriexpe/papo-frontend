@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.DocumentsContract
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.AccountChange
import io.github.chriexpe.papo.core.AccountProfile
import io.github.chriexpe.papo.core.SessionView
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.time.OffsetDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

private data class SettingsDestination(val title:String,val subtitle:String,val icon:ImageVector)
private val settingsDestinations=listOf(
    SettingsDestination("Appearance","Theme, glass and conversations",Icons.Outlined.Palette),
    SettingsDestination("Files","Downloads and save location",Icons.Outlined.Folder),
    SettingsDestination("Sessions","Manage signed-in sessions",Icons.Outlined.Devices),
    SettingsDestination("Diagnostics","Connection, media cache and app info",Icons.Outlined.MonitorHeart),
)
private data class SettingEntry(val id:String,val page:String,val title:String,val summary:String,val keywords:String="")
private data class AppearanceSwitch(val key:String,val title:String,val summary:String,val default:Boolean=true)
private val appearanceSwitches=listOf(
    AppearanceSwitch("avatars","Show avatars","Keep profile pictures beside messages"),
    AppearanceSwitch("timestamps","Show timestamps","Display the time beside each author"),
    AppearanceSwitch("compact","Compact messages","Reduce space between messages",false),
    AppearanceSwitch("animations","Animate GIFs and stickers","Turn off motion while keeping images visible"),
    AppearanceSwitch("description","Show channel description on open","Expand it for six seconds; tap to keep it open",false),
)
private val LocalSettingTarget=compositionLocalOf {""}
private fun searchFold(value:String)=java.text.Normalizer.normalize(value,java.text.Normalizer.Form.NFD).replace(Regex("\\p{M}+"),"").lowercase(java.util.Locale.ROOT)
private fun settingsEntries(server:ServerState,prefs:ClientPreferences):List<SettingEntry> = buildList {
    add(SettingEntry("nickname","Account","Display name","Your nickname on ${server.name}","profile name username"))
    add(SettingEntry("status","Account","Status message","A short message on your profile","profile bio"))
    add(SettingEntry("about","Account","About you","Your profile description","bio biography"))
    add(SettingEntry("avatar","Account","Change photo","Your profile picture","avatar image"))
    if(!server.isDemo) add(SettingEntry("banner","Account","Change banner","Your profile’s cover image","photo picture"))
    if(server.profiles.firstOrNull {it.id==server.me}?.banner?.isNotBlank()==true) add(SettingEntry("remove-banner","Account","Remove banner","Clear your profile’s cover image"))
    add(SettingEntry("presence","Account","Presence","Online, away or do not disturb","busy status"))
    add(SettingEntry("typing","Account","Typing phrase","Your custom typing indicator","is typing status"))
    if(!server.isDemo) {
        add(SettingEntry("password","Account","Change password","Choose a new password for this server","security login"))
        add(SettingEntry("logout","Account","Sign out","End this account’s session on ${server.name}","logout security"))
        add(SettingEntry("sessions","Sessions","Active sessions","Review and end signed-in sessions","devices connections security"))
    }
    add(SettingEntry("theme","Appearance","Theme",prefs.theme,"light dark system colors"))
    add(SettingEntry("glass","Appearance","Frosted controls",if(prefs.glass) "On · glass on floating bars and pills" else "Off · opaque bars and pills","translucency transparency blur"))
    appearanceSwitches.forEach {item ->add(SettingEntry(item.key,"Appearance",item.title,"${if(prefs.boolean(item.key,item.default)) "On" else "Off"} · ${item.summary}"))}
    add(SettingEntry("download-folder","Files","Save media to",prefs.string("download-label").ifBlank {"Downloads/Papo"},"download downloads folder files storage location"))
    if(prefs.string("download-tree").isNotBlank()) add(SettingEntry("download-reset","Files","Use Downloads/Papo","Return to the default Android download folder","save location reset"))
    if(!server.isDemo) add(SettingEntry("reconnect","Diagnostics","Reconnect","Refresh the connection and server state","offline network"))
    add(SettingEntry("cache","Diagnostics","Clear media cache","Remove reusable images, audio and video","storage space disk GIF sticker avatar"))
    add(SettingEntry("version","Diagnostics","Papo for Android",BuildConfig.VERSION_NAME,"about version build"))
    add(SettingEntry("android","Diagnostics","Android app settings","Permissions, storage and system preferences","system"))
}

/** Targets share the same controls as ordinary navigation. Bring the exact row
 * into view after the lazy list composes it, then briefly mark the result. */
@Composable
private fun settingTarget(id:String):Modifier {
    val selected=id.isNotBlank() && LocalSettingTarget.current==id
    val requester=remember {BringIntoViewRequester()}
    var highlighted by remember {mutableStateOf(false)}
    LaunchedEffect(selected) {if(selected) {withFrameNanos {};requester.bringIntoView();highlighted=true;kotlinx.coroutines.delay(2200);highlighted=false}}
    return Modifier.bringIntoViewRequester(requester).background(if(highlighted) MaterialTheme.colorScheme.primaryContainer else Color.Transparent,RoundedCornerShape(14.dp))
}

@Composable
internal fun ClientSettings(server:ServerState,model:PapoViewModel,onBack:()->Unit,onLogout:()->Unit) {
    var page by rememberSaveable(server.url,server.me) {mutableStateOf("Settings")}
    var query by rememberSaveable {mutableStateOf("")}
    var target by rememberSaveable {mutableStateOf("")}
    val prefs=LocalClientPreferences.current
    val focus=LocalFocusManager.current
    val keyboard=LocalSoftwareKeyboardController.current
    fun open(destination:String,id:String="") {focus.clearFocus();keyboard?.hide();target=id;page=destination}
    val snackbar=remember {SnackbarHostState()}
    val scope=rememberCoroutineScope()
    val feedback:(String)->Unit={message ->scope.launch {snackbar.showSnackbar(message)}}
    val back:()->Unit={if(page=="Settings") onBack() else page="Settings"}
    BackHandler(page!="Settings" && page!="Account") {back()}
    CompositionLocalProvider(LocalSettingTarget provides target) {
    Box(Modifier.fillMaxSize().testTag("client-settings")) {
        if(page=="Account") AccountSettings(server,model,back,onLogout,feedback)
        else Column(Modifier.fillMaxSize()) {
            SettingsHeader(page,back)
            when(page) {
                "Settings" -> LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current),contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(16.dp)) {
                    item {OutlinedTextField(query,{query=it},Modifier.fillMaxWidth().testTag("settings-search"),placeholder={Text("Search settings")},leadingIcon={Icon(Icons.Outlined.Search,null)},trailingIcon={if(query.isNotBlank()) IconButton(onClick={query=""}) {Icon(Icons.Outlined.Close,"Clear settings search")}},singleLine=true,shape=RoundedCornerShape(28.dp))}
                    if(query.isBlank()) {
                        item {SettingsCard {SettingsLink(server.myName,"Profile, presence and security",Icons.Outlined.Person,{open("Account")},leading={UserAvatar(server.me,server.myName)})}}
                        items(settingsDestinations.filter {!server.isDemo || it.title!="Sessions"},key={it.title}) {entry ->SettingsCard {SettingsLink(entry.title,entry.subtitle,entry.icon,{open(entry.title)})}}
                    } else {
                        val words=searchFold(query).trim().split(Regex("\\s+")).filter {it.isNotBlank()}
                        val matches=settingsEntries(server,prefs).filter {entry ->val haystack=searchFold("${entry.title} ${entry.summary} ${entry.page} ${entry.keywords}");words.all {haystack.contains(it)}}
                        items(matches,key={it.id}) {entry ->SettingsCard {
                            val icon=if(entry.page=="Account") Icons.Outlined.Person else settingsDestinations.first {it.title==entry.page}.icon
                            SettingsLink(entry.title,"${entry.page} · ${entry.summary}",icon,{open(entry.page,entry.id)})
                        }}
                        if(matches.isEmpty()) item {Text("No matching settings",Modifier.padding(16.dp),color=MaterialTheme.colorScheme.onSurfaceVariant)}
                    }
                    item {Text("Account settings apply to ${server.name}. Appearance and files are saved on this device.",Modifier.padding(horizontal=8.dp),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}
                    if(server.isDemo) item {TextButton(onClick=onLogout) {Icon(Icons.Outlined.Logout,null);Spacer(Modifier.width(8.dp));Text("Leave demo")}}
                }
                "Appearance" -> AppearanceSettings()
                "Files" -> FileSettings(feedback)
                "Sessions" -> SessionSettings(server,model,feedback)
                "Diagnostics" -> DiagnosticSettings(server,model,feedback)
            }
        }
        SnackbarHost(snackbar,Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(12.dp))
    }
}
}

@Composable
private fun SettingsHeader(title:String,onBack:()->Unit,actions:@Composable RowScope.()->Unit={}) {
    Row(Modifier.fillMaxWidth().statusBarsPadding().padding(horizontal=12.dp,vertical=8.dp),verticalAlignment=Alignment.CenterVertically) {
        GlassPill {IconButton(onClick=onBack) {Icon(Icons.Outlined.ArrowBack,"Back from $title")}}
        Text(title,Modifier.weight(1f).padding(horizontal=16.dp),style=MaterialTheme.typography.headlineSmall,fontWeight=FontWeight.SemiBold)
        actions()
    }
}
@Composable
private fun SettingsCard(content:@Composable ColumnScope.()->Unit) {
    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(24.dp)).background(MaterialTheme.colorScheme.surfaceContainerLow),content=content)
}
@Composable
private fun SettingsLink(title:String,subtitle:String,icon:ImageVector,onClick:()->Unit,leading:(@Composable ()->Unit)?=null,target:String="") {
    ListItem(headlineContent={Text(title,fontWeight=FontWeight.Medium)},supportingContent={Text(subtitle)},leadingContent={if(leading!=null) leading() else Surface(shape=RoundedCornerShape(14.dp),color=MaterialTheme.colorScheme.surfaceContainerHigh) {Icon(icon,null,Modifier.padding(12.dp))}},trailingContent={Icon(Icons.Outlined.ChevronRight,null)},colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.then(settingTarget(target)).clickable(onClick=onClick).heightIn(min=80.dp))
}
@Composable
private fun SettingsLabel(text:String) {Text(text,Modifier.padding(start=12.dp,top=8.dp),style=MaterialTheme.typography.labelLarge,color=MaterialTheme.colorScheme.primary)}
@Composable
private fun SettingsSwitch(title:String,subtitle:String,value:Boolean,id:String,onChange:(Boolean)->Unit) {
    ListItem(headlineContent={Text(title)},supportingContent={Text(subtitle)},trailingContent={Switch(value,onChange,Modifier.semantics {contentDescription=title})},colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.then(settingTarget(id)).heightIn(min=72.dp))
}
@Composable
private fun SettingsDivider() {HorizontalDivider(Modifier.padding(horizontal=16.dp),color=MaterialTheme.colorScheme.outlineVariant.copy(alpha=.5f))}

@Composable
private fun AppearanceSettings() {
    val prefs=LocalClientPreferences.current
    val target=LocalSettingTarget.current
    val list=rememberLazyListState()
    val order=listOf("preview","preview-note","theme-label","theme","glass","conversation-label")+appearanceSwitches.map {it.key}+"text-size"
    LaunchedEffect(target) {val index=order.indexOf(target);if(index>=0) list.scrollToItem(index)}
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("appearance-settings"),state=list,contentPadding=PaddingValues(20.dp,8.dp,20.dp,40.dp),verticalArrangement=Arrangement.spacedBy(6.dp)) {
        item("preview") {SettingsCard {
            Column(Modifier.padding(20.dp),verticalArrangement=Arrangement.spacedBy(if(prefs.compact) 6.dp else 14.dp)) {
                Text("# design-ui",style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
                Row(horizontalArrangement=Arrangement.spacedBy(12.dp),verticalAlignment=Alignment.Top) {
                    if(prefs.avatars) Avatar("Ana")
                    Column(verticalArrangement=Arrangement.spacedBy(4.dp)) {
                        Row(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalAlignment=Alignment.CenterVertically) {Text("Ana",fontWeight=FontWeight.SemiBold);if(prefs.timestamps) Text("14:02",style=MaterialTheme.typography.labelSmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}
                        Text("A little more like you.")
                    }
                }
                GlassPill {Text("Message #design-ui",Modifier.padding(14.dp),color=MaterialTheme.colorScheme.onSurfaceVariant)}
            }
        }}
        item("preview-note") {Text("A live preview of your theme and conversation layout.",Modifier.padding(horizontal=8.dp,vertical=8.dp),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}
        item("theme-label") {SettingsLabel("Theme")}
        item("theme") {SettingsCard {
            Column(Modifier.then(settingTarget("theme")).padding(16.dp)) {Text("Theme",style=MaterialTheme.typography.titleMedium);FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {listOf("System","Light","Dark").forEach {choice ->FilterChip(prefs.theme==choice,{prefs.set("theme",choice)},label={Text(choice)})}}}
        }}
        item("glass") {SettingsCard {SettingsSwitch("Frosted controls","Blur the content behind floating bars and pills",prefs.glass,"glass") {prefs.set("glass",it)}}}
        item("conversation-label") {SettingsLabel("Conversations")}
        items(appearanceSwitches,key={it.key}) {entry ->SettingsCard {
            SettingsSwitch(entry.title,entry.summary,prefs.boolean(entry.key,entry.default),entry.key) {prefs.set(entry.key,it)}
        }}
        item("text-size") {Text("Text size follows Android’s font and display settings.",Modifier.padding(8.dp),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}
    }
}

@Composable
private fun AccountSettings(server:ServerState,model:PapoViewModel,onBack:()->Unit,onLogout:()->Unit,feedback:(String)->Unit) {
    val scope=rememberCoroutineScope()
    var loaded by remember(server.url,server.me) {mutableStateOf<AccountProfile?>(null)}
    var loading by remember {mutableStateOf(true)}
    var error by remember {mutableStateOf("")}
    var busy by remember {mutableStateOf(false)}
    var nickname by rememberSaveable(server.url,server.me) {mutableStateOf<String?>(null)}
    var status by rememberSaveable(server.url,server.me) {mutableStateOf<String?>(null)}
    var about by rememberSaveable(server.url,server.me) {mutableStateOf<String?>(null)}
    var typing by rememberSaveable(server.url,server.me) {mutableStateOf<String?>(null)}
    var discard by remember {mutableStateOf(false)}
    var logout by remember {mutableStateOf(false)}
    var passwordDialog by remember {mutableStateOf(false)}
    var password by remember {mutableStateOf("")}
    var repeatPassword by remember {mutableStateOf("")}
    val target=LocalSettingTarget.current
    val list=rememberLazyListState()
    LaunchedEffect(target,loaded!=null,loading,error) {
        val base=2+(if(loading) 1 else 0)+(if(error.isNotBlank()) 1 else 0)
        val index=when(target) {"avatar","banner","remove-banner"->1;"nickname"->base+1;"status"->base+2;"about"->base+3;"presence"->base+5;"typing"->base+7;"password","logout"->base+10;else->-1}
        if(index>=0 && (loaded!=null || index==1)) list.scrollToItem(index)
    }
    val dirty=loaded?.let {nickname!=null && nickname!=it.nickname || status!=null && status!=it.statusMessage || about!=null && about!=it.description || typing!=null && typing!=it.typing} ?: false
    val back:()->Unit={if(!busy) {if(dirty) discard=true else onBack()}}
    BackHandler {back()}
    suspend fun load() {
        loading=true;error=""
        try {loaded=model.accountProfile(server)} catch(e:CancellationException) {throw e} catch(e:Exception) {error=e.message ?: "Couldn’t load your profile"} finally {loading=false}
    }
    LaunchedEffect(server.url,server.me) {load()}
    fun change(change:AccountChange,message:String,clearDraft:Boolean=false) {
        if(busy) return
        busy=true;error=""
        scope.launch {
            try {
                model.accountChange(server,change)
                if(clearDraft && change is AccountChange.Profile) {
                    loaded=loaded?.copy(nickname=change.nickname,statusMessage=change.statusMessage,description=change.description,typing=change.typing)
                    nickname=null;status=null;about=null;typing=null
                }
                if(change is AccountChange.Presence) loaded=loaded?.copy(presence=change.value)
                password="";repeatPassword="";passwordDialog=false
                feedback(message)
                // A failed read after an acknowledged save is reported as a
                // refresh failure, without prompting another mutation.
                try {loaded=model.accountProfile(server)} catch(e:CancellationException) {throw e} catch(e:Exception) {error="Saved, but couldn’t refresh the profile. ${e.message.orEmpty()}"}
            } catch(e:CancellationException) {throw e} catch(e:Exception) {error=e.message ?: "Couldn’t save changes"}
            finally {busy=false}
        }
    }
    var imageTarget by remember {mutableStateOf("avatar")}
    val context=LocalContext.current
    val imagePicker=rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) {uri ->
        if(uri!=null && !busy) {
            busy=true
            scope.launch {
                try {
                    val image=readProfileImage(context,uri)
                    model.accountChange(server,if(imageTarget=="avatar") AccountChange.Avatar(image.first,image.second) else AccountChange.Banner(image.first,image.second))
                    feedback(if(imageTarget=="avatar") "Profile photo updated" else "Profile banner updated")
                    try {loaded=model.accountProfile(server)} catch(e:CancellationException) {throw e} catch(e:Exception) {error="Image saved, but couldn’t refresh your profile. ${e.message.orEmpty()}"}
                } catch(e:CancellationException) {throw e} catch(e:Exception) {error=e.message ?: "Couldn’t update the image"}
                finally {busy=false}
            }
        }
    }
    Column(Modifier.fillMaxSize()) {
        SettingsHeader("Account",back) {
            if(busy) CircularProgressIndicator(Modifier.padding(12.dp).size(24.dp),strokeWidth=2.dp)
            else FilledTonalButton(enabled=dirty && loaded!=null,onClick={loaded?.let {p ->change(AccountChange.Profile(nickname ?: p.nickname,status ?: p.statusMessage,about ?: p.description,typing ?: p.typing),"Profile saved",true)}},modifier=Modifier.testTag("save-account")) {Text("Save")}
        }
        LazyColumn(Modifier.fillMaxSize().imePadding().hazeSource(LocalHaze.current).testTag("account-settings"),state=list,contentPadding=PaddingValues(20.dp,8.dp,20.dp,40.dp),verticalArrangement=Arrangement.spacedBy(14.dp)) {
            item {
                SettingsCard {
                    val profile=server.profiles.firstOrNull {it.id==server.me}
                    Box(Modifier.fillMaxWidth()) {
                        ProfileBanner(server,model,server.me,profile?.banner.orEmpty(),Modifier.fillMaxWidth().height(120.dp))
                        UserAvatar(server.me,server.myName,Modifier.padding(start=20.dp,top=84.dp).size(80.dp))
                    }
                    Column(Modifier.padding(20.dp),verticalArrangement=Arrangement.spacedBy(6.dp)) {
                        Text(nickname ?: loaded?.nickname ?: server.myName,style=MaterialTheme.typography.titleLarge,fontWeight=FontWeight.SemiBold)
                        Text("@${loaded?.username ?: server.members.firstOrNull {it.id==server.me}?.username.orEmpty()}",color=MaterialTheme.colorScheme.onSurfaceVariant)
                        val message=status ?: loaded?.statusMessage.orEmpty()
                        if(message.isNotBlank()) Text(message,style=MaterialTheme.typography.bodyMedium,maxLines=3,overflow=androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                    }
                }
            }
            item {FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                OutlinedButton(modifier=Modifier.then(settingTarget("avatar")),enabled=!busy && loaded!=null,onClick={imageTarget="avatar";imagePicker.launch(arrayOf("image/png","image/jpeg","image/gif","image/webp"))}) {Text("Change photo")}
                if(!server.isDemo) OutlinedButton(modifier=Modifier.then(settingTarget("banner")),enabled=!busy && loaded!=null,onClick={imageTarget="banner";imagePicker.launch(arrayOf("image/png","image/jpeg","image/gif","image/webp"))}) {Text("Change banner")}
                if(server.profiles.firstOrNull {it.id==server.me}?.banner?.isNotBlank()==true) TextButton(modifier=Modifier.then(settingTarget("remove-banner")),enabled=!busy,onClick={change(AccountChange.Banner("",""),"Banner removed")}) {Text("Remove banner")}
            }}
            if(loading) item {LinearProgressIndicator(Modifier.fillMaxWidth().semantics {contentDescription="Loading account"})}
            if(error.isNotBlank()) item {SettingsError(error,if(loaded==null) ({scope.launch {load()}}) else null)}
            if(loaded!=null) {
                item {SettingsLabel("Profile on ${server.name}")}
                item {OutlinedTextField(nickname ?: loaded!!.nickname,{nickname=it},Modifier.fillMaxWidth().then(settingTarget("nickname")).testTag("account-nickname"),label={Text("Display name")},singleLine=true,enabled=!busy)}
                item {OutlinedTextField(status ?: loaded!!.statusMessage,{status=it},Modifier.fillMaxWidth().then(settingTarget("status")).testTag("account-status"),label={Text("Status message")},maxLines=3,enabled=!busy)}
                item {OutlinedTextField(about ?: loaded!!.description,{about=it},Modifier.fillMaxWidth().then(settingTarget("about")).testTag("account-about"),label={Text("About you")},minLines=3,maxLines=6,enabled=!busy)}
                item {SettingsLabel("Presence")}
                item {SettingsCard {Column(Modifier.then(settingTarget("presence")).padding(16.dp)) {
                    FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {listOf("online" to "Online","away" to "Away","busy" to "Do not disturb").forEach {(value,label) ->FilterChip(loaded!!.presence==value,{change(AccountChange.Presence(value),"Presence updated")},label={Text(label)},enabled=!busy)}}
                    Text("Presence is shared with this server. It doesn’t change Android’s notification settings.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                }}}
                item {SettingsLabel("When you type")}
                item {OutlinedTextField(typing ?: loaded!!.typing,{typing=it},Modifier.fillMaxWidth().then(settingTarget("typing")).testTag("account-typing"),label={Text("Typing phrase")},supportingText={Text("Leave empty to use the default phrase.")},singleLine=true,enabled=!busy)}
                item {SettingsCard {Text("•••  ${nickname ?: loaded!!.nickname} ${ (typing ?: loaded!!.typing).ifBlank {"is typing…"} }",Modifier.padding(16.dp),style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)}}
            }
            if(!server.isDemo) {
                item {SettingsLabel("Security")}
                item {SettingsCard {
                    SettingsLink("Change password","Choose a new password for this server",Icons.Outlined.Lock,{if(!busy) passwordDialog=true},target="password")
                    SettingsDivider()
                    SettingsLink("Sign out","End this account’s session on ${server.name}",Icons.Outlined.Logout,{if(!busy) logout=true},target="logout")
                }}
            }
        }
    }
    if(discard) AlertDialog(onDismissRequest={discard=false},title={Text("Discard profile changes?")},text={Text("Your unsaved edits will be lost.")},confirmButton={TextButton(onClick={discard=false;onBack()}) {Text("Discard")}},dismissButton={TextButton(onClick={discard=false}) {Text("Keep editing")}})
    if(logout) AlertDialog(onDismissRequest={logout=false},title={Text("Sign out?")},text={Text("You’ll need to sign in again on ${server.name}.")},confirmButton={TextButton(onClick={logout=false;onLogout()}) {Text("Sign out")}},dismissButton={TextButton(onClick={logout=false}) {Text("Cancel")}})
    if(passwordDialog) AlertDialog(onDismissRequest={if(!busy) {passwordDialog=false;password="";repeatPassword=""}},title={Text("Change password")},text={Column(verticalArrangement=Arrangement.spacedBy(12.dp)) {
        Text("Use at least eight characters. This changes your password on ${server.name}.")
        OutlinedTextField(password,{password=it},label={Text("New password")},singleLine=true,visualTransformation=PasswordVisualTransformation(),keyboardOptions=KeyboardOptions(keyboardType=KeyboardType.Password),enabled=!busy)
        OutlinedTextField(repeatPassword,{repeatPassword=it},label={Text("Repeat password")},singleLine=true,visualTransformation=PasswordVisualTransformation(),keyboardOptions=KeyboardOptions(keyboardType=KeyboardType.Password),enabled=!busy)
        if(error.isNotBlank()) Text(error,color=MaterialTheme.colorScheme.error)
    }},confirmButton={TextButton(enabled=!busy && password.codePointCount(0,password.length)>=8 && password==repeatPassword,onClick={change(AccountChange.Password(password),"Password changed")}) {Text(if(busy) "Changing…" else "Change password")}},dismissButton={TextButton(enabled=!busy,onClick={passwordDialog=false;password="";repeatPassword=""}) {Text("Cancel")}})
}

/** Decode bounds and the actual image format before uploading; never trust a
 * provider's filename or MIME type, and never materialize unbounded input. */
internal suspend fun readProfileImage(context:android.content.Context,uri:Uri):Pair<String,String> = withContext(Dispatchers.IO) {
    val bytes=requireNotNull(context.contentResolver.openInputStream(uri)).use {input ->
        val output=java.io.ByteArrayOutputStream();val buffer=ByteArray(8192)
        while(true) {val count=input.read(buffer);if(count<0) break;require(output.size()+count<=8*1024*1024) {"Choose an image up to 8 MB"};output.write(buffer,0,count)}
        output.toByteArray()}
    require(bytes.size in 1..8*1024*1024) {"Choose an image up to 8 MB"}
    val bounds=android.graphics.BitmapFactory.Options().apply {inJustDecodeBounds=true}
    android.graphics.BitmapFactory.decodeByteArray(bytes,0,bytes.size,bounds)
    require(bounds.outWidth>0 && bounds.outHeight>0 && bounds.outWidth.toLong()*bounds.outHeight<=40_000_000) {"Choose an image up to 40 megapixels"}
    val format=when(bounds.outMimeType) {"image/png"->"PNG";"image/jpeg"->"JPEG";"image/gif"->"GIF";"image/webp"->"WEBP";else->error("Choose a PNG, JPEG, GIF or WebP image")}
    android.util.Base64.encodeToString(bytes,android.util.Base64.NO_WRAP) to format
}

@Composable
private fun FileSettings(feedback:(String)->Unit) {
    val context=LocalContext.current
    val prefs=LocalClientPreferences.current
    val folder=prefs.string("download-tree")
    val scope=rememberCoroutineScope()
    val picker=rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) {uri ->
        if(uri!=null) scope.launch {
            try {
                val label=withContext(Dispatchers.IO) {
                    context.contentResolver.takePersistableUriPermission(uri,Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
                    val document=DocumentsContract.buildDocumentUriUsingTree(uri,DocumentsContract.getTreeDocumentId(uri))
                    context.contentResolver.query(document,arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME),null,null,null)?.use {cursor ->if(cursor.moveToFirst()) cursor.getString(0) else null} ?: "Selected folder"
                }
                val previous=folder.takeIf {it.isNotBlank()}?.let(Uri::parse)
                prefs.set("download-tree",uri.toString());prefs.set("download-label",label)
                if(previous!=null && previous!=uri) withContext(Dispatchers.IO) {runCatching {context.contentResolver.releasePersistableUriPermission(previous,Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)}}
                feedback("Download folder updated")
            } catch(e:CancellationException) {throw e} catch(e:Exception) {feedback(e.message ?: "Couldn’t use that folder")}
        }
    }
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current),contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        item {SettingsLabel("Downloads")}
        item {SettingsCard {
            SettingsLink("Save media to",downloadLocation(context),Icons.Outlined.Folder,{picker.launch(folder.takeIf {it.isNotBlank()}?.let(Uri::parse))},target="download-folder")
            if(folder.isNotBlank()) {SettingsDivider();SettingsLink("Use Downloads/Papo","Return to the default Android folder",Icons.Outlined.Restore,{
                val old=Uri.parse(folder);prefs.set("download-tree","");prefs.set("download-label","")
                runCatching {context.contentResolver.releasePersistableUriPermission(old,Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)}
                feedback("Downloads will be saved to Downloads/Papo")
            },target="download-reset")}
        }}
        item {Text("Applies to the download button in the shared media viewer. Android manages access to the folder you choose.",Modifier.padding(horizontal=8.dp),style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)}
    }
}
@Composable
private fun SessionSettings(server:ServerState,model:PapoViewModel,feedback:(String)->Unit) {
    val scope=rememberCoroutineScope()
    var sessions by remember {mutableStateOf<List<SessionView>?>(null)}
    var busy by remember {mutableStateOf(false)}
    var error by remember {mutableStateOf("")}
    var ending by remember {mutableStateOf<SessionView?>(null)}
    suspend fun load() {busy=true;error="";try {sessions=model.accountSessions(server)} catch(e:CancellationException) {throw e} catch(e:Exception) {error=e.message ?: "Couldn’t load sessions"} finally {busy=false}}
    LaunchedEffect(server.url,server.me) {load()}
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current),contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        item {Text("Signed in to ${server.name}",Modifier.padding(8.dp),style=MaterialTheme.typography.titleMedium)}
        item {Text("The server provides session IDs and dates, but doesn’t identify devices or which session is this one.",Modifier.padding(8.dp),style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)}
        if(busy) item {LinearProgressIndicator(Modifier.fillMaxWidth())}
        if(error.isNotBlank()) item {SettingsError(error) {scope.launch {load()}}}
        items(sessions.orEmpty(),key={it.id}) {session ->SettingsCard {Column(Modifier.padding(16.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            Text("Session …${session.id.takeLast(8)}",style=MaterialTheme.typography.titleMedium)
            if(session.createdAt.isNotBlank()) Text("Created ${settingsDate(session.createdAt)}",style=MaterialTheme.typography.bodyMedium)
            if(session.expiresAt.isNotBlank()) Text("Expires ${settingsDate(session.expiresAt)}",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
            TextButton(enabled=!busy,onClick={ending=session}) {Text("End session")}
        }}}
        if(sessions?.isEmpty()==true && !busy) item {Text("The server returned no active sessions.",Modifier.padding(16.dp))}
        item {OutlinedButton(enabled=!busy,onClick={scope.launch {load()}}) {Icon(Icons.Outlined.Refresh,null);Spacer(Modifier.width(8.dp));Text("Refresh sessions")}}
    }
    ending?.let {session ->AlertDialog(onDismissRequest={ending=null},title={Text("End this session?")},text={Text("Session …${session.id.takeLast(8)} will be signed out. If it belongs to this app, you’ll need to sign in again.")},confirmButton={TextButton(onClick={ending=null;busy=true;scope.launch {try {model.accountChange(server,AccountChange.RevokeSession(session.id));sessions=sessions.orEmpty().filterNot {it.id==session.id};feedback("Session ended");load()} catch(e:CancellationException) {throw e} catch(e:Exception) {error=e.message ?: "Couldn’t end session"} finally {busy=false}}}) {Text("End session")}},dismissButton={TextButton(onClick={ending=null}) {Text("Cancel")}})}
}
private fun settingsDate(raw:String)=runCatching {OffsetDateTime.parse(raw).atZoneSameInstant(java.time.ZoneId.systemDefault()).format(DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM,FormatStyle.SHORT))}.getOrDefault(raw)

@Composable
private fun DiagnosticSettings(server:ServerState,model:PapoViewModel,feedback:(String)->Unit) {
    val scope=rememberCoroutineScope()
    val target=LocalSettingTarget.current
    val list=rememberLazyListState()
    LaunchedEffect(target) {val index=when(target) {"reconnect"->1;"cache"->3;"version","android"->6;else->-1};if(index>=0) list.scrollToItem(index)}
    var bytes by remember {mutableLongStateOf(model.mediaCache.size)}
    var clearing by remember {mutableStateOf(false)}
    var confirm by remember {mutableStateOf(false)}
    val context=LocalContext.current
    LaunchedEffect(Unit) {while(true) {bytes=model.mediaCache.size;kotlinx.coroutines.delay(2000)}}
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current),state=list,contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        item {SettingsLabel("Connection")}
        item {SettingsCard {
            ListItem(headlineContent={Text(server.name)},supportingContent={Text(if(server.isDemo) "Offline demo · no server connection" else "${server.connection.replaceFirstChar(Char::uppercase)}\n${server.url}")},leadingContent={Icon(Icons.Outlined.Cloud,null)},colors=ListItemDefaults.colors(containerColor=Color.Transparent))
            if(!server.isDemo) {SettingsDivider();SettingsLink("Reconnect","Refresh the connection and server state",Icons.Outlined.Refresh,{model.command(io.github.chriexpe.papo.core.Action.Refresh,server.url)},target="reconnect")}
        }}
        item {SettingsLabel("Media cache")}
        item {SettingsCard {
            ListItem(headlineContent={Text(android.text.format.Formatter.formatFileSize(context,bytes))},supportingContent={Text("256 MB shared limit · oldest media is removed first")},leadingContent={Icon(Icons.Outlined.Storage,null)},colors=ListItemDefaults.colors(containerColor=Color.Transparent))
            SettingsDivider()
            ListItem(headlineContent={Text("Clear media cache")},supportingContent={Text("Images, avatars, stickers, audio and video can be fetched again")},trailingContent={if(clearing) CircularProgressIndicator(Modifier.size(24.dp)) else Icon(Icons.Outlined.DeleteOutline,null)},colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.then(settingTarget("cache")).clickable(enabled=!clearing) {confirm=true})
        }}
        item {Text("Saved downloads, favorite GIF copies, messages, drafts and sign-ins are kept.",Modifier.padding(horizontal=8.dp),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}
        item {SettingsLabel("About")}
        item {SettingsCard {
            ListItem(modifier=Modifier.then(settingTarget("version")),headlineContent={Text("Papo for Android")},supportingContent={Text("${BuildConfig.VERSION_NAME} · Native Compose · papo-core")},leadingContent={Icon(Icons.Outlined.Forum,null)},colors=ListItemDefaults.colors(containerColor=Color.Transparent))
            SettingsDivider()
            ListItem(headlineContent={Text("Android ${Build.VERSION.RELEASE}")},supportingContent={Text("API ${Build.VERSION.SDK_INT}")},leadingContent={Icon(Icons.Outlined.Android,null)},colors=ListItemDefaults.colors(containerColor=Color.Transparent))
            SettingsDivider()
            SettingsLink("Android app settings","Permissions, storage and system preferences",Icons.Outlined.Settings,{try {context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS,Uri.parse("package:${context.packageName}")))} catch(e:Exception) {feedback(e.message ?: "Couldn’t open Android settings")}},target="android")
        }}
    }
    if(confirm) AlertDialog(onDismissRequest={confirm=false},title={Text("Clear cached media?")},text={Text("Media will need to download again. Saved files and account data are kept.")},confirmButton={TextButton(onClick={confirm=false;clearing=true;scope.launch {try {model.mediaCache.clear();MediaImages.loader(context).memoryCache?.clear();bytes=model.mediaCache.size;feedback("Media cache cleared")} catch(e:CancellationException) {throw e} catch(e:Exception) {feedback(e.message ?: "Couldn’t clear cache")} finally {clearing=false}}}) {Text("Clear cache")}},dismissButton={TextButton(onClick={confirm=false}) {Text("Cancel")}})
}
@Composable
private fun SettingsError(message:String,retry:(()->Unit)?=null) {
    Surface(color=MaterialTheme.colorScheme.errorContainer,shape=RoundedCornerShape(18.dp),modifier=Modifier.fillMaxWidth().semantics {liveRegion=LiveRegionMode.Polite}) {
        Column(Modifier.padding(16.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {Text(message,color=MaterialTheme.colorScheme.onErrorContainer,style=MaterialTheme.typography.bodyMedium);if(retry!=null) TextButton(onClick=retry) {Text("Try again")}}
    }
}
