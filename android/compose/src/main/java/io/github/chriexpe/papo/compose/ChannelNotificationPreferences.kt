package io.github.chriexpe.papo.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.NotificationPolicyResult
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

internal fun notificationPolicyLabel(value: String) =
    when (value) {
        "all" -> ui(R.string.all_messages)
        "off" -> uiLabel("Off")
        else -> ui(R.string.mentions_only)
    }

@Stable
internal class NotificationPolicyUi(
    private val model: PapoViewModel,
    private val scope: CoroutineScope,
) {
    var busy by mutableStateOf(false)
        private set

    var result by mutableStateOf<NotificationPolicyResult?>(null)
        private set

    var error by mutableStateOf("")
        private set

    var lastPolicy by mutableStateOf("only_mentions")
        private set

    var selected by mutableStateOf("only_mentions")

    fun apply(server: ServerState, ids: List<String>, policy: String) {
        if (busy || ids.isEmpty()) return
        busy = true
        error = ""
        result = null
        lastPolicy = policy
        scope.launch {
            try {
                result = model.notificationPolicy(server, ids, policy)
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                error = ui(R.string.notification_policy_failed)
            } finally {
                busy = false
            }
        }
    }
}

/**
 * The same controls handle individual changes and bulk results; acknowledged settings remain
 * visible when another channel's request is rejected.
 */
internal fun LazyListScope.channelNotificationPreferences(
    server: ServerState,
    state: NotificationPolicyUi,
) {
    val channels = server.channels.filter { it.kind == "text" || it.kind == "voice" }
    if (channels.isEmpty()) return
    item {
        Column(
            Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(ui(R.string.channel_notifications), style = MaterialTheme.typography.titleSmall)
            Text(
                ui(R.string.notification_policy_description),
                style = MaterialTheme.typography.bodySmall,
            )
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                NotificationPolicyMenu(state.selected, !state.busy, Modifier.weight(1f)) {
                    state.selected = it
                }
                FilledTonalButton(
                    enabled = !state.busy,
                    onClick = { state.apply(server, channels.map { it.id }, state.selected) },
                    modifier = Modifier.testTag("apply-notifications-all"),
                ) {
                    if (state.busy) {
                        CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(8.dp))
                        Text(ui(R.string.notification_policy_applying))
                    } else Text(ui(R.string.notification_policy_apply_all))
                }
            }
            state.result?.let { result ->
                val confirmed = result.applied.size
                Text(
                    ui(
                        R.string.notification_policy_result,
                        notificationPolicyLabel(state.lastPolicy),
                        uiQuantity(R.plurals.notification_policy_confirmed, confirmed),
                    ),
                    style = MaterialTheme.typography.bodySmall,
                )
                if (result.failed.isNotEmpty()) {
                    Text(
                        uiQuantity(R.plurals.notification_policy_unconfirmed, result.failed.size),
                        color = MaterialTheme.colorScheme.error,
                    )
                    result.failed.forEach { failure ->
                        val name =
                            server.channels.firstOrNull { it.id == failure.channelId }?.name
                                ?: ui(R.string.conversation_unavailable)
                        val reason =
                            ui(
                                when (failure.reason) {
                                    "missing" -> R.string.notification_policy_missing
                                    "denied" -> R.string.notification_policy_denied
                                    "rate_limited" -> R.string.notification_policy_rate_limited
                                    else -> R.string.notification_policy_unconfirmed_reason
                                }
                            )
                        Text(
                            ui(R.string.notification_policy_failure_item, name, reason),
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                    TextButton(
                        enabled = !state.busy,
                        onClick = {
                            state.apply(
                                server,
                                result.failed.map { it.channelId },
                                state.lastPolicy,
                            )
                        },
                    ) {
                        Text(ui(R.string.notification_policy_retry_failed))
                    }
                }
            }
            if (state.error.isNotBlank()) Text(state.error, color = MaterialTheme.colorScheme.error)
        }
    }
    items(channels, key = { it.id }) { channel ->
        var expanded by remember { mutableStateOf(false) }
        ListItem(
            headlineContent = {
                Text(channel.name, maxLines = 2, overflow = TextOverflow.Ellipsis)
            },
            leadingContent = {
                Icon(
                    if (channel.kind == "voice") Icons.Outlined.VolumeUp else Icons.Outlined.Tag,
                    null,
                )
            },
            trailingContent = {
                Box {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            notificationPolicyLabel(channel.notificationSettings),
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Icon(Icons.Outlined.ChevronRight, null, Modifier.size(20.dp))
                    }
                    PolicyChoices(expanded, { expanded = false }, channel.notificationSettings) {
                        value ->
                        expanded = false
                        state.apply(server, listOf(channel.id), value)
                    }
                }
            },
            colors =
                ListItemDefaults.colors(
                    containerColor = MaterialTheme.colorScheme.surfaceContainerLow
                ),
            modifier =
                Modifier.testTag("channel-notifications-${channel.id}").clickable(
                    enabled = !state.busy
                ) {
                    expanded = true
                },
        )
    }
}

@Composable
private fun NotificationPolicyMenu(
    value: String,
    enabled: Boolean,
    modifier: Modifier,
    onChange: (String) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    Box(modifier) {
        OutlinedButton(
            enabled = enabled,
            onClick = { expanded = true },
            modifier = Modifier.fillMaxWidth().testTag("notification-policy-choice"),
        ) {
            Text(
                notificationPolicyLabel(value),
                Modifier.weight(1f),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Icon(Icons.Outlined.ExpandMore, null)
        }
        PolicyChoices(expanded, { expanded = false }, value) {
            expanded = false
            onChange(it)
        }
    }
}

@Composable
private fun PolicyChoices(
    expanded: Boolean,
    onDismiss: () -> Unit,
    value: String,
    onSelect: (String) -> Unit,
) {
    DropdownMenu(expanded, onDismissRequest = onDismiss) {
        listOf("all", "only_mentions", "off").forEach { policy ->
            DropdownMenuItem(
                text = { Text(notificationPolicyLabel(policy)) },
                leadingIcon = { if (value == policy) Icon(Icons.Outlined.Check, null) },
                onClick = { onSelect(policy) },
            )
        }
    }
}
