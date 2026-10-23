package io.github.chriexpe.papo.compose

import androidx.compose.material3.*
import androidx.compose.runtime.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** Chat, pins and search share the same acknowledged mutation and failure behavior. */
@Composable
internal fun DeleteMessageDialog(
    message: ChatMessage,
    server: ServerState,
    model: PapoViewModel,
    onDismiss: () -> Unit,
    onDeleted: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = { Text(ui(R.string.delete_this_message)) },
        text = {
            Text(
                if (error) ui(R.string.delete_message_failed)
                else ui(R.string.this_removes_the_message_for_everyone_you_can_t_undo_it),
                color =
                    if (error) MaterialTheme.colorScheme.error
                    else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        confirmButton = {
            TextButton(
                enabled = !busy,
                onClick = {
                    busy = true
                    error = false
                    scope.launch {
                        try {
                            model.deleteMessage(server, message.id)
                            onDeleted()
                        } catch (e: CancellationException) {
                            throw e
                        } catch (_: Exception) {
                            error = true
                        } finally {
                            busy = false
                        }
                    }
                },
            ) {
                Text(ui(R.string.delete), color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = {
            TextButton(enabled = !busy, onClick = onDismiss) { Text(ui(R.string.cancel)) }
        },
    )
}
