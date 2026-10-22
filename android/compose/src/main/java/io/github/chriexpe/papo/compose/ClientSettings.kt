@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.DocumentsContract
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.AccountChange
import io.github.chriexpe.papo.core.AccountProfile
import io.github.chriexpe.papo.core.SessionView
import java.time.OffsetDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private data class SettingsDestination(
    val title: String,
    val subtitle: String,
    val icon: ImageVector,
)

private val settingsDestinations
    get() =
        listOf(
            SettingsDestination(
                "Appearance",
                ui(R.string.theme_glass_and_conversations),
                Icons.Outlined.Palette,
            ),
            SettingsDestination(
                "Notifications",
                ui(R.string.messages_mentions_and_android_delivery),
                Icons.Outlined.NotificationsNone,
            ),
            SettingsDestination(
                "Language",
                ui(R.string.portugu_s_brasil_and_english),
                Icons.Outlined.Language,
            ),
            SettingsDestination(
                "Links",
                ui(R.string.confirmation_and_trusted_domains),
                Icons.Outlined.OpenInNew,
            ),
            SettingsDestination(
                "Files",
                ui(R.string.downloads_and_save_location),
                Icons.Outlined.Folder,
            ),
            SettingsDestination(
                "Sessions",
                ui(R.string.manage_signed_in_sessions),
                Icons.Outlined.Devices,
            ),
            SettingsDestination(
                "Diagnostics",
                ui(R.string.connection_media_cache_and_app_info),
                Icons.Outlined.MonitorHeart,
            ),
        )

internal data class SettingEntry(
    val id: String,
    val page: String,
    val title: String,
    val summary: String,
    val keywords: String = "",
)

private data class AppearanceSwitch(
    val key: String,
    val title: String,
    val summary: String,
    val default: Boolean = true,
)

private val appearanceSwitches
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
                "compact",
                ui(R.string.compact_messages),
                ui(
                    R.string
                        .reduce_vertical_spacing_consecutive_messages_still_share_an_author_heading
                ),
                false,
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
internal val LocalSettingTarget = compositionLocalOf { "" }

internal fun searchFold(value: String) =
    java.text.Normalizer.normalize(value, java.text.Normalizer.Form.NFD)
        .replace(Regex("\\p{M}+"), "")
        .lowercase(java.util.Locale.ROOT)

private fun settingsEntries(server: ServerState, prefs: ClientPreferences): List<SettingEntry> =
    buildList {
        add(
            SettingEntry(
                "nickname",
                "Account",
                ui(R.string.display_name),
                ui(R.string.your_nickname_on_1_s, server.name),
                "profile name username",
            )
        )
        add(
            SettingEntry(
                "status",
                "Account",
                ui(R.string.status_message),
                ui(R.string.a_short_message_on_your_profile),
                "profile bio",
            )
        )
        add(
            SettingEntry(
                "about",
                "Account",
                ui(R.string.about_you),
                ui(R.string.your_profile_description),
                "bio biography",
            )
        )
        add(
            SettingEntry(
                "avatar",
                "Account",
                ui(R.string.change_photo),
                ui(R.string.your_profile_picture),
                "avatar image",
            )
        )
        if (!server.isDemo)
            add(
                SettingEntry(
                    "banner",
                    "Account",
                    ui(R.string.change_banner),
                    ui(R.string.your_profile_s_cover_image),
                    "photo picture",
                )
            )
        if (server.profiles.firstOrNull { it.id == server.me }?.banner?.isNotBlank() == true)
            add(
                SettingEntry(
                    "remove-banner",
                    "Account",
                    ui(R.string.remove_banner),
                    ui(R.string.clear_your_profile_s_cover_image),
                )
            )
        add(
            SettingEntry(
                "presence",
                "Account",
                ui(R.string.presence),
                ui(R.string.online_away_or_do_not_disturb),
                "busy status",
            )
        )
        add(
            SettingEntry(
                "typing",
                "Account",
                ui(R.string.typing_phrase),
                ui(R.string.your_custom_typing_indicator),
                "is typing status",
            )
        )
        if (!server.isDemo) {
            add(
                SettingEntry(
                    "password",
                    "Account",
                    ui(R.string.change_password),
                    ui(R.string.choose_a_new_password_for_this_server),
                    "security login",
                )
            )
            add(
                SettingEntry(
                    "logout",
                    "Account",
                    ui(R.string.sign_out),
                    ui(R.string.end_this_account_s_session_on_1_s, server.name),
                    "logout security",
                )
            )
            add(
                SettingEntry(
                    "sessions",
                    "Sessions",
                    ui(R.string.active_sessions),
                    ui(R.string.review_and_end_signed_in_sessions),
                    "devices connections security",
                )
            )
        }
        add(
            SettingEntry(
                "language",
                "Language",
                ui(R.string.language),
                ui(R.string.portugu_s_brasil_and_english),
                ui(R.string.idioma_portugu_s_ingles_english_language),
            )
        )
        add(
            SettingEntry(
                "confirm-external-links",
                "Links",
                ui(R.string.confirm_external_links),
                ui(R.string.ask_before_opening_an_unfamiliar_website),
                "browser links privacy",
            )
        )
        add(
            SettingEntry(
                "trusted-domains",
                "Links",
                ui(R.string.trusted_domains),
                ui(R.string.manage_websites_that_open_without_confirmation),
                "browser links privacy domain",
            )
        )
        add(
            SettingEntry(
                "theme",
                "Appearance",
                ui(R.string.theme),
                uiLabel(prefs.theme),
                "light dark system colors",
            )
        )
        add(
            SettingEntry(
                "glass",
                "Appearance",
                ui(R.string.frosted_controls),
                if (prefs.glass) ui(R.string.on_glass_on_floating_bars_and_pills)
                else ui(R.string.off_opaque_bars_and_pills),
                ui(R.string.translucency_transparency_blur),
            )
        )
        appearanceSwitches.forEach { item ->
            add(
                SettingEntry(
                    item.key,
                    "Appearance",
                    item.title,
                    "${uiLabel(if(prefs.boolean(item.key,item.default)) "On" else "Off")} · ${item.summary}",
                )
            )
        }
        add(
            SettingEntry(
                "notifications-enabled",
                "Notifications",
                ui(R.string.message_notifications),
                ui(R.string.messages_and_mentions_on_this_device),
                "alerts push firebase",
            )
        )
        add(
            SettingEntry(
                "notification-permission",
                "Notifications",
                ui(R.string.android_notification_settings),
                ui(R.string.permission_sound_and_notification_channel),
                ui(R.string.alerts_push),
            )
        )
        add(
            SettingEntry(
                "notification-delivery",
                "Notifications",
                ui(R.string.background_delivery),
                ui(R.string.push_and_periodic_message_checks),
                "firebase",
            )
        )
        add(
            SettingEntry(
                "download-folder",
                "Files",
                ui(R.string.save_media_to),
                prefs.string("download-label").ifBlank { "Downloads/Papo" },
                ui(R.string.download_downloads_folder_files_storage_location),
            )
        )
        if (prefs.string("download-tree").isNotBlank())
            add(
                SettingEntry(
                    "download-reset",
                    "Files",
                    ui(R.string.use_downloads_papo),
                    ui(R.string.return_to_the_default_android_download_folder),
                    "save location reset",
                )
            )
        if (!server.isDemo)
            add(
                SettingEntry(
                    "reconnect",
                    "Diagnostics",
                    ui(R.string.reconnect),
                    ui(R.string.refresh_the_connection_and_server_state),
                    "offline network",
                )
            )
        add(
            SettingEntry(
                "cache",
                "Diagnostics",
                ui(R.string.clear_media_cache),
                ui(R.string.remove_reusable_images_audio_and_video),
                "storage space disk GIF sticker avatar",
            )
        )
        add(
            SettingEntry(
                "version",
                "Diagnostics",
                ui(R.string.papo_for_android),
                BuildConfig.VERSION_NAME,
                "about version build",
            )
        )
        add(
            SettingEntry(
                "android",
                "Diagnostics",
                ui(R.string.android_app_settings),
                ui(R.string.permissions_storage_and_system_preferences),
                "system",
            )
        )
    }

/**
 * Targets share the same controls as ordinary navigation. Bring the exact row into view after the
 * lazy list composes it, then briefly mark the result.
 */
@Composable
internal fun settingTarget(id: String): Modifier {
    val selected = id.isNotBlank() && LocalSettingTarget.current == id
    val requester = remember { BringIntoViewRequester() }
    var highlighted by remember { mutableStateOf(false) }
    LaunchedEffect(selected) {
        if (selected) {
            withFrameNanos {}
            requester.bringIntoView()
            highlighted = true
            kotlinx.coroutines.delay(2200)
            highlighted = false
        }
    }
    return Modifier.bringIntoViewRequester(requester)
        .background(
            if (highlighted) MaterialTheme.colorScheme.primaryContainer else Color.Transparent,
            RoundedCornerShape(14.dp),
        )
}

@Composable
internal fun ClientSettings(
    server: ServerState,
    model: PapoViewModel,
    onBack: () -> Unit,
    onLogout: () -> Unit,
) {
    var page by rememberSaveable(server.url, server.me) { mutableStateOf("Settings") }
    var query by rememberSaveable { mutableStateOf("") }
    var target by rememberSaveable { mutableStateOf("") }
    val prefs = LocalClientPreferences.current
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    fun open(destination: String, id: String = "") {
        focus.clearFocus()
        keyboard?.hide()
        target = id
        page = destination
    }
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val feedback: (String) -> Unit = { message -> scope.launch { snackbar.showSnackbar(message) } }
    val back: () -> Unit = { if (page == "Settings") onBack() else page = "Settings" }
    BackHandler(page != "Settings" && page != "Account") { back() }
    CompositionLocalProvider(LocalSettingTarget provides target) {
        Box(Modifier.fillMaxSize().testTag("client-settings")) {
            if (page == "Account") AccountSettings(server, model, back, onLogout, feedback)
            else
                Column(Modifier.fillMaxSize()) {
                    SettingsHeader(uiLabel(page), back)
                    when (page) {
                        "Settings" ->
                            LazyColumn(
                                Modifier.fillMaxSize().hazeSource(LocalHaze.current),
                                contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
                                verticalArrangement = Arrangement.spacedBy(16.dp),
                            ) {
                                item {
                                    OutlinedTextField(
                                        query,
                                        { query = it },
                                        Modifier.fillMaxWidth().testTag("settings-search"),
                                        placeholder = { Text(ui(R.string.search_settings)) },
                                        leadingIcon = { Icon(Icons.Outlined.Search, null) },
                                        trailingIcon = {
                                            if (query.isNotBlank())
                                                IconButton(onClick = { query = "" }) {
                                                    Icon(
                                                        Icons.Outlined.Close,
                                                        ui(R.string.clear_settings_search),
                                                    )
                                                }
                                        },
                                        singleLine = true,
                                        shape = RoundedCornerShape(28.dp),
                                    )
                                }
                                if (query.isBlank()) {
                                    item {
                                        SettingsCard {
                                            SettingsLink(
                                                server.myName,
                                                ui(R.string.profile_presence_and_security),
                                                Icons.Outlined.Person,
                                                { open("Account") },
                                                leading = { UserAvatar(server.me, server.myName) },
                                            )
                                        }
                                    }
                                    items(
                                        settingsDestinations.filter {
                                            !server.isDemo || it.title != "Sessions"
                                        },
                                        key = { it.title },
                                    ) { entry ->
                                        SettingsCard {
                                            SettingsLink(
                                                uiLabel(entry.title),
                                                entry.subtitle,
                                                entry.icon,
                                                { open(entry.title) },
                                            )
                                        }
                                    }
                                } else {
                                    val words =
                                        searchFold(query).trim().split(Regex("\\s+")).filter {
                                            it.isNotBlank()
                                        }
                                    val matches =
                                        settingsEntries(server, prefs).filter { entry ->
                                            val haystack =
                                                searchFold(
                                                    "${entry.title} ${entry.summary} ${uiLabel(entry.page)} ${entry.page} ${entry.keywords}"
                                                )
                                            words.all { haystack.contains(it) }
                                        }
                                    items(matches, key = { it.id }) { entry ->
                                        SettingsCard {
                                            val icon =
                                                if (entry.page == "Account") Icons.Outlined.Person
                                                else
                                                    settingsDestinations
                                                        .first { it.title == entry.page }
                                                        .icon
                                            SettingsLink(
                                                entry.title,
                                                "${uiLabel(entry.page)} · ${entry.summary}",
                                                icon,
                                                { open(entry.page, entry.id) },
                                            )
                                        }
                                    }
                                    if (matches.isEmpty())
                                        item {
                                            Text(
                                                ui(R.string.no_matching_settings),
                                                Modifier.padding(16.dp),
                                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                            )
                                        }
                                }
                                item {
                                    Text(
                                        ui(
                                            R.string
                                                .account_settings_apply_to_1_s_appearance_and_files_are_saved_on_this_device,
                                            server.name,
                                        ),
                                        Modifier.padding(horizontal = 8.dp),
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                if (server.isDemo)
                                    item {
                                        TextButton(onClick = onLogout) {
                                            Icon(Icons.Outlined.Logout, null)
                                            Spacer(Modifier.width(8.dp))
                                            Text(ui(R.string.leave_demo))
                                        }
                                    }
                            }
                        "Appearance" -> AppearanceSettings()
                        "Notifications" -> NotificationSettings(server)
                        "Language" -> LanguageSettings()
                        "Links" -> LinkSettings()
                        "Files" -> FileSettings(feedback)
                        "Sessions" -> SessionSettings(server, model, feedback)
                        "Diagnostics" -> DiagnosticSettings(server, model, feedback)
                    }
                }
            SnackbarHost(
                snackbar,
                Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(12.dp),
            )
        }
    }
}

@Composable
internal fun SettingsHeader(
    title: String,
    onBack: () -> Unit,
    close: Boolean = false,
    enabled: Boolean = true,
    actions: @Composable RowScope.() -> Unit = {},
) {
    Row(
        Modifier.fillMaxWidth().statusBarsPadding().padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        GlassPill {
            IconButton(enabled = enabled, onClick = onBack) {
                Icon(
                    if (close) Icons.Outlined.Close else Icons.Outlined.ArrowBack,
                    if (close) ui(R.string.close_server_settings)
                    else ui(R.string.back_from_1_s, title),
                )
            }
        }
        Text(
            title,
            Modifier.weight(1f).padding(horizontal = 16.dp),
            style = MaterialTheme.typography.headlineSmall,
            fontWeight = FontWeight.SemiBold,
        )
        actions()
    }
}

@Composable
internal fun SettingsCard(content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier.fillMaxWidth()
            .clip(RoundedCornerShape(24.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerLow),
        content = content,
    )
}

@Composable
internal fun SettingsLink(
    title: String,
    subtitle: String,
    icon: ImageVector,
    onClick: () -> Unit,
    leading: (@Composable () -> Unit)? = null,
    target: String = "",
) {
    ListItem(
        headlineContent = { Text(title, fontWeight = FontWeight.Medium) },
        supportingContent = { Text(subtitle) },
        leadingContent = {
            if (leading != null) leading()
            else
                Surface(
                    shape = RoundedCornerShape(14.dp),
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                ) {
                    Icon(icon, null, Modifier.padding(12.dp))
                }
        },
        trailingContent = { Icon(Icons.Outlined.ChevronRight, null) },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier =
            Modifier.then(settingTarget(target)).clickable(onClick = onClick).heightIn(min = 80.dp),
    )
}

@Composable
internal fun SettingsLabel(text: String) {
    Text(
        text,
        Modifier.padding(start = 12.dp, top = 8.dp),
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.primary,
    )
}

@Composable
internal fun SettingsSwitch(
    title: String,
    subtitle: String,
    value: Boolean,
    id: String,
    onChange: (Boolean) -> Unit,
) {
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = { Text(subtitle) },
        trailingContent = {
            Switch(value, onChange, Modifier.semantics { contentDescription = title })
        },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.then(settingTarget(id)).heightIn(min = 72.dp),
    )
}

@Composable
internal fun SettingsDivider() {
    HorizontalDivider(
        Modifier.padding(horizontal = 16.dp),
        color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = .5f),
    )
}

@Composable
private fun NotificationSettings(server: ServerState) {
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
                    ui(R.string.notify_for_messages_and_mentions_while_you_re_away_from_their_chat),
                    prefs.boolean("notifications-enabled"),
                    "notifications-enabled",
                ) {
                    prefs.set("notifications-enabled", it)
                    available = MessageAlerts.listener.available()
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
                                ui(
                                    R.string
                                        .notifications_are_disabled_in_this_server_account_s_preferences
                                )
                            server.pushRegistered ->
                                ui(R.string.push_connected_messages_can_arrive_while_papo_is_closed)
                            MessageAlerts.configured ->
                                ui(
                                    R.string
                                        .connecting_push_periodic_checks_cover_delivery_until_registration_succeeds
                                )
                            else ->
                                ui(
                                    R.string
                                        .this_build_uses_periodic_background_checks_android_may_delay_them_instant_p
                                )
                        },
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(
                        ui(
                            R.string
                                .each_channel_s_notification_preference_still_applies_opening_a_notification
                        ),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}

@Composable
private fun AppearanceSettings() {
    val prefs = LocalClientPreferences.current
    val target = LocalSettingTarget.current
    val list = rememberLazyListState()
    val order =
        listOf("preview", "preview-note", "theme-label", "theme", "glass", "conversation-label") +
            appearanceSwitches.map { it.key } +
            "text-size"
    LaunchedEffect(target) {
        val index = order.indexOf(target)
        if (index >= 0) list.scrollToItem(index)
    }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).testTag("appearance-settings"),
        state = list,
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 40.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        item("preview") {
            SettingsCard {
                Column(
                    Modifier.padding(20.dp),
                    verticalArrangement = Arrangement.spacedBy(if (prefs.compact) 6.dp else 14.dp),
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
                        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
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
                            Text(ui(R.string.a_little_more_like_you))
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
        item("text-size") {
            Text(
                ui(R.string.text_size_follows_android_s_font_and_display_settings),
                Modifier.padding(8.dp),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun AccountSettings(
    server: ServerState,
    model: PapoViewModel,
    onBack: () -> Unit,
    onLogout: () -> Unit,
    feedback: (String) -> Unit,
) {
    val scope = rememberCoroutineScope()
    var loaded by remember(server.url, server.me) { mutableStateOf<AccountProfile?>(null) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var nickname by rememberSaveable(server.url, server.me) { mutableStateOf<String?>(null) }
    var status by rememberSaveable(server.url, server.me) { mutableStateOf<String?>(null) }
    var about by rememberSaveable(server.url, server.me) { mutableStateOf<String?>(null) }
    var typing by rememberSaveable(server.url, server.me) { mutableStateOf<String?>(null) }
    var discard by remember { mutableStateOf(false) }
    var logout by remember { mutableStateOf(false) }
    var passwordDialog by remember { mutableStateOf(false) }
    var password by remember { mutableStateOf("") }
    var repeatPassword by remember { mutableStateOf("") }
    val target = LocalSettingTarget.current
    val list = rememberLazyListState()
    LaunchedEffect(target, loaded != null, loading, error) {
        val base = 2 + (if (loading) 1 else 0) + (if (error.isNotBlank()) 1 else 0)
        val index =
            when (target) {
                "avatar",
                "banner",
                "remove-banner" -> 1
                "nickname" -> base + 1
                "status" -> base + 2
                "about" -> base + 3
                "presence" -> base + 5
                "typing" -> base + 7
                "password",
                "logout" -> base + 10
                else -> -1
            }
        if (index >= 0 && (loaded != null || index == 1)) list.scrollToItem(index)
    }
    val dirty =
        loaded?.let {
            nickname != null && nickname != it.nickname ||
                status != null && status != it.statusMessage ||
                about != null && about != it.description ||
                typing != null && typing != it.typing
        } ?: false
    val back: () -> Unit = {
        if (!busy) {
            if (dirty) discard = true else onBack()
        }
    }
    BackHandler { back() }
    suspend fun load() {
        loading = true
        error = ""
        try {
            loaded = model.accountProfile(server)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = e.message ?: ui(R.string.couldn_t_load_your_profile)
        } finally {
            loading = false
        }
    }
    LaunchedEffect(server.url, server.me) { load() }
    fun change(change: AccountChange, message: String, clearDraft: Boolean = false) {
        if (busy) return
        busy = true
        error = ""
        scope.launch {
            try {
                model.accountChange(server, change)
                if (clearDraft && change is AccountChange.Profile) {
                    loaded =
                        loaded?.copy(
                            nickname = change.nickname,
                            statusMessage = change.statusMessage,
                            description = change.description,
                            typing = change.typing,
                        )
                    nickname = null
                    status = null
                    about = null
                    typing = null
                }
                if (change is AccountChange.Presence) loaded = loaded?.copy(presence = change.value)
                password = ""
                repeatPassword = ""
                passwordDialog = false
                feedback(message)
                // A failed read after an acknowledged save is reported as a
                // refresh failure, without prompting another mutation.
                try {
                    loaded = model.accountProfile(server)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    error =
                        ui(R.string.saved_but_couldn_t_refresh_the_profile_1_s, e.message.orEmpty())
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = e.message ?: ui(R.string.couldn_t_save_changes)
            } finally {
                busy = false
            }
        }
    }
    var imageTarget by remember { mutableStateOf("avatar") }
    val context = LocalContext.current
    var imageSource by remember { mutableStateOf<MediaEditorSource?>(null) }
    val imagePicker =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            if (uri != null && !busy)
                scope.launch {
                    busy = true
                    try {
                        imageSource = MediaEditorSource.read(context, uri)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        error = e.message ?: ui(R.string.couldn_t_open_this_image)
                    } finally {
                        busy = false
                    }
                }
        }
    imageSource?.let { source ->
        MediaEditor(
            source,
            if (imageTarget == "avatar") io.github.chriexpe.papo.core.MediaEditPurpose.AVATAR
            else io.github.chriexpe.papo.core.MediaEditPurpose.BANNER,
            server,
            model,
            onDismiss = { imageSource = null },
        ) { prepared, _ ->
            busy = true
            try {
                model.accountChange(
                    server,
                    if (imageTarget == "avatar")
                        AccountChange.Avatar(prepared.blob, prepared.format)
                    else AccountChange.Banner(prepared.blob, prepared.format),
                )
                feedback(
                    if (imageTarget == "avatar") ui(R.string.profile_photo_updated)
                    else ui(R.string.profile_banner_updated)
                )
                try {
                    loaded = model.accountProfile(server)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    error =
                        ui(
                            R.string.image_saved_but_couldn_t_refresh_your_profile_1_s,
                            e.message.orEmpty(),
                        )
                }
            } finally {
                busy = false
            }
        }
    }
    Column(Modifier.fillMaxSize()) {
        SettingsHeader(uiLabel("Account"), back) {
            if (busy)
                CircularProgressIndicator(Modifier.padding(12.dp).size(24.dp), strokeWidth = 2.dp)
            else
                FilledTonalButton(
                    enabled = dirty && loaded != null,
                    onClick = {
                        loaded?.let { p ->
                            change(
                                AccountChange.Profile(
                                    nickname ?: p.nickname,
                                    status ?: p.statusMessage,
                                    about ?: p.description,
                                    typing ?: p.typing,
                                ),
                                ui(R.string.profile_saved),
                                true,
                            )
                        }
                    },
                    modifier = Modifier.testTag("save-account"),
                ) {
                    Text(ui(R.string.save))
                }
        }
        LazyColumn(
            Modifier.fillMaxSize()
                .imePadding()
                .hazeSource(LocalHaze.current)
                .testTag("account-settings"),
            state = list,
            contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 40.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            item {
                SettingsCard {
                    val profile = server.profiles.firstOrNull { it.id == server.me }
                    Box(Modifier.fillMaxWidth()) {
                        ProfileBanner(
                            server,
                            model,
                            server.me,
                            profile?.banner.orEmpty(),
                            Modifier.fillMaxWidth().height(120.dp),
                        )
                        UserAvatar(
                            server.me,
                            server.myName,
                            Modifier.padding(start = 20.dp, top = 84.dp).size(80.dp),
                        )
                    }
                    Column(
                        Modifier.padding(20.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text(
                            nickname ?: loaded?.nickname ?: server.myName,
                            style = MaterialTheme.typography.titleLarge,
                            fontWeight = FontWeight.SemiBold,
                        )
                        Text(
                            "@${loaded?.username ?: server.members.firstOrNull {it.id==server.me}?.username.orEmpty()}",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        val message = status ?: loaded?.statusMessage.orEmpty()
                        if (message.isNotBlank())
                            Text(
                                message,
                                style = MaterialTheme.typography.bodyMedium,
                                maxLines = 3,
                                overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                            )
                    }
                }
            }
            item {
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(
                        modifier = Modifier.then(settingTarget("avatar")),
                        enabled = !busy && loaded != null,
                        onClick = {
                            imageTarget = "avatar"
                            imagePicker.launch(
                                arrayOf("image/png", "image/jpeg", "image/gif", "image/webp")
                            )
                        },
                    ) {
                        Text(ui(R.string.change_photo))
                    }
                    if (!server.isDemo)
                        OutlinedButton(
                            modifier = Modifier.then(settingTarget("banner")),
                            enabled = !busy && loaded != null,
                            onClick = {
                                imageTarget = "banner"
                                imagePicker.launch(
                                    arrayOf("image/png", "image/jpeg", "image/gif", "image/webp")
                                )
                            },
                        ) {
                            Text(ui(R.string.change_banner))
                        }
                    if (
                        server.profiles.firstOrNull { it.id == server.me }?.banner?.isNotBlank() ==
                            true
                    )
                        TextButton(
                            modifier = Modifier.then(settingTarget("remove-banner")),
                            enabled = !busy,
                            onClick = {
                                change(AccountChange.Banner("", ""), ui(R.string.banner_removed))
                            },
                        ) {
                            Text(ui(R.string.remove_banner))
                        }
                }
            }
            if (loading)
                item {
                    LinearProgressIndicator(
                        Modifier.fillMaxWidth().semantics {
                            contentDescription = ui(R.string.loading_account)
                        }
                    )
                }
            if (error.isNotBlank())
                item {
                    SettingsError(
                        error,
                        if (loaded == null) ({ scope.launch { load() } }) else null,
                    )
                }
            if (loaded != null) {
                item { SettingsLabel(ui(R.string.profile_on_1_s, server.name)) }
                item {
                    OutlinedTextField(
                        nickname ?: loaded!!.nickname,
                        { nickname = it },
                        Modifier.fillMaxWidth()
                            .then(settingTarget("nickname"))
                            .testTag("account-nickname"),
                        label = { Text(ui(R.string.display_name)) },
                        singleLine = true,
                        enabled = !busy,
                    )
                }
                item {
                    OutlinedTextField(
                        status ?: loaded!!.statusMessage,
                        { status = it },
                        Modifier.fillMaxWidth()
                            .then(settingTarget("status"))
                            .testTag("account-status"),
                        label = { Text(ui(R.string.status_message)) },
                        maxLines = 3,
                        enabled = !busy,
                    )
                }
                item {
                    OutlinedTextField(
                        about ?: loaded!!.description,
                        { about = it },
                        Modifier.fillMaxWidth()
                            .then(settingTarget("about"))
                            .testTag("account-about"),
                        label = { Text(ui(R.string.about_you)) },
                        minLines = 3,
                        maxLines = 6,
                        enabled = !busy,
                    )
                }
                item { SettingsLabel(ui(R.string.presence)) }
                item {
                    SettingsCard {
                        Column(Modifier.then(settingTarget("presence")).padding(16.dp)) {
                            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                listOf(
                                        "online" to uiLabel("Online"),
                                        "away" to uiLabel("Away"),
                                        "busy" to ui(R.string.do_not_disturb),
                                    )
                                    .forEach { (value, label) ->
                                        FilterChip(
                                            loaded!!.presence == value,
                                            {
                                                change(
                                                    AccountChange.Presence(value),
                                                    ui(R.string.presence_updated),
                                                )
                                            },
                                            label = { Text(label) },
                                            enabled = !busy,
                                        )
                                    }
                            }
                            Text(
                                ui(
                                    R.string
                                        .presence_is_shared_with_this_server_it_doesn_t_change_android_s_notificatio
                                ),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
                item { SettingsLabel(ui(R.string.when_you_type)) }
                item {
                    OutlinedTextField(
                        typing ?: loaded!!.typing,
                        { typing = it },
                        Modifier.fillMaxWidth()
                            .then(settingTarget("typing"))
                            .testTag("account-typing"),
                        label = { Text(ui(R.string.typing_phrase)) },
                        supportingText = {
                            Text(ui(R.string.leave_empty_to_use_the_default_phrase))
                        },
                        singleLine = true,
                        enabled = !busy,
                    )
                }
                item {
                    SettingsCard {
                        Text(
                            "•••  ${nickname ?: loaded!!.nickname} ${ (typing ?: loaded!!.typing).ifBlank {ui(R.string.is_typing)} }",
                            Modifier.padding(16.dp),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
            if (!server.isDemo) {
                item { SettingsLabel(ui(R.string.security)) }
                item {
                    SettingsCard {
                        SettingsLink(
                            ui(R.string.change_password),
                            ui(R.string.choose_a_new_password_for_this_server),
                            Icons.Outlined.Lock,
                            { if (!busy) passwordDialog = true },
                            target = "password",
                        )
                        SettingsDivider()
                        SettingsLink(
                            ui(R.string.sign_out),
                            ui(R.string.end_this_account_s_session_on_1_s, server.name),
                            Icons.Outlined.Logout,
                            { if (!busy) logout = true },
                            target = "logout",
                        )
                    }
                }
            }
        }
    }
    if (discard)
        AlertDialog(
            onDismissRequest = { discard = false },
            title = { Text(ui(R.string.discard_profile_changes)) },
            text = { Text(ui(R.string.your_unsaved_edits_will_be_lost)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        discard = false
                        onBack()
                    }
                ) {
                    Text(ui(R.string.discard))
                }
            },
            dismissButton = {
                TextButton(onClick = { discard = false }) { Text(ui(R.string.keep_editing)) }
            },
        )
    if (logout)
        AlertDialog(
            onDismissRequest = { logout = false },
            title = { Text(ui(R.string.sign_out_2)) },
            text = { Text(ui(R.string.you_ll_need_to_sign_in_again_on_1_s, server.name)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        logout = false
                        onLogout()
                    }
                ) {
                    Text(ui(R.string.sign_out))
                }
            },
            dismissButton = {
                TextButton(onClick = { logout = false }) { Text(ui(R.string.cancel)) }
            },
        )
    if (passwordDialog)
        AlertDialog(
            onDismissRequest = {
                if (!busy) {
                    passwordDialog = false
                    password = ""
                    repeatPassword = ""
                }
            },
            title = { Text(ui(R.string.change_password)) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        ui(
                            R.string
                                .use_at_least_eight_characters_this_changes_your_password_on_1_s,
                            server.name,
                        )
                    )
                    OutlinedTextField(
                        password,
                        { password = it },
                        label = { Text(ui(R.string.new_password)) },
                        singleLine = true,
                        visualTransformation = PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                        enabled = !busy,
                    )
                    OutlinedTextField(
                        repeatPassword,
                        { repeatPassword = it },
                        label = { Text(ui(R.string.repeat_password)) },
                        singleLine = true,
                        visualTransformation = PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                        enabled = !busy,
                    )
                    if (error.isNotBlank()) Text(error, color = MaterialTheme.colorScheme.error)
                }
            },
            confirmButton = {
                TextButton(
                    enabled =
                        !busy &&
                            password.codePointCount(0, password.length) >= 8 &&
                            password == repeatPassword,
                    onClick = {
                        change(AccountChange.Password(password), ui(R.string.password_changed))
                    },
                ) {
                    Text(if (busy) ui(R.string.changing) else ui(R.string.change_password))
                }
            },
            dismissButton = {
                TextButton(
                    enabled = !busy,
                    onClick = {
                        passwordDialog = false
                        password = ""
                        repeatPassword = ""
                    },
                ) {
                    Text(ui(R.string.cancel))
                }
            },
        )
}

@Composable
private fun FileSettings(feedback: (String) -> Unit) {
    val context = LocalContext.current
    val prefs = LocalClientPreferences.current
    val folder = prefs.string("download-tree")
    val scope = rememberCoroutineScope()
    val picker =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
            if (uri != null)
                scope.launch {
                    try {
                        val label =
                            withContext(Dispatchers.IO) {
                                context.contentResolver.takePersistableUriPermission(
                                    uri,
                                    Intent.FLAG_GRANT_READ_URI_PERMISSION or
                                        Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                                )
                                val document =
                                    DocumentsContract.buildDocumentUriUsingTree(
                                        uri,
                                        DocumentsContract.getTreeDocumentId(uri),
                                    )
                                context.contentResolver
                                    .query(
                                        document,
                                        arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME),
                                        null,
                                        null,
                                        null,
                                    )
                                    ?.use { cursor ->
                                        if (cursor.moveToFirst()) cursor.getString(0) else null
                                    } ?: ui(R.string.selected_folder)
                            }
                        val previous = folder.takeIf { it.isNotBlank() }?.let(Uri::parse)
                        prefs.set("download-tree", uri.toString())
                        prefs.set("download-label", label)
                        if (previous != null && previous != uri)
                            withContext(Dispatchers.IO) {
                                runCatching {
                                    context.contentResolver.releasePersistableUriPermission(
                                        previous,
                                        Intent.FLAG_GRANT_READ_URI_PERMISSION or
                                            Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                                    )
                                }
                            }
                        feedback(ui(R.string.download_folder_updated))
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        feedback(e.message ?: ui(R.string.couldn_t_use_that_folder))
                    }
                }
        }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item { SettingsLabel(ui(R.string.downloads)) }
        item {
            SettingsCard {
                SettingsLink(
                    ui(R.string.save_media_to),
                    downloadLocation(context),
                    Icons.Outlined.Folder,
                    { picker.launch(folder.takeIf { it.isNotBlank() }?.let(Uri::parse)) },
                    target = "download-folder",
                )
                if (folder.isNotBlank()) {
                    SettingsDivider()
                    SettingsLink(
                        ui(R.string.use_downloads_papo),
                        ui(R.string.return_to_the_default_android_folder),
                        Icons.Outlined.Restore,
                        {
                            val old = Uri.parse(folder)
                            prefs.set("download-tree", "")
                            prefs.set("download-label", "")
                            runCatching {
                                context.contentResolver.releasePersistableUriPermission(
                                    old,
                                    Intent.FLAG_GRANT_READ_URI_PERMISSION or
                                        Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                                )
                            }
                            feedback(ui(R.string.downloads_will_be_saved_to_downloads_papo))
                        },
                        target = "download-reset",
                    )
                }
            }
        }
        item {
            Text(
                ui(
                    R.string
                        .applies_to_the_download_button_in_the_shared_media_viewer_android_manages_a
                ),
                Modifier.padding(horizontal = 8.dp),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun SessionSettings(server: ServerState, model: PapoViewModel, feedback: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    var sessions by remember { mutableStateOf<List<SessionView>?>(null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var ending by remember { mutableStateOf<SessionView?>(null) }
    suspend fun load() {
        busy = true
        error = ""
        try {
            sessions = model.accountSessions(server)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = e.message ?: ui(R.string.couldn_t_load_sessions)
        } finally {
            busy = false
        }
    }
    LaunchedEffect(server.url, server.me) { load() }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            Text(
                ui(R.string.signed_in_to_1_s, server.name),
                Modifier.padding(8.dp),
                style = MaterialTheme.typography.titleMedium,
            )
        }
        item {
            Text(
                ui(
                    R.string
                        .the_server_provides_session_ids_and_dates_but_doesn_t_identify_devices_or_w
                ),
                Modifier.padding(8.dp),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (busy) item { LinearProgressIndicator(Modifier.fillMaxWidth()) }
        if (error.isNotBlank()) item { SettingsError(error) { scope.launch { load() } } }
        items(sessions.orEmpty(), key = { it.id }) { session ->
            SettingsCard {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        ui(R.string.session_short, session.id.takeLast(8)),
                        style = MaterialTheme.typography.titleMedium,
                    )
                    if (session.createdAt.isNotBlank())
                        Text(
                            ui(R.string.created_at, settingsDate(session.createdAt)),
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    if (session.expiresAt.isNotBlank())
                        Text(
                            ui(R.string.expires_at, settingsDate(session.expiresAt)),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    TextButton(enabled = !busy, onClick = { ending = session }) {
                        Text(ui(R.string.end_session))
                    }
                }
            }
        }
        if (sessions?.isEmpty() == true && !busy)
            item {
                Text(ui(R.string.the_server_returned_no_active_sessions), Modifier.padding(16.dp))
            }
        item {
            OutlinedButton(enabled = !busy, onClick = { scope.launch { load() } }) {
                Icon(Icons.Outlined.Refresh, null)
                Spacer(Modifier.width(8.dp))
                Text(ui(R.string.refresh_sessions))
            }
        }
    }
    ending?.let { session ->
        AlertDialog(
            onDismissRequest = { ending = null },
            title = { Text(ui(R.string.end_this_session)) },
            text = {
                Text(
                    ui(
                        R.string
                            .session_1_s_will_be_signed_out_if_it_belongs_to_this_app_you_ll_need_to_sig,
                        session.id.takeLast(8),
                    )
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        ending = null
                        busy = true
                        scope.launch {
                            try {
                                model.accountChange(server, AccountChange.RevokeSession(session.id))
                                sessions = sessions.orEmpty().filterNot { it.id == session.id }
                                feedback(ui(R.string.session_ended))
                                load()
                            } catch (e: CancellationException) {
                                throw e
                            } catch (e: Exception) {
                                error = e.message ?: ui(R.string.couldn_t_end_session)
                            } finally {
                                busy = false
                            }
                        }
                    }
                ) {
                    Text(ui(R.string.end_session))
                }
            },
            dismissButton = {
                TextButton(onClick = { ending = null }) { Text(ui(R.string.cancel)) }
            },
        )
    }
}

private fun settingsDate(raw: String) = runCatching {
    OffsetDateTime.parse(raw)
        .atZoneSameInstant(java.time.ZoneId.systemDefault())
        .format(
            DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT)
                .withLocale(AppLanguage.locale)
        )
}
    .getOrDefault(raw)

@Composable
private fun DiagnosticSettings(
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
                                "${server.connection.replaceFirstChar(Char::uppercase)}\n${server.url}"
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
                ui(
                    R.string
                        .saved_downloads_favorite_gif_copies_messages_drafts_and_sign_ins_are_kept
                ),
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
                Text(
                    ui(
                        R.string
                            .media_will_need_to_download_again_saved_files_and_account_data_are_kept
                    )
                )
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

@Composable
internal fun SettingsError(message: String, retry: (() -> Unit)? = null) {
    Surface(
        color = MaterialTheme.colorScheme.errorContainer,
        shape = RoundedCornerShape(18.dp),
        modifier = Modifier.fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                message,
                color = MaterialTheme.colorScheme.onErrorContainer,
                style = MaterialTheme.typography.bodyMedium,
            )
            if (retry != null) TextButton(onClick = retry) { Text(ui(R.string.try_again)) }
        }
    }
}
