package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import io.github.chriexpe.papo.core.EmbedDraft
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** Replaces the complete custom-card list, preserving the caption's original user bindings. */
@Composable
internal fun EditMessageEmbeds(
    message: ChatMessage,
    initialIndex: Int,
    server: ServerState,
    model: PapoViewModel,
    onDismiss: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val keyboard = LocalSoftwareKeyboardController.current
    var caption by remember { mutableStateOf<ComposerDraft?>(null) }
    var original by remember { mutableStateOf<List<EmbedDraft>?>(null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    LaunchedEffect(server.url, server.me, message.id) {
        try {
            val result = model.editableEmbeds(server, message.id)
            // Never offer a partial replacement: it would silently delete protected media.
            if (!result.editable || !result.hasCustom) {
                error = ui(R.string.embed_protected_edit)
            } else {
                caption = model.editableMessage(server, message.content)
                original = result.drafts
            }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            error = ui(R.string.edit_message_load_failed)
        }
    }
    val cards = original
    val boundCaption = caption
    if (cards == null || boundCaption == null) {
        AlertDialog(
            onDismissRequest = onDismiss,
            title = { Text(ui(R.string.embed_editor_title)) },
            text = {
                if (error.isNotEmpty()) Text(error)
                else LinearProgressIndicator(Modifier.fillMaxWidth())
            },
            confirmButton = { TextButton(onClick = onDismiss) { Text(ui(R.string.cancel)) } },
        )
    } else {
        EmbedDraftEditor(
            cards,
            emptyList(),
            server,
            model,
            initialIndex = initialIndex,
            saving = busy,
            saveError = error,
            editingMessage = true,
            onDismiss = { if (!busy) onDismiss() },
            onApply = { next ->
                if (next == cards) onDismiss()
                else if (!busy) {
                    busy = true
                    error = ""
                    keyboard?.hide()
                    scope.launch {
                        try {
                            model.editMessage(server, message.id, boundCaption, next)
                            onDismiss()
                        } catch (e: CancellationException) {
                            throw e
                        } catch (_: Exception) {
                            error = ui(R.string.edit_message_save_failed)
                        } finally {
                            busy = false
                        }
                    }
                }
            },
        )
    }
}
