package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.AppearanceChange

@Composable
internal fun AccountFontSetting(
    server: ServerState,
    model: PapoViewModel,
    busy: Boolean,
    previewSelection: AccountFontSize? = null,
    onPreviewChanged: (AccountFontSize) -> Unit = {},
) {
    val selected = previewSelection ?: LocalClientPreferences.current.messageAppearance(server).font
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
                            onPreviewChanged(choice)
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
internal fun AccountDensitySetting(
    server: ServerState,
    model: PapoViewModel,
    busy: Boolean,
    previewSelection: MessageDensity? = null,
    onPreviewChanged: (MessageDensity) -> Unit = {},
) {
    val preferences = LocalClientPreferences.current
    val selected = previewSelection ?: preferences.messageAppearance(server).density
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
                            onPreviewChanged(choice)
                            preferences.setExtraCompact(
                                server,
                                choice.localPreview,
                                stacked =
                                    choice == MessageDensity.ExtraCompactStacked ||
                                        choice == MessageDensity.ExtraCompactStackedGrouped,
                                grouped = choice == MessageDensity.ExtraCompactStackedGrouped,
                            )
                            if (!choice.localPreview)
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
            Text(
                ui(R.string.extra_compact_description),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/**
 * Message text-size and density have independent editors and independent live samples. Highlight
 * colors remain in their existing editor unchanged.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun AccountAppearanceEditor(
    server: ServerState,
    model: PapoViewModel,
    busy: Boolean,
    density: Boolean,
    onDismiss: () -> Unit,
) {
    val prefs = LocalClientPreferences.current
    val current = prefs.messageAppearance(server)
    var previewFont by remember(server.url, server.me) { mutableStateOf(current.font) }
    var previewDensity by remember(server.url, server.me) { mutableStateOf(current.density) }
    val sample = MessageAppearance(previewFont, previewDensity)
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheet,
    ) {
        Column(
            // Wrap to the required content height, up to 80% of the screen.
            // Only oversized content scrolls; Material 3 bottom-anchors the sheet.
            Modifier.fillMaxWidth()
                .heightIn(max = LocalConfiguration.current.screenHeightDp.dp * .80f)
                .navigationBarsPadding()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 12.dp)
                .testTag(if (density) "density-editor" else "font-editor"),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                ui(if (density) R.string.message_density else R.string.account_text_size),
                style = MaterialTheme.typography.titleLarge,
            )
            Text(
                ui(R.string.highlight_preview),
                style = MaterialTheme.typography.titleSmall,
            )
            SettingsCard {
                AccountMessagePreview(
                    sample,
                    showAvatars = prefs.avatars,
                    showTimestamps = prefs.timestamps,
                )
            }
            if (density)
                AccountDensitySetting(
                    server,
                    model,
                    busy,
                    previewSelection = previewDensity,
                    onPreviewChanged = { previewDensity = it },
                )
            else
                AccountFontSetting(
                    server,
                    model,
                    busy,
                    previewSelection = previewFont,
                    onPreviewChanged = { previewFont = it },
                )
            val status = model.accountAppearance.states[server.appearanceOwner()]
            if (status != null) AppearanceSyncStatus(server, model, status)
        }
    }
}

/**
 * A chat-message sample, without a fake composer pill. Uses the same text sizing, spacing and
 * continuation-author rules as ConversationMessage.
 */
@Composable
internal fun AccountMessagePreview(
    appearance: MessageAppearance,
    showAvatars: Boolean,
    showTimestamps: Boolean,
) {
    val typography = remember(appearance.font) { messageTypography(appearance.font) }
    val content =
        listOf(
            ui(R.string.a_little_more_like_you),
            ui(R.string.appearance_preview_second),
            ui(R.string.appearance_preview_third),
        )
    Column(
        Modifier.fillMaxWidth().padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(appearance.contentGap),
    ) {
        Text(
            "# design-ui",
            style = typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (appearance.extraCompact) {
            Column(verticalArrangement = Arrangement.spacedBy(appearance.continuationGap)) {
                content.forEachIndexed { index, body ->
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                        verticalAlignment = Alignment.Top,
                    ) {
                        if (showTimestamps)
                            CompactMessageTime("14:02", index == 0, false, "appearance-$index")
                        if (appearance.inlineAuthor) {
                            Row {
                                Text(
                                    "Ana  ",
                                    style = typography.bodyLarge,
                                    fontWeight = FontWeight.SemiBold,
                                )
                                Text(
                                    body,
                                    style = typography.bodyLarge,
                                    lineHeight = appearance.lineHeight,
                                )
                            }
                        } else
                            Column(
                                verticalArrangement = Arrangement.spacedBy(appearance.headingGap)
                            ) {
                                if (index == 0 || !appearance.suppressRepeatedStackedAuthor)
                                    Text(
                                        "Ana",
                                        style = typography.bodyLarge,
                                        fontWeight = FontWeight.SemiBold,
                                    )
                                Text(
                                    body,
                                    style = typography.bodyLarge,
                                    lineHeight = appearance.lineHeight,
                                )
                            }
                    }
                }
            }
        } else
            Row(
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.Top,
            ) {
                if (showAvatars) Avatar("Ana")
                Column(verticalArrangement = Arrangement.spacedBy(appearance.headingGap)) {
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            "Ana",
                            style = typography.bodyLarge,
                            fontWeight = FontWeight.SemiBold,
                        )
                        if (showTimestamps)
                            Text(
                                "14:02",
                                style = typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                    }
                    Column(verticalArrangement = Arrangement.spacedBy(appearance.continuationGap)) {
                        content.forEach { message ->
                            Text(
                                message,
                                style = typography.bodyLarge,
                                lineHeight = appearance.lineHeight,
                            )
                        }
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
