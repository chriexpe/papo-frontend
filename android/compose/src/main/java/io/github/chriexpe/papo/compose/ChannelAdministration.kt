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
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

@Composable
internal fun AdministrationEmpty(text: String) {
    Text(text, Modifier.padding(20.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
}

@Composable
internal fun AdministrationDelete(
    title: String,
    text: String,
    busy: Boolean,
    onDismiss: () -> Unit,
    onDelete: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = { Text(title) },
        text = { Text(text) },
        confirmButton = {
            TextButton(enabled = !busy, onClick = onDelete) {
                Text(ui(R.string.delete), color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = {
            TextButton(enabled = !busy, onClick = onDismiss) { Text(ui(R.string.cancel)) }
        },
    )
}

@Composable
internal fun ChannelAdministrationList(
    server: ServerState,
    ui: AdministrationUi,
    open: (String, String) -> Unit,
) {
    val channels = ui.data?.channels.orEmpty().sortedBy { it.position }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-channels"),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(enabled = !ui.busy, onClick = { open("New channel", "") }) {
                    Icon(Icons.Outlined.Add, null)
                    Text(ui(R.string.create_channel))
                }
                OutlinedButton(enabled = !ui.busy, onClick = { open("New category", "") }) {
                    Text(ui(R.string.create_category))
                }
            }
        }
        if (channels.isEmpty() && !ui.loading)
            item {
                AdministrationEmpty(
                    ui(R.string.no_channels_yet_create_a_text_channel_to_get_started)
                )
            }
        items(channels, key = { it.id }) { channel ->
            val siblings = channels.filter { it.parentId == channel.parentId }
            val index = siblings.indexOfFirst { it.id == channel.id }
            SettingsCard {
                SettingsLink(
                    channel.name,
                    if (channel.kind == "category") "Category"
                    else
                        listOfNotNull(
                                channels.find { it.id == channel.parentId }?.name,
                                if (channel.kind == "voice") ui(R.string.voice_channel)
                                else ui(R.string.text_channel),
                            )
                            .joinToString(" · "),
                    when (channel.kind) {
                        "category" -> Icons.Outlined.Folder
                        "voice" -> Icons.Outlined.VolumeUp
                        else -> Icons.Outlined.Tag
                    },
                    { open("Channel/${channel.id}", "") },
                )
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 8.dp),
                    horizontalArrangement = Arrangement.End,
                ) {
                    IconButton(
                        enabled = !ui.busy && index > 0,
                        onClick = {
                            ui.mutate(
                                AdministrationChange.MoveChannel(
                                    channel.id,
                                    siblings[index - 1].id,
                                    null,
                                ),
                                ui(R.string.channel_moved),
                            )
                        },
                    ) {
                        Icon(Icons.Outlined.ArrowUpward, ui(R.string.move_1_s_up, channel.name))
                    }
                    IconButton(
                        enabled = !ui.busy && index >= 0 && index < siblings.lastIndex,
                        onClick = {
                            ui.mutate(
                                AdministrationChange.MoveChannel(
                                    channel.id,
                                    siblings[index + 1].id,
                                    null,
                                ),
                                ui(R.string.channel_moved),
                            )
                        },
                    ) {
                        Icon(Icons.Outlined.ArrowDownward, ui(R.string.move_1_s_down, channel.name))
                    }
                }
            }
        }
    }
}

@Composable
internal fun ChannelAdministrationEditor(
    server: ServerState,
    ui: AdministrationUi,
    id: String?,
    category: Boolean,
    onDirty: (Boolean) -> Unit,
    onSaved: () -> Unit,
    open: (String, String) -> Unit = { _, _ -> },
) {
    val existing = ui.data?.channels?.find { it.id == id }
    if (id != null && existing == null) {
        if (!ui.loading) AdministrationEmpty(ui(R.string.this_channel_no_longer_exists))
        return
    }
    var name by rememberSaveable(id) { mutableStateOf(existing?.name.orEmpty()) }
    var topic by rememberSaveable(id) { mutableStateOf(existing?.topic.orEmpty()) }
    var kind by
        rememberSaveable(id) {
            mutableStateOf(existing?.kind ?: if (category) "category" else "text")
        }
    var delete by remember { mutableStateOf(false) }
    val changed =
        if (id == null)
            name.isNotEmpty() ||
                topic.isNotEmpty() ||
                kind != (if (category) "category" else "text")
        else name != existing?.name || topic != existing.topic
    SideEffect { onDirty(changed) }
    val valid =
        name.trim().isNotEmpty() &&
            name.codePointCount(0, name.length) <= 32 &&
            topic.codePointCount(0, topic.length) <= 512
    val target = LocalSettingTarget.current
    val list = rememberLazyListState()
    LaunchedEffect(target) {
        when (target) {
            "channel-topic" -> list.scrollToItem(2)
            "channel-delete" -> list.scrollToItem(if (kind == "category") 3 else 5)
        }
    }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-channel-editor"),
        state = list,
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item {
            OutlinedTextField(
                name,
                { name = it },
                Modifier.fillMaxWidth()
                    .then(settingTarget("channel-name"))
                    .testTag("admin-channel-name"),
                label = {
                    Text(
                        if (kind == "category") ui(R.string.category_name)
                        else ui(R.string.channel_name)
                    )
                },
                singleLine = true,
                enabled = !ui.busy,
                isError = name.codePointCount(0, name.length) > 32,
                supportingText = { Text(ui(R.string.up_to_32_characters)) },
            )
        }
        item {
            SettingsLabel(ui(R.string.type))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                (if (category || existing?.kind == "category") listOf("category" to "Category")
                    else listOf("text" to "Text", "voice" to "Voice"))
                    .forEach { (value, label) ->
                        FilterChip(
                            kind == value,
                            { kind = value },
                            label = { Text(uiLabel(label)) },
                            enabled = id == null && !ui.busy && !category,
                        )
                    }
            }
            if (id != null)
                Text(
                    ui(R.string.the_channel_type_is_fixed_after_creation),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
        }
        if (kind != "category") {
            item {
                OutlinedTextField(
                    topic,
                    { topic = it },
                    Modifier.fillMaxWidth().then(settingTarget("channel-topic")),
                    label = { Text(ui(R.string.description)) },
                    minLines = 3,
                    maxLines = 8,
                    enabled = !ui.busy,
                    isError = topic.codePointCount(0, topic.length) > 512,
                    supportingText = { Text("${topic.codePointCount(0,topic.length)} / 512") },
                )
            }
            if (id != null)
                item {
                    SettingsCard {
                        SettingsLink(
                            ui(R.string.channel_access),
                            ui(R.string.read_send_moderate_and_connect_by_role),
                            Icons.Outlined.Lock,
                            { open("Access/$id", "channel-access") },
                            target = "channel-access",
                        )
                    }
                }
        }
        item {
            Button(
                enabled = valid && !ui.busy && (id == null || changed),
                onClick = {
                    val change =
                        if (id == null)
                            AdministrationChange.CreateChannel(name, kind, topic, null, emptyList())
                        else AdministrationChange.UpdateChannel(id, name, topic)
                    ui.mutate(
                        change,
                        if (id == null) "${if(kind=="category") "Category" else "Channel"} created"
                        else ui(R.string.channel_saved),
                    ) {
                        onSaved()
                    }
                },
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("save-channel"),
            ) {
                Text(
                    if (id == null) "Create ${if(kind=="category") "category" else "channel"}"
                    else ui(R.string.save_channel)
                )
            }
        }
        if (id != null)
            item {
                OutlinedButton(
                    enabled = !ui.busy,
                    onClick = { delete = true },
                    modifier =
                        Modifier.fillMaxWidth()
                            .heightIn(min = 48.dp)
                            .then(settingTarget("channel-delete")),
                ) {
                    Icon(Icons.Outlined.DeleteOutline, null)
                    Spacer(Modifier.width(8.dp))
                    Text(
                        "Delete ${if(kind=="category") "category" else "channel"}",
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
    }
    if (delete && id != null)
        AdministrationDelete(
            "Delete ${name.ifBlank {"channel"}}?",
            if (kind == "category")
                ui(R.string.this_removes_the_category_other_channels_remain_available)
            else ui(R.string.this_permanently_deletes_the_channel_and_its_messages),
            ui.busy,
            { delete = false },
        ) {
            ui.mutate(AdministrationChange.DeleteChannel(id), ui(R.string.channel_deleted)) {
                delete = false
                onSaved()
            }
        }
}

@Composable
internal fun ChannelAccessAdministration(server: ServerState, ui: AdministrationUi, id: String) {
    var rules by remember { mutableStateOf<List<ChannelRule>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf("") }
    var editing by remember { mutableStateOf<ChannelRule?>(null) }
    val scope = rememberCoroutineScope()
    suspend fun load() {
        loading = true
        error = ""
        try {
            rules = ui.model.channelRules(server, id)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = e.message.orEmpty()
        } finally {
            loading = false
        }
    }
    LaunchedEffect(server.url, server.me, id) { load() }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-channel-access"),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            Text(
                ui(
                    R.string
                        .access_is_saved_on_the_server_and_applies_to_every_device_with_no_role_rule
                ),
                style = MaterialTheme.typography.bodyMedium,
            )
        }
        if (loading) item { LinearProgressIndicator(Modifier.fillMaxWidth()) }
        if (error.isNotBlank()) item { SettingsError(error) { scope.launch { load() } } }
        if (!loading && rules.isEmpty())
            item { AdministrationEmpty(ui(R.string.everyone_can_access_this_channel)) }
        items(rules, key = { it.roleId }) { rule ->
            SettingsCard {
                SettingsLink(
                    rule.roleName,
                    listOfNotNull(
                            if (rule.access.read) uiLabel("Read") else null,
                            if (rule.access.send) uiLabel("Send") else null,
                            if (rule.access.deleteMessages) ui(R.string.moderate) else null,
                            if (rule.access.connectVoice) uiLabel("Voice") else null,
                        )
                        .joinToString(" · ")
                        .ifEmpty { ui(R.string.no_permissions) },
                    Icons.Outlined.Shield,
                    { editing = rule },
                )
            }
        }
        item {
            SettingsLabel(ui(R.string.add_a_role))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                server.roles
                    .filter { r -> rules.none { it.roleId == r.id } }
                    .forEach { role ->
                        AssistChip(
                            onClick = {
                                editing =
                                    ChannelRule(
                                        role.id,
                                        role.name,
                                        ChannelAccess(true, true, false, true),
                                    )
                            },
                            label = { Text(role.name) },
                            leadingIcon = { Icon(Icons.Outlined.Add, null) },
                            enabled = !ui.busy && !loading,
                        )
                    }
            }
        }
    }
    editing?.let { rule ->
        var read by remember(rule.roleId) { mutableStateOf(rule.access.read) }
        var send by remember(rule.roleId) { mutableStateOf(rule.access.send) }
        var moderate by remember(rule.roleId) { mutableStateOf(rule.access.deleteMessages) }
        var voice by remember(rule.roleId) { mutableStateOf(rule.access.connectVoice) }
        AlertDialog(
            onDismissRequest = { if (!ui.busy) editing = null },
            title = { Text(rule.roleName) },
            text = {
                Column {
                    AdministrationToggle(ui(R.string.read_channel), read, !ui.busy) { read = it }
                    AdministrationToggle(ui(R.string.send_messages), send, !ui.busy) { send = it }
                    AdministrationToggle(ui(R.string.delete_messages), moderate, !ui.busy) {
                        moderate = it
                    }
                    AdministrationToggle(ui(R.string.connect_to_voice), voice, !ui.busy) {
                        voice = it
                    }
                    if (rules.any { it.roleId == rule.roleId })
                        TextButton(
                            enabled = !ui.busy,
                            onClick = {
                                scope.launch {
                                    try {
                                        ui.apply(
                                            AdministrationChange.RemoveChannelRule(id, rule.roleId),
                                            ui(R.string.role_rule_removed),
                                        )
                                        editing = null
                                        load()
                                    } catch (e: CancellationException) {
                                        throw e
                                    } catch (_: Exception) {}
                                }
                            },
                        ) {
                            Text(
                                ui(R.string.remove_role_rule),
                                color = MaterialTheme.colorScheme.error,
                            )
                        }
                    if (ui.error.isNotBlank())
                        Text(ui.error, color = MaterialTheme.colorScheme.error)
                }
            },
            confirmButton = {
                TextButton(
                    enabled = !ui.busy,
                    onClick = {
                        scope.launch {
                            try {
                                ui.apply(
                                    AdministrationChange.SetChannelRule(
                                        id,
                                        rule.roleId,
                                        ChannelAccess(read, send, moderate, voice),
                                    ),
                                    ui(R.string.channel_access_saved),
                                )
                                editing = null
                                load()
                            } catch (e: CancellationException) {
                                throw e
                            } catch (_: Exception) {}
                        }
                    },
                ) {
                    Text(ui(R.string.save))
                }
            },
            dismissButton = {
                TextButton(enabled = !ui.busy, onClick = { editing = null }) {
                    Text(ui(R.string.cancel))
                }
            },
        )
    }
}

@Composable
internal fun AdministrationToggle(
    title: String,
    value: Boolean,
    enabled: Boolean,
    target: String = "",
    onChange: (Boolean) -> Unit,
) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).then(settingTarget(target)),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(title, Modifier.weight(1f))
        Switch(
            value,
            onChange,
            Modifier.semantics { contentDescription = title },
            enabled = enabled,
        )
    }
}
