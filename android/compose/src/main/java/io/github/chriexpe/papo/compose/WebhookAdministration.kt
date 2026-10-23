package io.github.chriexpe.papo.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

internal fun webhookFailure(error: Exception): String =
    when (error) {
        is WebhookException.AccountChanged -> ui(R.string.account_changed)
        is WebhookException.Denied -> ui(R.string.webhook_permission_denied)
        is WebhookException.InvalidName -> ui(R.string.webhook_invalid_name)
        is WebhookException.InvalidChannel -> ui(R.string.webhook_invalid_channel)
        is WebhookException.InvalidAvatar -> ui(R.string.webhook_invalid_avatar)
        is WebhookException.Missing -> ui(R.string.webhook_missing)
        is WebhookException.SignedOut -> ui(R.string.social_sign_in_again)
        is WebhookException.Unavailable -> ui(R.string.social_network_failure)
        is WebhookException.CreationUnconfirmed -> ui(R.string.webhook_creation_unconfirmed)
        else -> ui(R.string.webhook_request_failed)
    }

/** Transient management metadata only. Issued credentials are separate one-shot objects. */
@Stable
internal class WebhookUi(
    val server: ServerState,
    val model: PapoViewModel,
    private val scope: CoroutineScope,
    private val snackbar: SnackbarHostState,
) {
    var hooks by mutableStateOf<List<WebhookView>>(emptyList())
        private set

    var loading by mutableStateOf(false)
        private set

    var loaded by mutableStateOf(false)
        private set

    var busy by mutableStateOf(false)
        private set

    var error by mutableStateOf("")
        private set

    var issued by mutableStateOf<WebhookReceipt?>(null)
        private set

    var avatarRevisions by mutableStateOf<Map<String, Int>>(emptyMap())
        private set

    var creationUnconfirmed by mutableStateOf(false)
        private set

    fun dismissToken() {
        issued?.close()
        issued = null
    }

    fun load() {
        if (loading || busy) return
        scope.launch {
            loading = true
            error = ""
            try {
                hooks = model.webhooks(server)
                loaded = true
                creationUnconfirmed = false
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = webhookFailure(e)
            } finally {
                loading = false
            }
        }
    }

    private suspend fun <T> mutate(operation: suspend () -> T): T? {
        if (busy) return null
        busy = true
        error = ""
        return try {
            operation()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            creationUnconfirmed = e is WebhookException.CreationUnconfirmed
            error = webhookFailure(e)
            null
        } finally {
            busy = false
        }
    }

    fun create(channel: String, name: String, saved: () -> Unit) {
        scope.launch {
            if (creationUnconfirmed || !loaded || loading) return@launch
            val receipt = mutate { model.createWebhook(server, channel, name) } ?: return@launch
            val webhook = receipt.webhook()
            hooks = listOf(webhook) + hooks.filterNot { it.id == webhook.id }
            dismissToken()
            issued = receipt
            saved()
        }
    }

    suspend fun update(id: String, nickname: String, avatar: String?, format: String?): Boolean {
        val updated =
            mutate { model.updateWebhook(server, id, nickname, avatar, format) } ?: return false
        hooks = hooks.map { if (it.id == id) updated else it }
        if (avatar != null)
            avatarRevisions = avatarRevisions + (id to ((avatarRevisions[id] ?: 0) + 1))
        scope.launch { snackbar.showSnackbar(ui(R.string.webhook_identity_saved)) }
        return true
    }

    fun revoke(id: String, saved: () -> Unit) {
        scope.launch {
            mutate { model.revokeWebhook(server, id) } ?: return@launch
            hooks = hooks.filterNot { it.id == id }
            saved()
            snackbar.showSnackbar(ui(R.string.webhook_revoked))
        }
    }
}

@Composable
internal fun WebhookAdministrationList(
    server: ServerState,
    hooks: WebhookUi,
    open: (String, String) -> Unit,
) {
    var revoking by remember { mutableStateOf<WebhookView?>(null) }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("admin-webhooks"),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            Button(
                enabled = !hooks.busy && !hooks.loading && hooks.loaded,
                modifier = Modifier.testTag("create-webhook"),
                onClick = { open("New webhook", "") },
            ) {
                Icon(Icons.Outlined.Add, null)
                Spacer(Modifier.width(8.dp))
                Text(ui(R.string.create_webhook))
            }
        }
        if (hooks.error.isNotBlank())
            item { SettingsError(hooks.error, if (!hooks.busy) hooks::load else null) }
        if (hooks.loading) item { LinearProgressIndicator(Modifier.fillMaxWidth()) }
        if (hooks.loaded && hooks.hooks.isEmpty())
            item { AdministrationEmpty(ui(R.string.no_webhooks)) }
        items(hooks.hooks, key = { it.id }) { hook ->
            SettingsCard {
                ListItem(
                    headlineContent = {
                        Text(hook.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    },
                    supportingContent = {
                        Column {
                            if (hook.displayName.isNotBlank())
                                Text(ui(R.string.webhook_displays_as, hook.displayName))
                            Text(
                                server.channels.firstOrNull { it.id == hook.channelId }?.name
                                    ?: hook.channelId
                            )
                        }
                    },
                    leadingContent = {
                        key(hooks.avatarRevisions[hook.id]) {
                            ExternalAvatar(
                                server,
                                ExternalAuthorView(
                                    "webhook",
                                    hook.id,
                                    hook.displayName.ifBlank { hook.name },
                                    "/webhooks/${hook.id}/avatar",
                                    "",
                                ),
                            )
                        }
                    },
                    trailingContent = { Icon(Icons.Outlined.ChevronRight, null) },
                    colors =
                        ListItemDefaults.colors(
                            containerColor = androidx.compose.ui.graphics.Color.Transparent
                        ),
                    modifier =
                        Modifier.testTag("webhook-${hook.id}").clickable(enabled = !hooks.busy) {
                            open("Webhook/${hook.id}", "")
                        },
                )
                TextButton(
                    enabled = !hooks.busy,
                    modifier = Modifier.testTag("revoke-${hook.id}"),
                    onClick = { revoking = hook },
                ) {
                    Icon(Icons.Outlined.LinkOff, null)
                    Spacer(Modifier.width(8.dp))
                    Text(ui(R.string.revoke_webhook), color = MaterialTheme.colorScheme.error)
                }
            }
        }
    }
    revoking?.let { hook ->
        AlertDialog(
            onDismissRequest = { if (!hooks.busy) revoking = null },
            title = { Text(ui(R.string.revoke_webhook_title, hook.name)) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(ui(R.string.revoke_webhook_explanation))
                    if (hooks.error.isNotBlank()) SettingsError(hooks.error)
                }
            },
            confirmButton = {
                TextButton(
                    enabled = !hooks.busy,
                    modifier = Modifier.testTag("confirm-revoke-webhook"),
                    onClick = { hooks.revoke(hook.id) { revoking = null } },
                ) {
                    Text(ui(R.string.revoke_webhook))
                }
            },
            dismissButton = {
                TextButton(enabled = !hooks.busy, onClick = { revoking = null }) {
                    Text(ui(R.string.cancel))
                }
            },
        )
    }
}

internal fun String.codePointsCount() = codePointCount(0, length)

@Composable
internal fun WebhookCreation(
    server: ServerState,
    hooks: WebhookUi,
    dirty: (Boolean) -> Unit,
    saved: () -> Unit,
) {
    var name by rememberSaveable { mutableStateOf("") }
    var selected by rememberSaveable { mutableStateOf("") }
    var expanded by remember { mutableStateOf(false) }
    val channels = server.channels.filter { it.kind == "text" || it.kind == "voice" }
    LaunchedEffect(name, selected) { dirty(name.isNotEmpty() || selected.isNotEmpty()) }
    Column(
        Modifier.fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(20.dp)
            .testTag("new-webhook"),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        if (hooks.error.isNotBlank()) SettingsError(hooks.error)
        OutlinedTextField(
            name,
            { name = it },
            Modifier.fillMaxWidth().testTag("webhook-name"),
            singleLine = true,
            enabled = !hooks.busy,
            label = { Text(ui(R.string.webhook_name)) },
            isError = name.codePointsCount() > 80,
            supportingText = { Text(ui(R.string.webhook_name_limit)) },
        )
        Box {
            OutlinedButton(
                enabled = !hooks.busy,
                onClick = { expanded = true },
                modifier = Modifier.testTag("webhook-channel"),
            ) {
                Text(
                    channels.firstOrNull { it.id == selected }?.name ?: ui(R.string.choose_channel)
                )
                Icon(Icons.Outlined.ExpandMore, null)
            }
            DropdownMenu(expanded, { expanded = false }) {
                channels.forEach { channel ->
                    DropdownMenuItem(
                        text = { Text(channel.name) },
                        onClick = {
                            selected = channel.id
                            expanded = false
                        },
                        leadingIcon = {
                            Icon(
                                if (channel.kind == "voice") Icons.Outlined.VolumeUp
                                else Icons.Outlined.Tag,
                                null,
                            )
                        },
                    )
                }
            }
        }
        if (channels.isEmpty()) Text(ui(R.string.webhook_no_channels))
        Button(
            enabled =
                !hooks.busy &&
                    !hooks.creationUnconfirmed &&
                    name.trim().isNotEmpty() &&
                    name.codePointsCount() <= 80 &&
                    channels.any { it.id == selected },
            onClick = { hooks.create(selected, name.trim(), saved) },
            modifier = Modifier.testTag("save-new-webhook"),
        ) {
            Text(ui(R.string.create_webhook))
        }
        if (hooks.creationUnconfirmed) Text(ui(R.string.webhook_check_list))
    }
}
