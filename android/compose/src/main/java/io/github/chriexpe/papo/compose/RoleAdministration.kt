package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.*

private fun roleColor(value:String)=runCatching {Color(android.graphics.Color.parseColor(value))}.getOrDefault(Color.Gray)

@Composable
internal fun RoleAdministrationList(server:ServerState,ui:AdministrationUi,open:(String,String)->Unit) {
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-roles"),contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        item {Button(enabled=!ui.busy,onClick={open("New role","")}) {Icon(Icons.Outlined.Add,null);Text("Create role")}}
        if(server.roles.isEmpty()) item {AdministrationEmpty("No roles yet. Roles control permissions and nickname colors.")}
        items(server.roles,key={it.id}) {role ->SettingsCard {SettingsLink(role.name,administrationPeople(ui.data?.people.orEmpty().count {role.id in it.roles}),Icons.Outlined.Badge,{open("Role/${role.id}","")},leading={Box(Modifier.size(20.dp).background(roleColor(role.color),CircleShape))})}}
    }
}

@Composable
internal fun RoleAdministrationEditor(server:ServerState,ui:AdministrationUi,id:String?,onDirty:(Boolean)->Unit,onSaved:()->Unit,open:(String,String)->Unit) {
    val existing=server.roles.find {it.id==id}
    if(id!=null && existing==null) {AdministrationEmpty("This role no longer exists.");return}
    var name by rememberSaveable(id) {mutableStateOf(existing?.name.orEmpty())}
    var color by rememberSaveable(id) {mutableStateOf(existing?.color.orEmpty())}
    val initial=existing?.permissions ?: Capabilities(false,false,false,false,false,false,false)
    var manageServer by rememberSaveable(id) {mutableStateOf(initial.manageServer)}
    var manageChannels by rememberSaveable(id) {mutableStateOf(initial.manageChannels)}
    var manageRoles by rememberSaveable(id) {mutableStateOf(initial.manageRoles)}
    var banMembers by rememberSaveable(id) {mutableStateOf(initial.banMembers)}
    var pinMessage by rememberSaveable(id) {mutableStateOf(initial.pinMessage)}
    var everyoneMessage by rememberSaveable(id) {mutableStateOf(initial.everyoneMessage)}
    var sendAttachment by rememberSaveable(id) {mutableStateOf(initial.sendAttachment)}
    var delete by remember {mutableStateOf(false)}
    val permissions=Capabilities(manageServer,manageChannels,manageRoles,banMembers,pinMessage,everyoneMessage,sendAttachment)
    val changed=name!=existing?.name.orEmpty() || color!=existing?.color.orEmpty() || permissions!=initial
    SideEffect {onDirty(changed)}
    val colorValid=color.isEmpty() || Regex("#[0-9a-fA-F]{6}").matches(color)
    val valid=name.trim().isNotEmpty() && name.codePointCount(0,name.length)<=32 && colorValid
    val target=LocalSettingTarget.current
    val list=rememberLazyListState()
    LaunchedEffect(target) {if(target.startsWith("role-") && target!="role-name") list.scrollToItem(when(target) {"role-color"->1;"role-delete"->5;else->2})}
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-role-editor"),state=list,contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        item {OutlinedTextField(name,{name=it},Modifier.fillMaxWidth().then(settingTarget("role-name")).testTag("admin-role-name"),label={Text("Role name")},singleLine=true,enabled=!ui.busy,isError=name.isNotEmpty() && name.codePointCount(0,name.length)>32)}
        item {SettingsCard {Column(Modifier.padding(16.dp).then(settingTarget("role-color")),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            SettingsLabel("Color")
            FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {listOf("#FF6338","#F6BB00","#2CB34A","#32BDF1","#168BFF","#8454FF","#F24691","#99999F").forEach {value ->FilterChip(color.equals(value,true),{color=value},label={Box(Modifier.size(22.dp).background(roleColor(value),CircleShape))},enabled=!ui.busy,modifier=Modifier.semantics {contentDescription="Color $value"})};FilterChip(color.isEmpty(),{color=""},label={Text("Default")},enabled=!ui.busy)}
            OutlinedTextField(color,{color=it},Modifier.fillMaxWidth(),label={Text("Hex color")},placeholder={Text("#RRGGBB")},singleLine=true,enabled=!ui.busy,isError=!colorValid)
        }}}
        item {SettingsCard {Column(Modifier.padding(horizontal=16.dp).then(settingTarget("role-permissions"))) {
            SettingsLabel("Permissions")
            AdministrationToggle("Manage server",manageServer,!ui.busy,"role-manage-server") {manageServer=it}
            AdministrationToggle("Manage channels",manageChannels,!ui.busy,"role-manage-channels") {manageChannels=it}
            AdministrationToggle("Manage roles",manageRoles,!ui.busy,"role-manage-roles") {manageRoles=it}
            AdministrationToggle("Ban people",banMembers,!ui.busy,"role-ban-members") {banMembers=it}
            AdministrationToggle("Pin messages",pinMessage,!ui.busy,"role-pin-message") {pinMessage=it}
            AdministrationToggle("Mention everyone",everyoneMessage,!ui.busy,"role-everyone-message") {everyoneMessage=it}
            AdministrationToggle("Send attachments",sendAttachment,!ui.busy,"role-send-attachment") {sendAttachment=it}
        }}}
        item {Button(enabled=valid && !ui.busy && (id==null || changed),onClick={ui.mutate(if(id==null) AdministrationChange.CreateRole(name,color,permissions) else AdministrationChange.UpdateRole(id,name,color,permissions),if(id==null) "Role created" else "Role saved") {onSaved()}},modifier=Modifier.fillMaxWidth().heightIn(min=48.dp).testTag("save-role")) {Text(if(id==null) "Create role" else "Save role")}}
        if(id!=null) {
            item {SettingsCard {SettingsLink("People with this role",administrationPeople(ui.data?.people.orEmpty().count {id in it.roles}),Icons.Outlined.PeopleOutline,{open("RolePeople/$id","role-members")})}}
            item {OutlinedButton(enabled=!ui.busy,onClick={delete=true},modifier=Modifier.fillMaxWidth().heightIn(min=48.dp).then(settingTarget("role-delete"))) {Text("Delete role",color=MaterialTheme.colorScheme.error)}}
        }
    }
    if(delete && id!=null) AdministrationDelete("Delete $name?","This removes the role from everyone and deletes its channel rules.",ui.busy,{delete=false}) {ui.mutate(AdministrationChange.DeleteRole(id),"Role deleted") {delete=false;onSaved()}}
}

@Composable
internal fun RoleMembershipAdministration(server:ServerState,ui:AdministrationUi,id:String) {
    val role=server.roles.find {it.id==id}
    var query by rememberSaveable {mutableStateOf("")}
    if(role==null) {AdministrationEmpty("This role no longer exists.");return}
    val people=ui.data?.people.orEmpty().filter {searchFold("${it.name} ${it.username}").contains(searchFold(query))}.sortedWith(compareByDescending<ManagedPerson> {id in it.roles}.thenBy {it.name.lowercase()})
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-role-people"),contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        item {Text(role.name,style=MaterialTheme.typography.titleLarge);OutlinedTextField(query,{query=it},Modifier.fillMaxWidth().padding(top=12.dp),label={Text("Search people")},singleLine=true,leadingIcon={Icon(Icons.Outlined.Search,null)})}
        if(people.isEmpty() && !ui.loading) item {AdministrationEmpty("No matching people")}
        items(people,key={it.id}) {person ->SettingsCard {
            ListItem(headlineContent={Text(person.name)},supportingContent={Text("@${person.username}${if(person.banned) " · Banned" else ""}")},leadingContent={UserAvatar(person.id,person.name)},trailingContent={Switch(id in person.roles,{assigned ->ui.mutate(AdministrationChange.AssignRole(person.id,id,assigned),if(assigned) "Role assigned" else "Role removed")},modifier=Modifier.semantics {contentDescription="Assign ${role.name} to ${person.name}"},enabled=!ui.busy)},colors=ListItemDefaults.colors(containerColor=Color.Transparent))
        }}
    }
}

@Composable
internal fun PeopleAdministration(server:ServerState,ui:AdministrationUi) {
    var query by rememberSaveable {mutableStateOf("")}
    var selected by remember {mutableStateOf<ManagedPerson?>(null)}
    val people=ui.data?.people.orEmpty().filter {searchFold("${it.name} ${it.username} ${it.id}").contains(searchFold(query))}
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-people"),contentPadding=PaddingValues(20.dp,8.dp,20.dp,32.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        item {OutlinedTextField(query,{query=it},Modifier.fillMaxWidth(),label={Text("Search people")},singleLine=true,leadingIcon={Icon(Icons.Outlined.Search,null)})}
        if(people.isEmpty() && !ui.loading) item {AdministrationEmpty("No matching people")}
        items(people,key={it.id}) {person ->SettingsCard {ListItem(headlineContent={Text(person.name)},supportingContent={Text("@${person.username}${if(person.owner) " · Owner" else if(person.banned) " · Banned" else ""}")},leadingContent={UserAvatar(person.id,person.name)},trailingContent={if(!person.owner && person.id!=server.me) TextButton(enabled=!ui.busy,onClick={selected=person}) {Text(if(person.banned) "Unban" else "Ban",color=if(person.banned) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.error)}},colors=ListItemDefaults.colors(containerColor=Color.Transparent))}}
    }
    selected?.let {person ->AlertDialog(onDismissRequest={if(!ui.busy) selected=null},title={Text("${if(person.banned) "Unban" else "Ban"} ${person.name}?")},text={Text(if(person.banned) "Restore this person’s server access." else "This person will lose access to this server. Their messages remain in the conversation.")},confirmButton={TextButton(enabled=!ui.busy,onClick={ui.mutate(AdministrationChange.Ban(person.id,!person.banned),if(person.banned) "Person unbanned" else "Person banned") {selected=null}}) {Text(if(person.banned) "Unban" else "Ban")}},dismissButton={TextButton(enabled=!ui.busy,onClick={selected=null}) {Text("Cancel")}})}
}
