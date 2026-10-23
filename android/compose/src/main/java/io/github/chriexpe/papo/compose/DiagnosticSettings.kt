@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

@Composable
internal fun DiagnosticSettings(
    server: ServerState,
    model: PapoViewModel,
    feedback: (String) -> Unit,
) {
    val scope = rememberCoroutineScope()
    val target = LocalSettingTarget.current
    val list = rememberLazyListState()
    LaunchedEffect(target) {
        val index =
            when (target) {
                "reconnect" -> 1
                "cache" -> 3
                "version",
                "android" -> 6
                else -> -1
            }
        if (index >= 0) list.scrollToItem(index)
    }
    var bytes by remember { mutableLongStateOf(model.mediaCache.size) }
    var clearing by remember { mutableStateOf(false) }
    var confirm by remember { mutableStateOf(false) }
    val context = LocalContext.current
    LaunchedEffect(Unit) {
        while (true) {
            bytes = model.mediaCache.size
            kotlinx.coroutines.delay(2000)
        }
    }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current),
        state = list,
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item { SettingsLabel(ui(R.string.connection)) }
        item {
            SettingsCard {
                ListItem(
                    headlineContent = { Text(server.name) },
                    supportingContent = {
                        Text(
                            if (server.isDemo) ui(R.string.offline_demo_no_server_connection)
                            else
                                "${uiLabel(server.connection.replaceFirstChar(Char::uppercase))}\n${server.url}"
                        )
                    },
                    leadingContent = { Icon(Icons.Outlined.Cloud, null) },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                )
                if (!server.isDemo) {
                    SettingsDivider()
                    SettingsLink(
                        ui(R.string.reconnect),
                        ui(R.string.refresh_the_connection_and_server_state),
                        Icons.Outlined.Refresh,
                        { model.command(io.github.chriexpe.papo.core.Action.Refresh, server.url) },
                        target = "reconnect",
                    )
                }
            }
        }
        item { SettingsLabel(ui(R.string.media_cache)) }
        item {
            SettingsCard {
                ListItem(
                    headlineContent = {
                        Text(android.text.format.Formatter.formatFileSize(context, bytes))
                    },
                    supportingContent = {
                        Text(ui(R.string.label_256_mb_shared_limit_oldest_media_is_removed_first))
                    },
                    leadingContent = { Icon(Icons.Outlined.Storage, null) },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                )
                SettingsDivider()
                ListItem(
                    headlineContent = { Text(ui(R.string.clear_media_cache)) },
                    supportingContent = {
                        Text(
                            ui(
                                R.string
                                    .images_avatars_stickers_audio_and_video_can_be_fetched_again
                            )
                        )
                    },
                    trailingContent = {
                        if (clearing) CircularProgressIndicator(Modifier.size(24.dp))
                        else Icon(Icons.Outlined.DeleteOutline, null)
                    },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    modifier =
                        Modifier.then(settingTarget("cache")).clickable(enabled = !clearing) {
                            confirm = true
                        },
                )
            }
        }
        item {
            Text(
                ui(R.string.clear_cache_keeps_account_data),
                Modifier.padding(horizontal = 8.dp),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        item { SettingsLabel(ui(R.string.about)) }
        item {
            SettingsCard {
                ListItem(
                    modifier = Modifier.then(settingTarget("version")),
                    headlineContent = { Text(ui(R.string.papo_for_android)) },
                    supportingContent = {
                        Text(
                            ui(
                                R.string.label_1_s_native_compose_papo_core,
                                BuildConfig.VERSION_NAME,
                            )
                        )
                    },
                    leadingContent = { Icon(Icons.Outlined.Forum, null) },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                )
                SettingsDivider()
                ListItem(
                    headlineContent = { Text("Android ${Build.VERSION.RELEASE}") },
                    supportingContent = { Text("API ${Build.VERSION.SDK_INT}") },
                    leadingContent = { Icon(Icons.Outlined.Android, null) },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                )
                SettingsDivider()
                SettingsLink(
                    ui(R.string.android_app_settings),
                    ui(R.string.permissions_storage_and_system_preferences),
                    Icons.Outlined.Settings,
                    {
                        try {
                            context.startActivity(
                                Intent(
                                    Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                                    Uri.parse("package:${context.packageName}"),
                                )
                            )
                        } catch (e: Exception) {
                            feedback(e.message ?: ui(R.string.couldn_t_open_android_settings))
                        }
                    },
                    target = "android",
                )
            }
        }
    }
    if (confirm)
        AlertDialog(
            onDismissRequest = { confirm = false },
            title = { Text(ui(R.string.clear_cached_media)) },
            text = {
                Text(ui(R.string.clear_cache_confirmation))
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirm = false
                        clearing = true
                        scope.launch {
                            try {
                                model.mediaCache.clear()
                                MediaImages.loader(context).memoryCache?.clear()
                                bytes = model.mediaCache.size
                                feedback(ui(R.string.media_cache_cleared))
                            } catch (e: CancellationException) {
                                throw e
                            } catch (e: Exception) {
                                feedback(e.message ?: ui(R.string.couldn_t_clear_cache))
                            } finally {
                                clearing = false
                            }
                        }
                    }
                ) {
                    Text(ui(R.string.clear_cache))
                }
            },
            dismissButton = {
                TextButton(onClick = { confirm = false }) { Text(ui(R.string.cancel)) }
            },
        )
}
