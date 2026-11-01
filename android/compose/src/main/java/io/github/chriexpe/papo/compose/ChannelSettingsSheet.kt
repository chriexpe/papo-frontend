package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.Action

/** Per-channel shortcuts reached by long-pressing a channel in the list. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ChannelSettingsSheet(
    server: ServerState,
    model: PapoViewModel,
    channel: Conversation,
    onOpenChat: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val policy = remember(model, scope) { NotificationPolicyUi(model, scope) }
    // The live channel reflects the acknowledged setting once the core polls again.
    val live = server.channels.firstOrNull { it.id == channel.id } ?: channel
    val acknowledged = live.notificationSettings.ifBlank { "only_mentions" }
    // Show the tapped choice immediately; keep it until the server value catches up, and drop it
    // (reverting to the acknowledged value) when the request fails.
    var pending by remember { mutableStateOf<String?>(null) }
    val failed = policy.error.isNotBlank() || policy.result?.failed?.isNotEmpty() == true
    LaunchedEffect(policy.busy, acknowledged, failed) {
        if (!policy.busy && (failed || acknowledged == pending)) pending = null
    }
    val current = pending ?: acknowledged
    // Opening the modal shows what the account says now, not what this device last fetched.
    LaunchedEffect(server.url, server.me) { model.refreshChannelSettings(server) }
    // Synchronized by default; a stored local policy means this channel stays on this device.
    var sync by
        remember(channel.id) { mutableStateOf(model.localChannelPolicy(server, channel.id) == null) }
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier.fillMaxWidth()
                .navigationBarsPadding()
                .padding(horizontal = 20.dp)
                .padding(bottom = 16.dp)
                .testTag("channel-settings")
        ) {
            Text(ui(R.string.channel_settings), style = MaterialTheme.typography.titleLarge)
            Text(
                if (channel.kind == "voice") channel.name else "#${channel.name}",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(bottom = 12.dp),
            )
            Text(
                ui(R.string.notifications),
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(vertical = 8.dp),
            )
            listOf("all", "only_mentions", "off").forEach { option ->
                Row(
                    Modifier.fillMaxWidth()
                        .heightIn(min = 48.dp)
                        .selectable(
                            selected = current == option,
                            enabled = !policy.busy,
                            role = Role.RadioButton,
                            onClick = {
                                pending = option
                                if (!sync) model.setLocalChannelPolicy(server, channel.id, option)
                                policy.apply(server, listOf(channel.id), option, sync)
                            },
                        )
                        .testTag("channel-notification-$option"),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RadioButton(selected = current == option, onClick = null)
                    Text(
                        notificationPolicyLabel(option),
                        Modifier.padding(start = 12.dp),
                        style = MaterialTheme.typography.bodyLarge,
                    )
                }
            }
            Row(
                Modifier.fillMaxWidth()
                    .heightIn(min = 48.dp)
                    .toggleable(
                        value = sync,
                        enabled = !policy.busy,
                        role = Role.Switch,
                        onValueChange = { enabled ->
                            sync = enabled
                            if (enabled) {
                                // Back on: the account takes the choice shown here.
                                model.setLocalChannelPolicy(server, channel.id, null)
                                policy.apply(server, listOf(channel.id), current, true)
                            } else model.setLocalChannelPolicy(server, channel.id, current)
                        },
                    )
                    .testTag("channel-notification-sync"),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(
                        ui(R.string.sync_notifications),
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    Text(
                        ui(R.string.sync_notifications_description),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Switch(checked = sync, onCheckedChange = null)
            }
            if (policy.error.isNotBlank())
                Text(
                    policy.error,
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                )
            HorizontalDivider(Modifier.padding(vertical = 8.dp))
            MuteSection(server, channel)
            HorizontalDivider(Modifier.padding(vertical = 8.dp))
            if (channel.kind == "voice")
                SettingsAction(Icons.Outlined.ChatBubbleOutline, ui(R.string.open_chat)) {
                    onDismiss()
                    onOpenChat(channel.id)
                }
            SettingsAction(Icons.Outlined.DoneAll, ui(R.string.mark_channel_as_read)) {
                model.command(Action.MarkChannelRead(channel.id, server.me), server.url)
                onDismiss()
            }
        }
    }
}

/** Local mute, by time or until turned back on; it never reaches the account. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun MuteSection(server: ServerState, channel: Conversation) {
    val revision by MessageAlerts.muteChanges.collectAsState()
    val until =
        remember(revision, server.url, server.me, channel.id) {
            MessageAlerts.muteUntil(server.url, server.me, channel.id)
        }
    fun mute(millis: Long) =
        MessageAlerts.setMute(
            server.url,
            server.me,
            channel.id,
            if (millis == MessageAlerts.MUTED_FOREVER) millis
            else System.currentTimeMillis() + millis,
        )
    Text(
        ui(R.string.mute_channel),
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(vertical = 8.dp),
    )
    if (until > 0L) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("channel-muted"),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                if (until == MessageAlerts.MUTED_FOREVER) ui(R.string.muted_until_turned_on)
                else
                    ui(
                        R.string.muted_until,
                        java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT)
                            .format(java.util.Date(until)),
                    ),
                Modifier.weight(1f),
                style = MaterialTheme.typography.bodyMedium,
            )
            TextButton(
                onClick = { MessageAlerts.setMute(server.url, server.me, channel.id, 0L) },
                modifier = Modifier.testTag("channel-unmute"),
            ) {
                Text(ui(R.string.unmute_channel))
            }
        }
    } else {
        val minute = 60_000L
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(
                    R.string.mute_15_minutes to 15 * minute,
                    R.string.mute_1_hour to 60 * minute,
                    R.string.mute_8_hours to 8 * 60 * minute,
                    R.string.mute_24_hours to 24 * 60 * minute,
                    R.string.mute_until_unmuted to MessageAlerts.MUTED_FOREVER,
                )
                .forEach { (label, millis) ->
                    AssistChip(
                        onClick = { mute(millis) },
                        label = { Text(ui(label)) },
                        modifier = Modifier.testTag("channel-mute-$millis"),
                    )
                }
        }
    }
}

@Composable
private fun SettingsAction(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    label: String,
    onClick: () -> Unit,
) {
    TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
        Icon(icon, null)
        Spacer(Modifier.width(12.dp))
        Text(label, Modifier.weight(1f))
    }
}
