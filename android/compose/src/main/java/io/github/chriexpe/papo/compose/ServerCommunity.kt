@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
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
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.Capabilities

internal val Capabilities.canAdmin
    get() = manageServer || manageChannels || manageRoles || manageWebhooks

// The backend currently checks manage_server on both ban and unban. The raw
// banMembers bit still belongs to role editing and must round-trip independently.
internal val Capabilities.canBanMembers
    get() = manageServer

@Composable
internal fun ServerIcon(server: ServerState, model: PapoViewModel, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val revision = server.serverIconRevision
    val image =
        remember(server.url, server.me, revision) {
            if (revision.isBlank()) null
            else
                CachedImage("${server.url}/${server.me}/server-icon/$revision") {
                    model.serverIcon(server)
                }
        }
    var loaded by remember(server.url, server.me, revision) { mutableStateOf(false) }
    Box(
        modifier.size(48.dp).clip(RoundedCornerShape(16.dp)).testTag("server-icon").semantics {
            contentDescription = ui(R.string.label_1_s_server_icon, server.name)
            stateDescription = if (loaded) ui(R.string.icon_loaded) else uiLabel("Initials")
        }
    ) {
        if (!loaded) Avatar(server.name, Modifier.fillMaxSize())
        image?.let {
            ManagedAsyncImage(
                it,
                null,
                MediaImages.loader(context),
                Modifier.fillMaxSize(),
                ContentScale.Crop,
                onSuccess = { loaded = true },
                onError = { loaded = false },
                retentionKey = "server/${server.url}/$revision",
                eager = true,
            )
        }
    }
}

@Composable
internal fun ServerHeader(
    server: ServerState,
    model: PapoViewModel,
    appError: String,
    modifier: Modifier = Modifier,
) {
    var panel by rememberSaveable(server.url) { mutableStateOf("") }
    var settingsFromInfo by remember { mutableStateOf(false) }
    val canAdmin = server.capabilities.canAdmin
    val dimensions = LocalUiDimensions.current
    GlassPill(modifier.fillMaxWidth(), radius = 24) {
        Row(
            Modifier.fillMaxWidth()
                .height(dimensions.headerHeight)
                .testTag("server-header")
                .clickable(onClickLabel = ui(R.string.server_information)) { panel = "info" }
                .padding(horizontal = dimensions.surfaceInset),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(Modifier.size(dimensions.touchSize), contentAlignment = Alignment.Center) {
                ServerIcon(server, model, Modifier.size(dimensions.headerIcon))
            }
            Text(
                server.name,
                Modifier.weight(1f).testTag("server-header-name"),
                style = MaterialTheme.typography.titleMedium,
                textAlign = TextAlign.Center,
                fontWeight = FontWeight.SemiBold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (canAdmin)
                IconButton(
                    onClick = {
                        settingsFromInfo = false
                        panel = "settings"
                    }
                ) {
                    Icon(Icons.Outlined.Settings, ui(R.string.server_settings))
                }
            else Spacer(Modifier.size(dimensions.touchSize))
        }
    }
    if (panel == "info")
        ServerInfoSheet(
            server,
            model,
            appError,
            onDismiss = { panel = "" },
            onAdmin = {
                settingsFromInfo = true
                panel = "settings"
            },
        )
    if (panel == "settings" && canAdmin)
        ServerSettingsSheet(server, model, appError) {
            panel = if (settingsFromInfo) "info" else ""
        }
    LaunchedEffect(canAdmin) { if (!canAdmin && panel == "settings") panel = "" }
}

@Composable
private fun ServerInfoSheet(
    server: ServerState,
    model: PapoViewModel,
    appError: String,
    onDismiss: () -> Unit,
    onAdmin: () -> Unit,
) {
    BackHandler(onBack = onDismiss)
    val clipboard = LocalClipboardManager.current
    val scope = rememberCoroutineScope()
    val notificationPreferences =
        remember(server.url, server.me) { NotificationPolicyUi(model, scope) }
    var copied by remember { mutableStateOf(false) }
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        LazyColumn(
            Modifier.fillMaxWidth()
                .heightIn(max = 640.dp)
                .navigationBarsPadding()
                .testTag("server-info"),
            contentPadding = PaddingValues(bottom = 24.dp),
        ) {
            item {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    ServerIcon(server, model)
                    Column(Modifier.weight(1f)) {
                        Text(
                            server.name,
                            style = MaterialTheme.typography.titleLarge,
                            fontWeight = FontWeight.SemiBold,
                        )
                        Text(
                            ui(
                                R.string.online_people,
                                server.members.count { it.presence == "online" },
                                administrationPeople(server.members.size),
                            ),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    if (server.capabilities.canAdmin)
                        IconButton(onClick = onAdmin) {
                            Icon(Icons.Outlined.Settings, ui(R.string.server_settings))
                        }
                    IconButton(onClick = onDismiss) {
                        Icon(Icons.Outlined.Close, ui(R.string.close_server_information))
                    }
                }
            }
            item { CommunitySection(ui(R.string.address)) }
            item {
                ListItem(
                    headlineContent = {
                        Text(
                            server.url.removePrefix("https://").removePrefix("http://"),
                            style = MaterialTheme.typography.bodyMedium,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                    },
                    supportingContent =
                        if (copied) ({ Text(ui(R.string.copied_address)) }) else null,
                    trailingContent = {
                        Icon(Icons.Outlined.ContentCopy, ui(R.string.copy_server_address))
                    },
                    colors =
                        ListItemDefaults.colors(
                            containerColor = MaterialTheme.colorScheme.surfaceContainerLow
                        ),
                    modifier =
                        Modifier.clickable {
                            clipboard.setText(AnnotatedString(server.url))
                            copied = true
                        },
                )
            }
            channelNotificationPreferences(server, notificationPreferences)
            item {
                HorizontalDivider(Modifier.padding(vertical = 12.dp))
                TextButton(
                    onClick = { model.command(Action.MarkServerRead, server.url) },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Icon(Icons.Outlined.DoneAll, null)
                    Spacer(Modifier.width(8.dp))
                    Text(ui(R.string.mark_server_as_read))
                }
            }
            if (appError.isNotBlank() || server.error.isNotBlank())
                item { CommunityError(appError.ifBlank { server.error }) }
        }
    }
}

@Composable
private fun CommunitySection(title: String) {
    HorizontalDivider(Modifier.padding(top = 16.dp, bottom = 12.dp))
    Text(
        title,
        Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun CommunityError(error: String) {
    Text(
        error,
        Modifier.padding(20.dp),
        color = MaterialTheme.colorScheme.error,
        style = MaterialTheme.typography.bodyMedium,
    )
}
