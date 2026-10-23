@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
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

internal fun administrationPeople(count: Int) = uiQuantity(R.plurals.people_count, count)

@Stable
internal class AdministrationUi(
    val server: ServerState,
    val model: PapoViewModel,
    private val scope: CoroutineScope,
    val snackbar: SnackbarHostState,
) {
    var data by mutableStateOf<AdministrationState?>(null)
        private set

    var loading by mutableStateOf(false)
        private set

    var busy by mutableStateOf(false)
        private set

    var error by mutableStateOf("")

    fun refresh() {
        if (loading || busy) return
        scope.launch {
            loading = true
            error = ""
            try {
                data = model.administration(server)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = e.message ?: ui(R.string.couldn_t_load_server_settings)
            } finally {
                loading = false
            }
        }
    }

    suspend fun apply(change: AdministrationChange, message: String): AdministrationOutcome {
        check(!busy) { ui(R.string.wait_for_the_current_change_to_finish) }
        busy = true
        error = ""
        try {
            val result = model.administer(server, change)
            var refreshError = result.refreshError
            try {
                data = model.administration(server, false)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                refreshError = refreshError.ifBlank { e.message ?: ui(R.string.refresh_failed) }
            }
            if (refreshError.isNotBlank())
                error = ui(R.string.saved_but_couldn_t_refresh_the_server_1_s, refreshError)
            scope.launch { snackbar.showSnackbar(message) }
            return result
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = e.message ?: ui(R.string.couldn_t_save_changes)
            throw e
        } finally {
            busy = false
        }
    }

    fun mutate(
        change: AdministrationChange,
        message: String,
        onSaved: (AdministrationOutcome) -> Unit = {},
    ) {
        if (busy) return
        scope.launch {
            try {
                onSaved(apply(change, message))
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {}
        }
    }
}

internal fun serverSettingEntries(
    server: ServerState,
    data: AdministrationState?,
): List<SettingEntry> = buildList {
    val p = server.capabilities
    if (p.manageServer) {
        add(
            SettingEntry(
                "server-name",
                "General",
                ui(R.string.server_name),
                server.name,
                "rename general",
            )
        )
        add(
            SettingEntry(
                "server-icon",
                "General",
                ui(R.string.server_picture),
                ui(R.string.crop_and_update_the_server_icon),
                "photo avatar image",
            )
        )
        add(
            SettingEntry(
                "server-access",
                "General",
                ui(R.string.server_access),
                ui(R.string.public_or_password_protected),
                "private password",
            )
        )
        if (!server.serverPublic)
            add(
                SettingEntry(
                    "server-password",
                    "General",
                    ui(R.string.server_password),
                    ui(R.string.change_the_server_s_access_password),
                    "private security",
                )
            )
        add(
            SettingEntry(
                "stickers",
                "Stickers",
                ui(R.string.server_stickers),
                ui(R.string.add_animated_or_static_stickers),
                "emoji GIF images delete",
            )
        )
        add(
            SettingEntry(
                "audit",
                "Audit",
                ui(R.string.audit_log),
                ui(R.string.who_changed_what),
                "activity history deleted messages actor channel date",
            )
        )
    }
    if (p.manageChannels) {
        add(
            SettingEntry(
                "create-channel",
                "Channels",
                ui(R.string.create_channel),
                ui(R.string.text_and_voice_channels),
                "new",
            )
        )
        add(
            SettingEntry(
                "create-category",
                "Channels",
                ui(R.string.create_category),
                ui(R.string.group_server_channels),
                "new",
            )
        )
        add(
            SettingEntry(
                "reorder",
                "Channels",
                ui(R.string.channel_order),
                ui(R.string.move_channels_and_categories),
                "reorder organize",
            )
        )
        data?.channels?.forEach { channel ->
            add(
                SettingEntry(
                    "channel-name",
                    "Channel/${channel.id}",
                    "${channel.name}: name",
                    ui(R.string.rename_this_channel),
                    "channel",
                )
            )
            if (channel.kind != "category")
                add(
                    SettingEntry(
                        "channel-topic",
                        "Channel/${channel.id}",
                        "${channel.name}: description",
                        channel.topic.ifBlank { ui(R.string.channel_topic) },
                        ui(R.string.topic_channel),
                    )
                )
            add(
                SettingEntry(
                    "channel-delete",
                    "Channel/${channel.id}",
                    "Delete ${channel.name}",
                    ui(R.string.permanently_delete_this_channel),
                    "remove channel",
                )
            )
            if (channel.kind != "category")
                add(
                    SettingEntry(
                        "channel-access",
                        "Access/${channel.id}",
                        ui(R.string.access_to_1_s, channel.name),
                        ui(R.string.role_permissions),
                        "read send voice channel",
                    )
                )
        }
    }
    if (p.manageRoles) {
        add(
            SettingEntry(
                "create-role",
                "Roles",
                ui(R.string.create_role),
                ui(R.string.name_color_and_permissions),
                "new tag",
            )
        )
        server.roles.forEach { role ->
            add(
                SettingEntry(
                    "role-name",
                    "Role/${role.id}",
                    "${role.name}: name",
                    ui(R.string.rename_this_role),
                    "tag",
                )
            )
            add(
                SettingEntry(
                    "role-color",
                    "Role/${role.id}",
                    "${role.name}: color",
                    ui(R.string.set_a_nickname_color),
                    "hex tag",
                )
            )
            listOf(
                    "manage-server" to ui(R.string.manage_server),
                    "manage-channels" to ui(R.string.manage_channels),
                    "manage-roles" to ui(R.string.manage_roles),
                    "manage-webhooks" to ui(R.string.manage_webhooks),
                    "ban-members" to ui(R.string.ban_people),
                    "pin-message" to ui(R.string.pin_messages),
                    "everyone-message" to ui(R.string.mention_everyone),
                    "send-attachment" to ui(R.string.send_attachments),
                )
                .forEach { (id, label) ->
                    add(
                        SettingEntry(
                            "role-$id",
                            "Role/${role.id}",
                            "${role.name}: $label",
                            ui(R.string.role_permission),
                            "permissions",
                        )
                    )
                }
            add(
                SettingEntry(
                    "role-delete",
                    "Role/${role.id}",
                    "Delete ${role.name}",
                    ui(R.string.remove_this_role_and_its_channel_rules),
                    "delete tag",
                )
            )
            add(
                SettingEntry(
                    "role-members",
                    "RolePeople/${role.id}",
                    ui(R.string.people_with_1_s, role.name),
                    ui(R.string.assign_or_remove_this_role),
                    "member tag",
                )
            )
        }
    }
    if (p.banMembers)
        add(
            SettingEntry(
                "people",
                "People",
                ui(R.string.people_and_moderation),
                ui(R.string.ban_or_restore_server_access),
                "members unban",
            )
        )
}

@Composable
internal fun ServerSettingsSheet(
    server: ServerState,
    model: PapoViewModel,
    appError: String,
    onDismiss: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }
    val ui = remember(server.url, server.me) { AdministrationUi(server, model, scope, snackbar) }
    var pages by rememberSaveable(server.url, server.me) { mutableStateOf(listOf("Settings")) }
    var dirtyPages by
        rememberSaveable(server.url, server.me) { mutableStateOf(emptyList<String>()) }
    var query by rememberSaveable { mutableStateOf("") }
    var target by rememberSaveable { mutableStateOf("") }
    var discard by remember { mutableStateOf(false) }
    val holder = rememberSaveableStateHolder()
    val route = pages.last()
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    fun pop() {
        holder.removeState(route)
        dirtyPages = dirtyPages - route
        if (pages.size > 1) pages = pages.dropLast(1) else onDismiss()
    }
    fun back() {
        if (!ui.busy) {
            if (route in dirtyPages) discard = true else pop()
        }
    }
    fun open(page: String, id: String = "") {
        focus.clearFocus()
        keyboard?.hide()
        target = id
        pages = pages + page
    }
    val dirty: (Boolean) -> Unit = { value ->
        if (value && route !in dirtyPages) dirtyPages = dirtyPages + route
        else if (!value && route in dirtyPages) dirtyPages = dirtyPages - route
    }
    val saved: () -> Unit = {
        dirtyPages = dirtyPages - route
        pop()
    }
    LaunchedEffect(ui) { ui.refresh() }
    Dialog(
        onDismissRequest = ::back,
        properties =
            DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false),
    ) {
        val haze = remember { HazeState() }
        CompositionLocalProvider(
            LocalSettingTarget provides target,
            LocalHaze provides haze,
            LocalProfileContext provides ProfileContext(server, model),
        ) {
            Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
                Box(
                    Modifier.fillMaxSize()
                        .navigationBarsPadding()
                        .imePadding()
                        .testTag("server-settings")
                ) {
                    Column(Modifier.fillMaxSize()) {
                        val title =
                            when {
                                route == "Settings" -> ui(R.string.server_settings)
                                route.startsWith("Channel/") -> ui(R.string.edit_channel)
                                route.startsWith("Access/") -> ui(R.string.channel_access)
                                route.startsWith("RolePeople/") -> ui(R.string.role_members)
                                route.startsWith("Role/") -> ui(R.string.edit_role)
                                else -> uiLabel(route)
                            }
                        SettingsHeader(
                            title,
                            ::back,
                            close = route == "Settings",
                            enabled = !ui.busy,
                        ) {
                            if (ui.busy)
                                CircularProgressIndicator(
                                    Modifier.padding(12.dp).size(24.dp),
                                    strokeWidth = 2.dp,
                                )
                        }
                        if (ui.error.isNotBlank())
                            SettingsError(
                                ui.error,
                                if (!ui.busy && !ui.loading) ({ ui.refresh() }) else null,
                            )
                        if (ui.loading)
                            LinearProgressIndicator(
                                Modifier.fillMaxWidth().testTag("administration-loading")
                            )
                        holder.SaveableStateProvider(route) {
                            when {
                                route == "Settings" ->
                                    ServerSettingsOverview(
                                        server,
                                        model,
                                        ui,
                                        query,
                                        { query = it },
                                        ::open,
                                    )
                                route == "General" ->
                                    GeneralAdministration(server, model, ui, dirty, saved)
                                route == "Channels" -> ChannelAdministrationList(server, ui, ::open)
                                route == "New channel" ->
                                    ChannelAdministrationEditor(
                                        server,
                                        ui,
                                        null,
                                        false,
                                        dirty,
                                        saved,
                                    )
                                route == "New category" ->
                                    ChannelAdministrationEditor(
                                        server,
                                        ui,
                                        null,
                                        true,
                                        dirty,
                                        saved,
                                    )
                                route.startsWith("Channel/") ->
                                    ChannelAdministrationEditor(
                                        server,
                                        ui,
                                        route.substringAfter('/'),
                                        false,
                                        dirty,
                                        saved,
                                        ::open,
                                    )
                                route.startsWith("Access/") ->
                                    ChannelAccessAdministration(
                                        server,
                                        ui,
                                        route.substringAfter('/'),
                                    )
                                route == "Roles" -> RoleAdministrationList(server, ui, ::open)
                                route == "New role" ->
                                    RoleAdministrationEditor(server, ui, null, dirty, saved, ::open)
                                route.startsWith("Role/") ->
                                    RoleAdministrationEditor(
                                        server,
                                        ui,
                                        route.substringAfter('/'),
                                        dirty,
                                        saved,
                                        ::open,
                                    )
                                route.startsWith("RolePeople/") ->
                                    RoleMembershipAdministration(
                                        server,
                                        ui,
                                        route.substringAfter('/'),
                                    )
                                route == "People" -> PeopleAdministration(server, ui)
                                route == "Stickers" ->
                                    ServerStickerAdministration(server, model, ui)
                                route == "Audit" -> AuditAdministration(server, model)
                            }
                        }
                    }
                    SnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).padding(12.dp))
                }
                if (discard)
                    AlertDialog(
                        onDismissRequest = { discard = false },
                        title = { Text(ui(R.string.discard_changes)) },
                        text = { Text(ui(R.string.your_unsaved_edits_will_be_lost)) },
                        confirmButton = {
                            TextButton(
                                onClick = {
                                    discard = false
                                    pop()
                                }
                            ) {
                                Text(ui(R.string.discard))
                            }
                        },
                        dismissButton = {
                            TextButton(onClick = { discard = false }) {
                                Text(ui(R.string.keep_editing))
                            }
                        },
                    )
            }
        }
    }
}

@Composable
private fun ServerSettingsOverview(
    server: ServerState,
    model: PapoViewModel,
    ui: AdministrationUi,
    query: String,
    onQuery: (String) -> Unit,
    open: (String, String) -> Unit,
) {
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("server-settings-overview"),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        item {
            OutlinedTextField(
                query,
                onQuery,
                Modifier.fillMaxWidth().testTag("server-settings-search"),
                singleLine = true,
                placeholder = { Text(ui(R.string.search_server_settings)) },
                leadingIcon = { Icon(Icons.Outlined.Search, null) },
                trailingIcon = {
                    if (query.isNotBlank())
                        IconButton(onClick = { onQuery("") }) {
                            Icon(Icons.Outlined.Close, ui(R.string.clear_server_settings_search))
                        }
                },
                shape = RoundedCornerShape(28.dp),
            )
        }
        if (query.isNotBlank()) {
            val words = searchFold(query).trim().split(Regex("\\s+")).filter { it.isNotBlank() }
            val matches =
                serverSettingEntries(server, ui.data).filter { entry ->
                    val text =
                        searchFold(
                            "${entry.title} ${entry.page} ${entry.summary} ${entry.keywords}"
                        )
                    words.all(text::contains)
                }
            items(matches, key = { "${it.page}/${it.id}" }) { entry ->
                SettingsCard {
                    SettingsLink(
                        entry.title,
                        entry.summary,
                        Icons.Outlined.Tune,
                        { open(entry.page, entry.id) },
                    )
                }
            }
            if (matches.isEmpty())
                item {
                    Text(
                        ui(R.string.no_matching_settings),
                        Modifier.padding(16.dp),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
        } else {
            item {
                SettingsCard {
                    Row(
                        Modifier.fillMaxWidth().padding(16.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        ServerIcon(server, model)
                        Column {
                            Text(server.name, style = MaterialTheme.typography.titleMedium)
                            Text(
                                "${if(server.serverPublic) ui(R.string.public_server_label) else ui(R.string.private_server_label)} · ${administrationPeople(ui.data?.people?.size ?: server.members.size)}",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
            val p = server.capabilities
            if (p.manageServer)
                item {
                    SettingsCard {
                        SettingsLink(
                            uiLabel("General"),
                            ui(R.string.name_picture_and_server_access),
                            Icons.Outlined.Settings,
                            { open("General", "") },
                        )
                    }
                }
            if (p.manageChannels)
                item {
                    SettingsCard {
                        SettingsLink(
                            uiLabel("Channels"),
                            ui(
                                R.string.label_1_s_channels_and_categories,
                                ui.data?.channels?.size
                                    ?: server.channels.count { it.kind != "direct" },
                            ),
                            Icons.Outlined.Tag,
                            { open("Channels", "") },
                        )
                    }
                }
            if (p.manageRoles)
                item {
                    SettingsCard {
                        SettingsLink(
                            uiLabel("Roles"),
                            ui(
                                R.string.roles_summary,
                                uiQuantity(R.plurals.role_count, server.roles.size),
                            ),
                            Icons.Outlined.Badge,
                            { open("Roles", "") },
                        )
                    }
                }
            if (p.banMembers)
                item {
                    SettingsCard {
                        SettingsLink(
                            uiLabel("People"),
                            ui(R.string.server_access_and_moderation),
                            Icons.Outlined.PeopleOutline,
                            { open("People", "") },
                        )
                    }
                }
            if (p.manageServer) {
                item {
                    SettingsCard {
                        SettingsLink(
                            uiLabel("Stickers"),
                            uiQuantity(R.plurals.server_sticker_count, server.emojis.size),
                            Icons.Outlined.EmojiEmotions,
                            { open("Stickers", "") },
                        )
                    }
                }
                item {
                    SettingsCard {
                        SettingsLink(
                            uiLabel("Audit"),
                            ui(R.string.who_changed_what),
                            Icons.Outlined.History,
                            { open("Audit", "") },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun GeneralAdministration(
    server: ServerState,
    model: PapoViewModel,
    ui: AdministrationUi,
    onDirty: (Boolean) -> Unit,
    onSaved: () -> Unit,
) {
    var name by rememberSaveable { mutableStateOf(server.name) }
    var public by rememberSaveable { mutableStateOf(server.serverPublic) }
    var password by remember { mutableStateOf("") }
    var editing by remember { mutableStateOf<MediaEditorSource?>(null) }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val picker =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            if (uri != null)
                scope.launch {
                    try {
                        editing = MediaEditorSource.read(context, uri)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        ui.error = e.message ?: ui(R.string.couldn_t_open_this_picture)
                    }
                }
        }
    val dirty = name.trim() != server.name || public != server.serverPublic || password.isNotEmpty()
    SideEffect { onDirty(dirty) }
    val invalid =
        (!public && server.serverPublic || password.isNotEmpty()) &&
            password.codePointCount(0, password.length) < 8
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("server-general-settings"),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item {
            SettingsCard {
                SettingsLink(
                    ui(R.string.server_picture),
                    ui(R.string.crop_and_preview_before_saving),
                    Icons.Outlined.AddPhotoAlternate,
                    {
                        picker.launch(arrayOf("image/png", "image/jpeg", "image/gif", "image/webp"))
                    },
                    leading = { ServerIcon(server, model, Modifier.size(64.dp)) },
                    target = "server-icon",
                )
            }
        }
        item {
            OutlinedTextField(
                name,
                { name = it },
                Modifier.fillMaxWidth()
                    .then(settingTarget("server-name"))
                    .testTag("server-name-field"),
                label = { Text(ui(R.string.server_name)) },
                singleLine = true,
                enabled = !ui.busy,
                isError = name.trim().isEmpty() || name.codePointCount(0, name.length) > 32,
            )
        }
        item {
            SettingsCard {
                Column(Modifier.padding(horizontal = 16.dp).then(settingTarget("server-access"))) {
                    AdministrationToggle(ui(R.string.public_server), public, !ui.busy) {
                        public = it
                    }
                    Text(
                        ui(R.string.private_servers_require_a_server_password),
                        Modifier.padding(bottom = 16.dp),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
        if (!public)
            item {
                OutlinedTextField(
                    password,
                    { password = it },
                    Modifier.fillMaxWidth().then(settingTarget("server-password")),
                    singleLine = true,
                    enabled = !ui.busy,
                    visualTransformation = PasswordVisualTransformation(),
                    label = {
                        Text(
                            if (server.serverPublic) ui(R.string.server_password)
                            else ui(R.string.new_password_optional)
                        )
                    },
                    isError = invalid,
                    supportingText = {
                        Text(
                            if (invalid) ui(R.string.use_at_least_eight_characters)
                            else ui(R.string.leave_empty_to_keep_the_current_password)
                        )
                    },
                )
            }
        item {
            Button(
                enabled =
                    dirty &&
                        name.trim().isNotEmpty() &&
                        name.codePointCount(0, name.length) <= 32 &&
                        !invalid &&
                        !ui.busy,
                onClick = {
                    ui.mutate(
                        AdministrationChange.Server(
                            name,
                            public,
                            password.takeIf { it.isNotEmpty() },
                        ),
                        ui(R.string.server_saved),
                    ) {
                        password = ""
                        onSaved()
                    }
                },
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
            ) {
                Text(ui(R.string.save_server))
            }
        }
    }
    editing?.let { source ->
        MediaEditor(
            source,
            MediaEditPurpose.SERVER_ICON,
            server,
            model,
            onDismiss = { editing = null },
        ) { prepared, _ ->
            ui.apply(
                AdministrationChange.Icon(prepared.blob, prepared.format),
                ui(R.string.server_picture_updated),
            )
        }
    }
}
