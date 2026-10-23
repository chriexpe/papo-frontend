package io.github.chriexpe.papo.compose

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

@Composable
internal fun WebhookIdentityEditor(
    server: ServerState,
    model: PapoViewModel,
    hooks: WebhookUi,
    id: String,
    dirty: (Boolean) -> Unit,
    saved: () -> Unit,
) {
    val hook = hooks.hooks.firstOrNull { it.id == id }
    if (hook == null) {
        if (hooks.loading) LinearProgressIndicator(Modifier.fillMaxWidth())
        else SettingsError(hooks.error.ifBlank { ui(R.string.webhook_missing) }, hooks::load)
        return
    }
    key(server.url, server.me, id) {
        var nickname by rememberSaveable { mutableStateOf(hook.displayName) }
        var avatar by remember { mutableStateOf<PreparedMediaImage?>(null) }
        var removed by rememberSaveable { mutableStateOf(false) }
        var source by remember { mutableStateOf<MediaEditorSource?>(null) }
        var opening by remember { mutableStateOf(false) }
        var imageError by remember { mutableStateOf("") }
        val scope = rememberCoroutineScope()
        val context = LocalContext.current
        val picker =
            rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
                if (uri != null)
                    scope.launch {
                        opening = true
                        imageError = ""
                        try {
                            source = MediaEditorSource.read(context, uri)
                        } catch (e: CancellationException) {
                            throw e
                        } catch (_: Exception) {
                            imageError = ui(R.string.couldn_t_open_this_image)
                        } finally {
                            opening = false
                        }
                    }
            }
        val changed = nickname.trim() != hook.displayName || avatar != null || removed
        LaunchedEffect(changed) { dirty(changed) }
        Column(
            Modifier.fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(20.dp)
                .testTag("edit-webhook"),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(hook.name, style = MaterialTheme.typography.titleLarge)
            if (hooks.error.isNotBlank()) SettingsError(hooks.error)
            if (imageError.isNotBlank()) SettingsError(imageError)
            if (opening) LinearProgressIndicator(Modifier.fillMaxWidth())
            OutlinedTextField(
                nickname,
                { nickname = it },
                Modifier.fillMaxWidth().testTag("webhook-nickname"),
                enabled = !hooks.busy,
                label = { Text(ui(R.string.display_name)) },
                placeholder = { Text(hook.name) },
                singleLine = true,
                isError = nickname.codePointsCount() > 100,
                supportingText = { Text(ui(R.string.webhook_nickname_hint)) },
            )
            if (avatar != null) {
                val bytes =
                    remember(avatar) {
                        android.util.Base64.decode(avatar!!.blob, android.util.Base64.DEFAULT)
                    }
                ManagedAsyncImage(
                    bytes,
                    ui(R.string.webhook_avatar_preview),
                    MediaImages.loader(context),
                    Modifier.size(112.dp),
                    retentionKey = "webhook-edit/$id/${bytes.contentHashCode()}",
                )
            } else if (!removed) {
                ExternalAvatar(
                    server,
                    ExternalAuthorView(
                        "webhook",
                        id,
                        hook.displayName.ifBlank { hook.name },
                        "/webhooks/$id/avatar",
                        "",
                    ),
                    Modifier.size(80.dp),
                )
            }
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(
                    enabled = !hooks.busy && !opening,
                    onClick = {
                        picker.launch(arrayOf("image/png", "image/jpeg", "image/gif", "image/webp"))
                    },
                ) {
                    Text(ui(R.string.change_photo))
                }
                TextButton(
                    enabled = !hooks.busy,
                    onClick = {
                        avatar = null
                        removed = true
                    },
                ) {
                    Text(ui(R.string.remove_photo))
                }
                if (avatar != null || removed)
                    TextButton(
                        enabled = !hooks.busy,
                        onClick = {
                            avatar = null
                            removed = false
                        },
                    ) {
                        Text(ui(R.string.webhook_keep_avatar))
                    }
            }
            Text(
                ui(R.string.webhook_avatar_hint),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Button(
                enabled = changed && !hooks.busy && !opening && nickname.codePointsCount() <= 100,
                modifier = Modifier.testTag("save-webhook-identity"),
                onClick = {
                    scope.launch {
                        val blob = if (removed) "" else avatar?.blob
                        if (
                            hooks.update(
                                id,
                                nickname.trim(),
                                blob,
                                if (removed) null else avatar?.format,
                            )
                        )
                            saved()
                    }
                },
            ) {
                Text(ui(R.string.save_changes))
            }
        }
        source?.let { image ->
            MediaEditor(
                image,
                MediaEditPurpose.AVATAR,
                server,
                model,
                onDismiss = { source = null },
            ) { prepared, _ ->
                avatar = prepared
                removed = false
                source = null
            }
        }
    }
}
