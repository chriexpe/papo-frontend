@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.Action

/** Channel context actions. Uses the same bottom-sheet pattern as message actions. */
@Composable
internal fun ChannelActionsSheet(
    server: ServerState,
    model: PapoViewModel,
    channel: Conversation,
    onDismiss: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val preferences = remember(model, scope) { NotificationPolicyUi(model, scope) }
    val actual = server.channels.firstOrNull { it.id == channel.id } ?: channel
    var policies by remember(channel.id) { mutableStateOf(false) }
    var mutes by remember(channel.id) { mutableStateOf(false) }
    val muteRevision by MessageAlerts.muteChanges.collectAsState()
    val mutedUntil = remember(muteRevision, server.url, server.me, channel.id) {
        MessageAlerts.muteUntil(server.url, server.me, channel.id)
    }
    val mode = when (actual.notificationSettings) {
        "all" -> ui(R.string.all_messages)
        "off" -> ui(R.string.notification_policy_nothing)
        else -> ui(R.string.mentions_only)
    }

    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier.fillMaxWidth()
                .navigationBarsPadding()
                .padding(horizontal = 16.dp)
                .padding(bottom = 16.dp)
                .testTag("channel-actions"),
        ) {
            Text(
                if (channel.kind == "voice") channel.name else "#${channel.name}",
                Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                style = MaterialTheme.typography.titleMedium,
            )
            ListItem(
                headlineContent = { Text(ui(R.string.notifications)) },
                supportingContent = { Text(mode) },
                leadingContent = { Icon(Icons.Outlined.Notifications, null) },
                trailingContent = { Icon(Icons.Outlined.ChevronRight, null) },
                modifier = Modifier.testTag("channel-policy-menu"),
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(onClick = { policies = !policies }) {
                    Text(if (policies) ui(R.string.close) else ui(R.string.change_notification_policy))
                }
            }
            if (policies)
                listOf(
                    "all" to R.string.all_messages,
                    "only_mentions" to R.string.mentions_only,
                    "off" to R.string.notification_policy_nothing,
                ).forEach { (value, label) ->
                    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
                        RadioButton(
                            selected = actual.notificationSettings == value,
                            onClick = {
                                preferences.apply(server, listOf(channel.id), value)
                                policies = false
                            },
                            enabled = !preferences.busy,
                        )
                        TextButton(
                            enabled = !preferences.busy,
                            onClick = {
                                preferences.apply(server, listOf(channel.id), value)
                                policies = false
                            },
                            modifier = Modifier.weight(1f),
                        ) { Text(ui(label)) }
                    }
                }
            if (preferences.error.isNotBlank())
                Text(preferences.error, color = MaterialTheme.colorScheme.error)
            HorizontalDivider(Modifier.padding(vertical = 8.dp))
            ListItem(
                headlineContent = { Text(ui(R.string.mute_channel)) },
                supportingContent = {
                    if (mutedUntil > 0L) Text(ui(R.string.channel_muted_state))
                },
                leadingContent = { Icon(Icons.Outlined.NotificationsOff, null) },
                modifier = Modifier.testTag("channel-mute-menu"),
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(onClick = { mutes = !mutes }) {
                    Text(if (mutes) ui(R.string.close) else ui(R.string.change_mute))
                }
            }
            if (mutes) {
                if (mutedUntil > 0L)
                    TextButton(onClick = { MessageAlerts.setMute(server.url, server.me, channel.id, 0L); mutes = false }) {
                        Text(ui(R.string.unmute_channel))
                    }
                else
                    listOf(
                        R.string.mute_15_minutes to 15L * 60_000,
                        R.string.mute_1_hour to 60L * 60_000,
                        R.string.mute_8_hours to 8L * 60 * 60_000,
                        R.string.mute_24_hours to 24L * 60 * 60_000,
                        R.string.mute_until_unmuted to MessageAlerts.MUTED_FOREVER,
                    ).forEach { (label, duration) ->
                        TextButton(
                            onClick = {
                                val until = if (duration == MessageAlerts.MUTED_FOREVER) duration
                                    else System.currentTimeMillis() + duration
                                MessageAlerts.setMute(server.url, server.me, channel.id, until)
                                mutes = false
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) { Text(ui(label)) }
                    }
            }
            HorizontalDivider(Modifier.padding(vertical = 8.dp))
            TextButton(
                onClick = {
                    model.command(Action.MarkChannelRead(channel.id, server.me), server.url)
                    MessageAlerts.clear(server.url, server.me, channel.id)
                    onDismiss()
                },
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("channel-mark-read"),
            ) {
                Icon(Icons.Outlined.DoneAll, null)
                Spacer(Modifier.width(12.dp))
                Text(ui(R.string.mark_channel_as_read), Modifier.weight(1f))
            }
        }
    }
}
