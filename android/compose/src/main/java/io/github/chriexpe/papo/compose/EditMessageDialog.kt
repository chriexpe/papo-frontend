package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
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
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    LaunchedEffect(server.url, server.me, message.id) {
        if (!loaded)
            try {
                draft = model.editableMessage(server, message.content)
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
            Column {
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
                if (error.isNotEmpty()) Text(error, color = MaterialTheme.colorScheme.error)
            }
        },
        confirmButton = {
            TextButton(
                enabled = loaded && draft.text.isNotBlank() && !busy,
                onClick = {
                    busy = true
                    error = ""
                    scope.launch {
                        try {
                            model.editMessage(server, message.id, draft)
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
}
