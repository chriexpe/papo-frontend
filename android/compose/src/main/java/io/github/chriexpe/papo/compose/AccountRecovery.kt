package io.github.chriexpe.papo.compose

import android.content.ClipData
import android.content.ClipboardManager
import android.os.PersistableBundle
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import io.github.chriexpe.papo.core.RecoveryReceipt
import io.github.chriexpe.papo.core.recoverAccount
import io.github.chriexpe.papo.core.recoveryServer
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal fun isRecoveryIntent(raw: String): Boolean = runCatching {
    val uri = android.net.Uri.parse(raw)
    uri.scheme == "papo-reset" && uri.host == "passwordchange" ||
        uri.scheme == "https" &&
            uri.host == "papo.cyberasilo.online" &&
            uri.path.orEmpty().startsWith("/passwordchange/")
}
    .getOrDefault(false)

/**
 * Tokens and passwords deliberately use remember, never saved instance state, preferences,
 * navigation arguments or the diagnostic action stream.
 */
@Composable
internal fun AccountRecovery(initialLink: String, initialServer: String, onDismiss: () -> Unit) {
    var link by remember { mutableStateOf(initialLink) }
    var server by remember { mutableStateOf(initialServer) }
    var password by remember { mutableStateOf("") }
    var confirmation by remember { mutableStateOf("") }
    var error by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var success by remember { mutableStateOf(false) }
    val hint = remember(link) { runCatching { recoveryServer(link) }.getOrNull() }
    val scope = rememberCoroutineScope()
    Dialog(
        onDismissRequest = { if (!busy) onDismiss() },
        properties =
            DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false),
    ) {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            Column(
                Modifier.fillMaxSize()
                    .navigationBarsPadding()
                    .imePadding()
                    .testTag("account-recovery")
            ) {
                SettingsHeader(ui(R.string.account_recovery), onDismiss, enabled = !busy)
                Column(
                    Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(24.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    if (success) {
                        Icon(Icons.Outlined.CheckCircle, null)
                        Text(
                            ui(R.string.password_updated),
                            style = MaterialTheme.typography.headlineSmall,
                        )
                        Text(ui(R.string.recovery_completed_description))
                        Button(onClick = onDismiss) { Text(ui(R.string.back_to_sign_in)) }
                    } else {
                        Text(ui(R.string.use_the_single_use_link_provided_by_your_server_owner))
                        OutlinedTextField(
                            link,
                            {
                                link = it
                                error = ""
                            },
                            Modifier.fillMaxWidth().testTag("recovery-link"),
                            label = { Text(ui(R.string.recovery_link)) },
                            enabled = !busy,
                            singleLine = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                            isError = link.isNotBlank() && hint == null,
                        )
                        if (!hint.isNullOrBlank())
                            Text(
                                ui(R.string.this_link_belongs_to_1_s, hint),
                                style = MaterialTheme.typography.bodySmall,
                            )
                        OutlinedTextField(
                            server,
                            {
                                server = it
                                error = ""
                            },
                            Modifier.fillMaxWidth().testTag("recovery-server"),
                            label = { Text(ui(R.string.server_address)) },
                            enabled = !busy,
                            singleLine = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                        )
                        Text(
                            ui(R.string.recovery_server_confirmation),
                            style = MaterialTheme.typography.bodySmall,
                        )
                        OutlinedTextField(
                            password,
                            { password = it },
                            Modifier.fillMaxWidth().testTag("recovery-password"),
                            label = { Text(ui(R.string.new_password)) },
                            singleLine = true,
                            enabled = !busy,
                            visualTransformation = PasswordVisualTransformation(),
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                        )
                        OutlinedTextField(
                            confirmation,
                            { confirmation = it },
                            Modifier.fillMaxWidth().testTag("recovery-confirmation"),
                            label = { Text(ui(R.string.confirm_password)) },
                            singleLine = true,
                            enabled = !busy,
                            visualTransformation = PasswordVisualTransformation(),
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                            isError = confirmation.isNotBlank() && password != confirmation,
                        )
                        if (error.isNotBlank())
                            Text(
                                error,
                                color = MaterialTheme.colorScheme.error,
                                modifier = Modifier.testTag("recovery-error"),
                            )
                        Button(
                            enabled =
                                !busy &&
                                    hint != null &&
                                    server.isNotBlank() &&
                                    password.isNotEmpty() &&
                                    password == confirmation,
                            onClick = {
                                busy = true
                                error = ""
                                scope.launch {
                                    try {
                                        withContext(Dispatchers.IO) {
                                            recoverAccount(server.trim(), link.trim(), password)
                                        }
                                        password = ""
                                        confirmation = ""
                                        link = ""
                                        success = true
                                    } catch (e: CancellationException) {
                                        throw e
                                    } catch (e: Exception) {
                                        error = recoveryFailure(e)
                                    } finally {
                                        busy = false
                                    }
                                }
                            },
                            modifier = Modifier.fillMaxWidth().testTag("recover-account"),
                        ) {
                            if (busy)
                                CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                            else Text(ui(R.string.reset_password))
                        }
                    }
                }
            }
        }
    }
}

@Composable
internal fun GenerateRecoveryLink(
    server: ServerState,
    model: PapoViewModel,
    id: String,
    name: String,
    onDismiss: () -> Unit,
) {
    var receipt by remember { mutableStateOf<RecoveryReceipt?>(null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var copied by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        modifier = Modifier.testTag("generate-recovery-link"),
        title = {
            Text(
                if (receipt == null) ui(R.string.generate_recovery_link)
                else ui(R.string.recovery_link_ready)
            )
        },
        text = {
            Column(
                Modifier.heightIn(max = 360.dp).verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(ui(R.string.account_name, name))
                if (receipt == null) Text(ui(R.string.recovery_generate_description))
                else {
                    Text(ui(R.string.share_this_link_privately_with_the_account_owner))
                    Text(receipt!!.url, style = MaterialTheme.typography.bodySmall)
                    Text(
                        ui(R.string.expires_at, recoveryExpiry(receipt!!.expiresAt)),
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
                if (error.isNotBlank()) Text(error, color = MaterialTheme.colorScheme.error)
                if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            }
        },
        confirmButton = {
            if (receipt == null)
                TextButton(
                    enabled = !busy,
                    onClick = {
                        busy = true
                        error = ""
                        scope.launch {
                            try {
                                receipt = model.passwordResetLink(server, id)
                            } catch (e: CancellationException) {
                                throw e
                            } catch (e: Exception) {
                                error = recoveryFailure(e)
                            } finally {
                                busy = false
                            }
                        }
                    },
                ) {
                    Text(ui(R.string.generate_link))
                }
            else
                TextButton(
                    onClick = {
                        val clip =
                            ClipData.newPlainText(ui(R.string.papo_recovery_link), receipt!!.url)
                        clip.description.extras =
                            PersistableBundle().apply {
                                putBoolean("android.content.extra.IS_SENSITIVE", true)
                            }
                        context.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
                        copied = true
                    }
                ) {
                    Text(if (copied) ui(R.string.copied) else ui(R.string.copy_link))
                }
        },
        dismissButton = {
            TextButton(enabled = !busy, onClick = onDismiss) {
                Text(if (receipt == null) ui(R.string.cancel) else ui(R.string.done))
            }
        },
    )
}

internal fun recoveryExpiry(raw: String): String = runCatching {
    java.time.OffsetDateTime.parse(raw)
        .atZoneSameInstant(java.time.ZoneId.systemDefault())
        .format(
            java.time.format.DateTimeFormatter.ofLocalizedDateTime(
                    java.time.format.FormatStyle.MEDIUM
                )
                .withLocale(AppLanguage.locale)
        )
}
    .getOrDefault(raw)

/**
 * Server problem details remain server content; local validation and transport errors are semantic
 * UniFFI variants and receive Android presentation text.
 */
internal fun recoveryFailure(error: Exception): String =
    when (error) {
        is io.github.chriexpe.papo.core.RecoveryException.InvalidLink ->
            ui(R.string.invalid_recovery_link)
        is io.github.chriexpe.papo.core.RecoveryException.InvalidServer ->
            ui(R.string.invalid_server_address)
        is io.github.chriexpe.papo.core.RecoveryException.WrongServer ->
            ui(R.string.select_the_server_this_recovery_link_belongs_to)
        is io.github.chriexpe.papo.core.RecoveryException.EmptyPassword ->
            ui(R.string.enter_a_new_password)
        is io.github.chriexpe.papo.core.RecoveryException.NotOwner ->
            ui(R.string.only_the_server_owner_can_generate_recovery_links)
        is io.github.chriexpe.papo.core.RecoveryException.InvalidMember ->
            ui(R.string.choose_another_server_member)
        is io.github.chriexpe.papo.core.RecoveryException.AccountChanged ->
            ui(R.string.account_changed)
        is io.github.chriexpe.papo.core.RecoveryException.Network -> ui(R.string.recovery_network)
        is io.github.chriexpe.papo.core.RecoveryException.RateLimited ->
            ui(R.string.recovery_rate_limited)
        is io.github.chriexpe.papo.core.RecoveryException.Server ->
            ui(R.string.recovery_backend_error, error.details)
        else -> ui(R.string.recovery_unavailable)
    }
