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
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.*

private fun roleColor(value: String) = runCatching {
    Color(android.graphics.Color.parseColor(value))
}
    .getOrDefault(Color.Gray)

@Composable
internal fun RoleAdministrationList(
    server: ServerState,
    ui: AdministrationUi,
    open: (String, String) -> Unit,
) {
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-roles"),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            Button(enabled = !ui.busy, onClick = { open("New role", "") }) {
                Icon(Icons.Outlined.Add, null)
                Text(ui(R.string.create_role))
            }
        }
        if (server.roles.isEmpty())
            item {
                AdministrationEmpty(
                    ui(R.string.no_roles_yet_roles_control_permissions_and_nickname_colors)
                )
            }
        items(server.roles, key = { it.id }) { role ->
            SettingsCard {
                SettingsLink(
                    role.name,
                    administrationPeople(ui.data?.people.orEmpty().count { role.id in it.roles }),
                    Icons.Outlined.Badge,
                    { open("Role/${role.id}", "") },
                    leading = {
                        Box(Modifier.size(20.dp).background(roleColor(role.color), CircleShape))
                    },
                )
            }
        }
    }
}

@Composable
internal fun RoleAdministrationEditor(
    server: ServerState,
    ui: AdministrationUi,
    id: String?,
    onDirty: (Boolean) -> Unit,
    onSaved: () -> Unit,
    open: (String, String) -> Unit,
) {
    val existing = server.roles.find { it.id == id }
    if (id != null && existing == null) {
        AdministrationEmpty(ui(R.string.this_role_no_longer_exists))
        return
    }
    var name by rememberSaveable(id) { mutableStateOf(existing?.name.orEmpty()) }
    var color by rememberSaveable(id) { mutableStateOf(existing?.color.orEmpty()) }
    val initial =
        existing?.permissions ?: Capabilities(false, false, false, false, false, false, false)
    // Seed once per role. Live server updates affect the dirty comparison, not the user's draft.
    var permissions by
        rememberSaveable(id, stateSaver = rolePermissionsSaver) {
            mutableStateOf(initial)
        }
    var delete by remember { mutableStateOf(false) }
    val changed =
        name != existing?.name.orEmpty() ||
            color != existing?.color.orEmpty() ||
            permissions != initial
    SideEffect { onDirty(changed) }
    val colorValid = color.isEmpty() || Regex("#[0-9a-fA-F]{6}").matches(color)
    val valid = name.trim().isNotEmpty() && name.codePointCount(0, name.length) <= 32 && colorValid
    val target = LocalSettingTarget.current
    val list = rememberLazyListState()
    LaunchedEffect(target) {
        if (target.startsWith("role-") && target != "role-name")
            list.scrollToItem(
                when (target) {
                    "role-color" -> 1
                    "role-delete" -> 5
                    else -> 2
                }
            )
    }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-role-editor"),
        state = list,
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item {
            OutlinedTextField(
                name,
                { name = it },
                Modifier.fillMaxWidth().then(settingTarget("role-name")).testTag("admin-role-name"),
                label = { Text(ui(R.string.role_name)) },
                singleLine = true,
                enabled = !ui.busy,
                isError = name.isNotEmpty() && name.codePointCount(0, name.length) > 32,
            )
        }
        item {
            RoleColorEditor(color, colorValid, !ui.busy) { color = it }
        }
        item {
            RolePermissionsEditor(permissions, !ui.busy) { permissions = it }
        }
        item {
            Button(
                enabled = valid && !ui.busy && (id == null || changed),
                onClick = {
                    ui.mutate(
                        if (id == null) AdministrationChange.CreateRole(name, color, permissions)
                        else AdministrationChange.UpdateRole(id, name, color, permissions),
                        if (id == null) ui(R.string.role_created) else ui(R.string.role_saved),
                    ) {
                        onSaved()
                    }
                },
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("save-role"),
            ) {
                Text(if (id == null) ui(R.string.create_role) else ui(R.string.save_role))
            }
        }
        if (id != null) {
            item {
                SettingsCard {
                    SettingsLink(
                        ui(R.string.people_with_this_role),
                        administrationPeople(ui.data?.people.orEmpty().count { id in it.roles }),
                        Icons.Outlined.PeopleOutline,
                        { open("RolePeople/$id", "role-members") },
                    )
                }
            }
            item {
                OutlinedButton(
                    enabled = !ui.busy,
                    onClick = { delete = true },
                    modifier =
                        Modifier.fillMaxWidth()
                            .heightIn(min = 48.dp)
                            .then(settingTarget("role-delete")),
                ) {
                    Text(ui(R.string.delete_role), color = MaterialTheme.colorScheme.error)
                }
            }
        }
    }
    if (delete && id != null)
        AdministrationDelete(
            ui(R.string.delete_named, name),
            ui(R.string.this_removes_the_role_from_everyone_and_deletes_its_channel_rules),
            ui.busy,
            { delete = false },
        ) {
            ui.mutate(AdministrationChange.DeleteRole(id), ui(R.string.role_deleted)) {
                delete = false
                onSaved()
            }
        }
}

@Composable
internal fun RoleMembershipAdministration(server: ServerState, ui: AdministrationUi, id: String) {
    val role = server.roles.find { it.id == id }
    var query by rememberSaveable { mutableStateOf("") }
    if (role == null) {
        AdministrationEmpty(ui(R.string.this_role_no_longer_exists))
        return
    }
    val people =
        ui.data
            ?.people
            .orEmpty()
            .filter { searchFold("${it.name} ${it.username}").contains(searchFold(query)) }
            .sortedWith(
                compareByDescending<ManagedPerson> { id in it.roles }.thenBy { it.name.lowercase() }
            )
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-role-people"),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item {
            Text(role.name, style = MaterialTheme.typography.titleLarge)
            OutlinedTextField(
                query,
                { query = it },
                Modifier.fillMaxWidth().padding(top = 12.dp),
                label = { Text(ui(R.string.search_people)) },
                singleLine = true,
                leadingIcon = { Icon(Icons.Outlined.Search, null) },
            )
        }
        if (people.isEmpty() && !ui.loading)
            item { AdministrationEmpty(ui(R.string.no_matching_people)) }
        items(people, key = { it.id }) { person ->
            SettingsCard {
                ListItem(
                    headlineContent = { Text(person.name) },
                    supportingContent = {
                        Text(
                            "@${person.username}${if(person.banned) " · ${ui(R.string.banned)}" else ""}"
                        )
                    },
                    leadingContent = { UserAvatar(person.id, person.name) },
                    trailingContent = {
                        Switch(
                            id in person.roles,
                            { assigned ->
                                ui.mutate(
                                    AdministrationChange.AssignRole(person.id, id, assigned),
                                    if (assigned) ui(R.string.role_assigned)
                                    else ui(R.string.role_removed),
                                )
                            },
                            modifier =
                                Modifier.semantics {
                                    contentDescription =
                                        ui(R.string.assign_1_s_to_2_s, role.name, person.name)
                                },
                            enabled = !ui.busy,
                        )
                    },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                )
            }
        }
    }
}

@Composable
internal fun PeopleAdministration(server: ServerState, ui: AdministrationUi) {
    var query by rememberSaveable { mutableStateOf("") }
    var selected by remember { mutableStateOf<ManagedPerson?>(null) }
    val people =
        ui.data?.people.orEmpty().filter {
            searchFold("${it.name} ${it.username} ${it.id}").contains(searchFold(query))
        }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-people"),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item {
            OutlinedTextField(
                query,
                { query = it },
                Modifier.fillMaxWidth(),
                label = { Text(ui(R.string.search_people)) },
                singleLine = true,
                leadingIcon = { Icon(Icons.Outlined.Search, null) },
            )
        }
        if (people.isEmpty() && !ui.loading)
            item { AdministrationEmpty(ui(R.string.no_matching_people)) }
        items(people, key = { it.id }) { person ->
            SettingsCard {
                ListItem(
                    headlineContent = { Text(person.name) },
                    supportingContent = {
                        Text(
                            "@${person.username}${if(person.owner) " · ${uiLabel("Owner")}" else if(person.banned) " · ${ui(R.string.banned)}" else ""}"
                        )
                    },
                    leadingContent = { UserAvatar(person.id, person.name) },
                    trailingContent = {
                        if (!person.owner && person.id != server.me)
                            TextButton(enabled = !ui.busy, onClick = { selected = person }) {
                                Text(
                                    if (person.banned) uiLabel("Unban") else uiLabel("Ban"),
                                    color =
                                        if (person.banned) MaterialTheme.colorScheme.primary
                                        else MaterialTheme.colorScheme.error,
                                )
                            }
                    },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                )
            }
        }
    }
    selected?.let { person ->
        AlertDialog(
            onDismissRequest = { if (!ui.busy) selected = null },
            title = {
                Text(
                    ui(
                        if (person.banned) R.string.unban_person else R.string.ban_person,
                        person.name,
                    )
                )
            },
            text = {
                Text(
                    if (person.banned) ui(R.string.restore_this_person_s_server_access)
                    else
                        ui(
                            R.string
                                .this_person_will_lose_access_to_this_server_their_messages_remain_in_the_co
                        )
                )
            },
            confirmButton = {
                TextButton(
                    enabled = !ui.busy,
                    onClick = {
                        ui.mutate(
                            AdministrationChange.Ban(person.id, !person.banned),
                            if (person.banned) ui(R.string.person_unbanned)
                            else ui(R.string.person_banned),
                        ) {
                            selected = null
                        }
                    },
                ) {
                    Text(if (person.banned) uiLabel("Unban") else uiLabel("Ban"))
                }
            },
            dismissButton = {
                TextButton(enabled = !ui.busy, onClick = { selected = null }) {
                    Text(ui(R.string.cancel))
                }
            },
        )
    }
}

private val rolePermissionsSaver =
    listSaver<Capabilities, Boolean>(
        save = {
            listOf(
                it.manageServer,
                it.manageChannels,
                it.manageRoles,
                it.banMembers,
                it.pinMessage,
                it.everyoneMessage,
                it.sendAttachment,
            )
        },
        restore = { Capabilities(it[0], it[1], it[2], it[3], it[4], it[5], it[6]) },
    )

@Composable
private fun RoleColorEditor(
    color: String,
    colorValid: Boolean,
    enabled: Boolean,
    onChange: (String) -> Unit,
) {
    SettingsCard {
        Column(
            Modifier.padding(16.dp).then(settingTarget("role-color")),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            SettingsLabel(ui(R.string.color))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf(
                        "#FF6338",
                        "#F6BB00",
                        "#2CB34A",
                        "#32BDF1",
                        "#168BFF",
                        "#8454FF",
                        "#F24691",
                        "#99999F",
                    )
                    .forEach { value ->
                        FilterChip(
                            color.equals(value, true),
                            { onChange(value) },
                            label = {
                                Box(Modifier.size(22.dp).background(roleColor(value), CircleShape))
                            },
                            enabled = enabled,
                            modifier =
                                Modifier.semantics {
                                    contentDescription = ui(R.string.color_value, value)
                                },
                        )
                    }
                FilterChip(
                    color.isEmpty(),
                    { onChange("") },
                    label = { Text(ui(R.string.default_color)) },
                    enabled = enabled,
                )
            }
            OutlinedTextField(
                color,
                onChange,
                Modifier.fillMaxWidth(),
                label = { Text(ui(R.string.hex_color)) },
                placeholder = { Text("#RRGGBB") },
                singleLine = true,
                enabled = enabled,
                isError = !colorValid,
            )
        }
    }
}

@Composable
private fun RolePermissionsEditor(
    permissions: Capabilities,
    enabled: Boolean,
    onChange: (Capabilities) -> Unit,
) {
    SettingsCard {
        Column(Modifier.padding(horizontal = 16.dp).then(settingTarget("role-permissions"))) {
            SettingsLabel(ui(R.string.permissions))
            AdministrationToggle(
                ui(R.string.manage_server),
                permissions.manageServer,
                enabled,
                "role-manage-server",
            ) {
                onChange(permissions.copy(manageServer = it))
            }
            AdministrationToggle(
                ui(R.string.manage_channels),
                permissions.manageChannels,
                enabled,
                "role-manage-channels",
            ) {
                onChange(permissions.copy(manageChannels = it))
            }
            AdministrationToggle(
                ui(R.string.manage_roles),
                permissions.manageRoles,
                enabled,
                "role-manage-roles",
            ) {
                onChange(permissions.copy(manageRoles = it))
            }
            AdministrationToggle(
                ui(R.string.ban_people),
                permissions.banMembers,
                enabled,
                "role-ban-members",
            ) {
                onChange(permissions.copy(banMembers = it))
            }
            AdministrationToggle(
                ui(R.string.pin_messages),
                permissions.pinMessage,
                enabled,
                "role-pin-message",
            ) {
                onChange(permissions.copy(pinMessage = it))
            }
            AdministrationToggle(
                ui(R.string.mention_everyone),
                permissions.everyoneMessage,
                enabled,
                "role-everyone-message",
            ) {
                onChange(permissions.copy(everyoneMessage = it))
            }
            AdministrationToggle(
                ui(R.string.send_attachments),
                permissions.sendAttachment,
                enabled,
                "role-send-attachment",
            ) {
                onChange(permissions.copy(sendAttachment = it))
            }
        }
    }
}
