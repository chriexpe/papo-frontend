package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.AppearanceChange

@Composable
internal fun AccountFontSetting(server: ServerState, model: PapoViewModel, busy: Boolean) {
    val selected = LocalClientPreferences.current.messageAppearance(server).font
    SettingsCard {
        Column(Modifier.then(settingTarget("text-size")).padding(16.dp)) {
            Text(ui(R.string.account_text_size), style = MaterialTheme.typography.titleMedium)
            Text(
                ui(R.string.account_text_size_description),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                AccountFontSize.entries.forEach { choice ->
                    FilterChip(
                        selected == choice,
                        {
                            model.accountAppearance.change(
                                server,
                                AppearanceChange.FontSize(choice.value),
                            )
                        },
                        label = { Text(ui(choice.labelId)) },
                        enabled = !busy,
                        modifier = Modifier.testTag("account-font-${choice.value}"),
                    )
                }
            }
        }
    }
}

@Composable
internal fun AccountDensitySetting(server: ServerState, model: PapoViewModel, busy: Boolean) {
    val selected = LocalClientPreferences.current.messageAppearance(server).density
    SettingsCard {
        Column(Modifier.then(settingTarget("message-density")).padding(16.dp)) {
            Text(ui(R.string.message_density), style = MaterialTheme.typography.titleMedium)
            Text(
                ui(R.string.message_density_description),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                MessageDensity.entries.forEach { choice ->
                    FilterChip(
                        selected == choice,
                        {
                            model.accountAppearance.change(
                                server,
                                AppearanceChange.MessageDensity(choice.value),
                            )
                        },
                        label = { Text(ui(choice.labelId)) },
                        enabled = !busy,
                        modifier = Modifier.testTag("message-density-${choice.value}"),
                    )
                }
            }
        }
    }
}

@Composable
internal fun AppearanceSyncStatus(
    server: ServerState,
    model: PapoViewModel,
    status: AccountAppearanceStatus,
) {
    if (status.busy) {
        Column(Modifier.padding(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            LinearProgressIndicator(Modifier.fillMaxWidth())
            Text(ui(R.string.appearance_syncing), style = MaterialTheme.typography.bodySmall)
        }
    } else
        status.error?.let { error ->
            Column(Modifier.padding(8.dp)) {
                Text(ui(error), color = MaterialTheme.colorScheme.error)
                TextButton(onClick = { model.accountAppearance.retry(server) }) {
                    Text(ui(R.string.retry))
                }
            }
        }
}
