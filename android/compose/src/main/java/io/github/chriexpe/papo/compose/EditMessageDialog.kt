package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.encodeEmbedDrafts
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** Editor bindings are a snapshot, not labels reconstructed on each member-list refresh. */
@Composable
internal fun EditMessageDialog(
    message: ChatMessage,
    server: ServerState,
    model: PapoViewModel,
    onDismiss: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    var draft by
        rememberSaveable(server.url, server.me, message.id, stateSaver = ComposerDraft.Saver) {
            mutableStateOf(ComposerDraft())
        }
    var loaded by rememberSaveable(server.url, server.me, message.id) { mutableStateOf(false) }
    var originalEmbeds by rememberSaveable { mutableStateOf("[]") }
    var embedJson by rememberSaveable { mutableStateOf("[]") }
    var embedsEditable by rememberSaveable { mutableStateOf(false) }
    var hasCustom by rememberSaveable { mutableStateOf(false) }
    var embedEditor by rememberSaveable { mutableStateOf(false) }
    val embeds = remember(embedJson) { restoredEmbeds(embedJson) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    LaunchedEffect(server.url, server.me, message.id) {
        if (!loaded)
            try {
                draft = model.editableMessage(server, message.content)
                val result = model.editableEmbeds(server, message.id)
                originalEmbeds = encodeEmbedDrafts(result.drafts)
                embedJson = originalEmbeds
                embedsEditable = result.editable
                hasCustom = result.hasCustom
                loaded = true
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                error = ui(R.string.edit_message_load_failed)
            }
    }
    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        modifier = Modifier.testTag("edit-message-dialog"),
        title = { Text(ui(R.string.edit_message)) },
        text = {
            Column(Modifier.heightIn(max = 440.dp).verticalScroll(rememberScrollState())) {
                if (!loaded && error.isEmpty()) CircularProgressIndicator()
                if (loaded) {
                    if (!busy) ComposerSuggestions(draft, server, model) { draft = it }
                    ComposerInput(
                        draft,
                        server,
                        model,
                        ui(R.string.message),
                        { draft = it },
                        Modifier.fillMaxWidth(),
                        inputTag = "edit-message-input",
                        enabled = !busy,
                    )
                }
                if (loaded) {
                    TextButton(
                        enabled = !busy && embedsEditable,
                        onClick = { embedEditor = true },
                    ) {
                        Text(ui(R.string.embed_manage))
                    }
                    if (!embedsEditable)
                        Text(
                            ui(R.string.embed_protected_edit),
                            style = MaterialTheme.typography.bodySmall,
                        )
                    else if (embeds.isNotEmpty())
                        Text(
                            uiQuantity(R.plurals.embed_draft_count, embeds.size),
                            style = MaterialTheme.typography.bodySmall,
                        )
                }
                if (error.isNotEmpty()) Text(error, color = MaterialTheme.colorScheme.error)
            }
        },
        confirmButton = {
            TextButton(
                enabled =
                    loaded &&
                        !busy &&
                        (draft.text.isNotBlank() ||
                            message.attachments.isNotEmpty() ||
                            (if (embedsEditable) embeds.isNotEmpty() else hasCustom)),
                onClick = {
                    busy = true
                    error = ""
                    scope.launch {
                        try {
                            // Omission preserves existing IDs/media; an explicit empty list removes
                            // custom embeds.
                            model.editMessage(
                                server,
                                message.id,
                                draft,
                                embeds.takeIf { embedJson != originalEmbeds },
                            )
                            onDismiss()
                        } catch (e: CancellationException) {
                            throw e
                        } catch (_: Exception) {
                            error = ui(R.string.edit_message_save_failed)
                        } finally {
                            busy = false
                        }
                    }
                },
            ) {
                Text(ui(R.string.save))
            }
        },
        dismissButton = {
            TextButton(enabled = !busy, onClick = onDismiss) { Text(ui(R.string.cancel)) }
        },
    )
    if (embedEditor)
        EmbedDraftEditor(
            embeds,
            emptyList(),
            server,
            model,
            onApply = {
                embedJson = encodeEmbedDrafts(it)
                embedEditor = false
            },
            onDismiss = { embedEditor = false },
        )
}
