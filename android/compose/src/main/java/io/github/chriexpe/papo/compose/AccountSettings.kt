@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.AccountChange
import io.github.chriexpe.papo.core.AccountProfile
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

@Composable
internal fun AccountSettings(
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
    // Null means untouched: refreshed server values must not overwrite an in-progress draft.
    var nickname by rememberSaveable(server.url, server.me) { mutableStateOf<String?>(null) }
    var status by rememberSaveable(server.url, server.me) { mutableStateOf<String?>(null) }
    var about by rememberSaveable(server.url, server.me) { mutableStateOf<String?>(null) }
    var typing by rememberSaveable(server.url, server.me) { mutableStateOf<String?>(null) }
    var discard by remember { mutableStateOf(false) }
    var logout by remember { mutableStateOf(false) }
    var passwordDialog by remember { mutableStateOf(false) }
    // Credentials live only in this composition, never in saved instance state or preferences.
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
                AccountProfilePreview(
                    server = server,
                    model = model,
                    displayName = nickname ?: loaded?.nickname ?: server.myName,
                    username =
                        loaded?.username
                            ?: server.members
                                .firstOrNull { it.id == server.me }
                                ?.username
                                .orEmpty(),
                    statusMessage = status ?: loaded?.statusMessage.orEmpty(),
                )
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
                    AccountPresence(loaded!!.presence, busy) { value ->
                        change(AccountChange.Presence(value), ui(R.string.presence_updated))
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
private fun AccountProfilePreview(
    server: ServerState,
    model: PapoViewModel,
    displayName: String,
    username: String,
    statusMessage: String,
) {
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
                displayName,
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                "@$username",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            val message = statusMessage
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

@Composable
private fun AccountPresence(presence: String, busy: Boolean, onChange: (String) -> Unit) {
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
                            presence == value,
                            { onChange(value) },
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
