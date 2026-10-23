@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import android.content.Intent
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import kotlinx.coroutines.launch

@Composable
internal fun NotificationSettings(server: ServerState) {
    val context = LocalContext.current
    val prefs = LocalClientPreferences.current
    val lifecycle = androidx.lifecycle.compose.LocalLifecycleOwner.current.lifecycle
    var available by remember { mutableStateOf(MessageAlerts.listener.available()) }
    val permission =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
            available = MessageAlerts.listener.available()
            MessageAlerts.refresh()
        }
    LaunchedEffect(lifecycle) {
        lifecycle.currentStateFlow.collect {
            if (it.isAtLeast(androidx.lifecycle.Lifecycle.State.RESUMED)) {
                available = MessageAlerts.listener.available()
                MessageAlerts.refresh()
            }
        }
    }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("notification-settings"),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 40.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item {
            SettingsCard {
                SettingsSwitch(
                    ui(R.string.message_notifications),
                    ui(R.string.notifications_description),
                    prefs.boolean("notifications-enabled"),
                    "notifications-enabled",
                ) {
                    prefs.set("notifications-enabled", it)
                    available = MessageAlerts.listener.available()
                }
                SettingsDivider()
                SettingsSwitch(
                    ui(R.string.reply_notifications_default),
                    ui(R.string.reply_notifications_default_description),
                    prefs.replyNotifications,
                    "reply-notifications",
                ) {
                    prefs.set("reply-notifications", it)
                }
            }
        }
        item {
            SettingsCard {
                SettingsLink(
                    if (available) ui(R.string.android_notifications_allowed)
                    else ui(R.string.allow_android_notifications),
                    ui(R.string.manage_permission_sound_and_notification_channels),
                    Icons.Outlined.NotificationsActive,
                    {
                        if (
                            Build.VERSION.SDK_INT >= 33 &&
                                context.checkSelfPermission(
                                    android.Manifest.permission.POST_NOTIFICATIONS
                                ) != android.content.pm.PackageManager.PERMISSION_GRANTED
                        )
                            permission.launch(android.Manifest.permission.POST_NOTIFICATIONS)
                        else
                            context.startActivity(
                                Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                                    .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
                            )
                    },
                    target = "notification-permission",
                )
                SettingsDivider()
                SettingsLink(
                    ui(R.string.android_notification_settings),
                    ui(R.string.choose_sound_vibration_and_lock_screen_visibility),
                    Icons.Outlined.Settings,
                    {
                        context.startActivity(
                            Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                                .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
                        )
                    },
                )
            }
        }
        item {
            SettingsCard {
                Column(
                    Modifier.then(settingTarget("notification-delivery")).padding(20.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        ui(R.string.background_delivery),
                        style = MaterialTheme.typography.titleMedium,
                    )
                    Text(
                        when {
                            !prefs.boolean("notifications-enabled") ->
                                ui(R.string.notifications_are_off_on_this_device)
                            !available -> ui(R.string.allow_android_notifications_to_receive_alerts)
                            !server.notificationsEnabled ->
                                ui(R.string.server_notifications_disabled)
                            server.pushRegistered ->
                                ui(R.string.push_connected_messages_can_arrive_while_papo_is_closed)
                            MessageAlerts.configured -> ui(R.string.push_registration_pending)
                            else -> ui(R.string.push_unavailable_description)
                        },
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(
                        ui(R.string.notification_preferences_description),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}
