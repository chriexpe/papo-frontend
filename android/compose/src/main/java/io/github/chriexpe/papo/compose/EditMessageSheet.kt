@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** Editing has its own bound draft; changing it never replaces the unsent chat composer. */
@Composable
internal fun EditMessageSheet(
    message: ChatMessage,
    server: ServerState,
    model: PapoViewModel,
    onDismiss: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
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
    LaunchedEffect(loaded) {
        if (loaded) {
            withFrameNanos {}
            focus.requestFocus()
            keyboard?.show()
        }
    }
    val canSave =
        loaded &&
            !busy &&
            (draft.text.isNotBlank() ||
                message.attachments.isNotEmpty() ||
                message.embeds.any { it.sourceType == "custom" })
    fun save() {
        if (!canSave) return
        busy = true
        error = ""
        scope.launch {
            try {
                // Omit embeds: text edits preserve their existing IDs and media.
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
    }
    ModalBottomSheet(
        onDismissRequest = { if (!busy) onDismiss() },
        sheetState =
            rememberModalBottomSheetState(
                skipPartiallyExpanded = true,
                confirmValueChange = { !busy },
            ),
    ) {
        Column(
            Modifier.fillMaxWidth()
                .imePadding()
                .heightIn(max = 560.dp)
                .padding(horizontal = 20.dp)
                .padding(bottom = 20.dp)
                .testTag("edit-message-sheet"),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(ui(R.string.edit_message), style = MaterialTheme.typography.titleLarge)
            Column(
                Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (!loaded && error.isEmpty()) LinearProgressIndicator(Modifier.fillMaxWidth())
                if (loaded) {
                    Text(
                        ui(R.string.message),
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Surface(
                        shape = MaterialTheme.shapes.medium,
                        color = MaterialTheme.colorScheme.surfaceContainerLow,
                        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                        modifier = Modifier.fillMaxWidth().testTag("edit-message-field"),
                    ) {
                        // Reuse rich input for bound mentions/stickers. Enter keeps a newline
                        // here even when Enter-to-send is enabled for the chat composer.
                        ComposerInput(
                            draft,
                            server,
                            model,
                            ui(R.string.message),
                            { draft = it },
                            Modifier.fillMaxWidth().heightIn(min = 104.dp).focusRequester(focus),
                            inputTag = "edit-message-input",
                            enabled = !busy,
                        )
                    }
                    if (!busy) ComposerSuggestions(draft, server, model) { draft = it }
                }
                if (error.isNotEmpty()) Text(error, color = MaterialTheme.colorScheme.error)
            }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(enabled = !busy, onClick = onDismiss) { Text(ui(R.string.cancel)) }
                Spacer(Modifier.width(8.dp))
                Button(
                    enabled = canSave,
                    onClick = ::save,
                    modifier = Modifier.testTag("edit-message-save"),
                ) {
                    if (busy) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                    else Text(ui(R.string.save))
                }
            }
        }
    }
}
