@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource

internal data class AppearanceSwitch(
    val key: String,
    val title: String,
    val summary: String,
    val default: Boolean = true,
)

internal val appearanceSwitches
    get() =
        listOf(
            AppearanceSwitch(
                "catch-up-hourglass",
                ui(R.string.catch_up_hourglass),
                ui(R.string.show_skipped_unread_blocks_and_their_navigation_controls),
            ),
            AppearanceSwitch(
                "open-newest",
                ui(R.string.open_at_newest_message),
                ui(R.string.turn_off_to_return_to_where_you_left_off),
            ),
            AppearanceSwitch(
                "avatars",
                ui(R.string.show_avatars),
                ui(R.string.keep_profile_pictures_beside_messages),
            ),
            AppearanceSwitch(
                "timestamps",
                ui(R.string.show_timestamps),
                ui(R.string.display_the_time_beside_each_author),
            ),
            AppearanceSwitch(
                "animations",
                ui(R.string.animate_gifs_and_stickers),
                ui(R.string.turn_off_motion_while_keeping_images_visible),
            ),
            AppearanceSwitch(
                "description",
                ui(R.string.show_channel_description_on_open),
                ui(R.string.expand_it_for_six_seconds_tap_to_keep_it_open),
                false,
            ),
        )

@Composable
internal fun AppearanceSettings(server: ServerState, model: PapoViewModel) {
    val prefs = LocalClientPreferences.current
    val target = LocalSettingTarget.current
    val list = rememberLazyListState()
    val messageAppearance = LocalMessageAppearance.current
    val status =
        model.accountAppearance.states[server.appearanceOwner()] ?: AccountAppearanceStatus()
    LaunchedEffect(server.url, server.me) { model.accountAppearance.refresh(server, force = true) }
    val order =
        listOf(
            "preview",
            "preview-note",
            "theme-label",
            "theme",
            "glass",
            "ui-density",
            "account-note",
            "text-size",
            "message-density",
            "appearance-sync",
            "conversation-label",
        ) + appearanceSwitches.map { it.key }
    LaunchedEffect(target) {
        val index = order.indexOf(target)
        if (index >= 0) list.scrollToItem(index)
    }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("appearance-settings"),
        state = list,
        contentPadding =
            PaddingValues(
                LocalUiDimensions.current.screenPadding,
                8.dp,
                LocalUiDimensions.current.screenPadding,
                40.dp,
            ),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        item("preview") {
            SettingsCard {
                Column(
                    Modifier.padding(LocalUiDimensions.current.screenPadding),
                    verticalArrangement = Arrangement.spacedBy(messageAppearance.contentGap),
                ) {
                    Text(
                        "# design-ui",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                        verticalAlignment = Alignment.Top,
                    ) {
                        if (prefs.avatars) Avatar("Ana")
                        Column(
                            verticalArrangement = Arrangement.spacedBy(messageAppearance.headingGap)
                        ) {
                            Row(
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Text("Ana", fontWeight = FontWeight.SemiBold)
                                if (prefs.timestamps)
                                    Text(
                                        "14:02",
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                            }
                            Column(
                                verticalArrangement =
                                    Arrangement.spacedBy(messageAppearance.continuationGap)
                            ) {
                                Text(
                                    ui(R.string.a_little_more_like_you),
                                    lineHeight = messageAppearance.lineHeight,
                                )
                                Text(
                                    ui(R.string.appearance_preview_second),
                                    lineHeight = messageAppearance.lineHeight,
                                )
                                Text(
                                    ui(R.string.appearance_preview_third),
                                    lineHeight = messageAppearance.lineHeight,
                                )
                            }
                        }
                    }
                    GlassPill {
                        Text(
                            ui(R.string.message_channel, "design-ui"),
                            Modifier.padding(14.dp),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
        item("preview-note") {
            Text(
                ui(R.string.a_live_preview_of_your_theme_and_conversation_layout),
                Modifier.padding(horizontal = 8.dp, vertical = 8.dp),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        item("theme-label") { SettingsLabel("Theme") }
        item("theme") {
            SettingsCard {
                Column(Modifier.then(settingTarget("theme")).padding(16.dp)) {
                    Text(ui(R.string.theme), style = MaterialTheme.typography.titleMedium)
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        listOf("System", "Light", "Dark").forEach { choice ->
                            FilterChip(
                                prefs.theme == choice,
                                { prefs.set("theme", choice) },
                                label = { Text(uiLabel(choice)) },
                            )
                        }
                    }
                }
            }
        }
        item("glass") {
            SettingsCard {
                SettingsSwitch(
                    ui(R.string.frosted_controls),
                    ui(R.string.blur_the_content_behind_floating_bars_and_pills),
                    prefs.glass,
                    "glass",
                ) {
                    prefs.set("glass", it)
                }
            }
        }
        item("ui-density") {
            SettingsCard {
                Column(Modifier.then(settingTarget("ui-density")).padding(16.dp)) {
                    Text(ui(R.string.ui_density), style = MaterialTheme.typography.titleMedium)
                    Text(
                        ui(R.string.ui_density_description),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        UiDensity.entries.forEach { density ->
                            FilterChip(
                                prefs.uiDensity == density,
                                { prefs.set("ui-density", density.value) },
                                label = { Text(density.label) },
                                modifier = Modifier.testTag("ui-density-${density.value}"),
                            )
                        }
                    }
                }
            }
        }
        item("account-note") {
            Text(
                if (server.isDemo) ui(R.string.appearance_demo_scope)
                else ui(R.string.appearance_account_scope, server.name),
                Modifier.padding(8.dp),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        item("text-size") { AccountFontSetting(server, model, status.busy) }
        item("message-density") { AccountDensitySetting(server, model, status.busy) }
        item("appearance-sync") { AppearanceSyncStatus(server, model, status) }
        item("conversation-label") { SettingsLabel(ui(R.string.conversations)) }
        items(appearanceSwitches, key = { it.key }) { entry ->
            SettingsCard {
                SettingsSwitch(
                    entry.title,
                    entry.summary,
                    prefs.boolean(entry.key, entry.default),
                    entry.key,
                ) {
                    prefs.set(entry.key, it)
                }
            }
        }
    }
}
