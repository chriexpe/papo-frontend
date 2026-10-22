@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import kotlinx.coroutines.launch

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
                            ClientSettingsHome(
                                server,
                                prefs,
                                query,
                                { query = it },
                                ::open,
                                onLogout,
                            )
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
private fun ClientSettingsHome(
    server: ServerState,
    prefs: ClientPreferences,
    query: String,
    onQueryChange: (String) -> Unit,
    open: (String, String) -> Unit,
    onLogout: () -> Unit,
) {
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item {
            OutlinedTextField(
                query,
                onQueryChange,
                Modifier.fillMaxWidth().testTag("settings-search"),
                placeholder = { Text(ui(R.string.search_settings)) },
                leadingIcon = { Icon(Icons.Outlined.Search, null) },
                trailingIcon = {
                    if (query.isNotBlank())
                        IconButton(onClick = { onQueryChange("") }) {
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
                        { open("Account", "") },
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
                        { open(entry.title, "") },
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
                        else settingsDestinations.first { it.title == entry.page }.icon
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
}
