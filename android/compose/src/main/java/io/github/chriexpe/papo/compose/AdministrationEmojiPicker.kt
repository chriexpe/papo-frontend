package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.SentimentSatisfiedAlt
import androidx.compose.material3.Icon
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp

/** Insert at the editor's saved cursor, replacing any selected text. */
internal fun insertAdministrationEmoji(value: TextFieldValue, choice: EmojiChoice): TextFieldValue {
    val start = value.selection.min.coerceIn(0, value.text.length)
    val end = value.selection.max.coerceIn(start, value.text.length)
    val token = choice.token
    return TextFieldValue(value.text.replaceRange(start, end, token), TextRange(start + token.length))
}

/**
 * Shared channel/server editor affordance. The editor root already applies IME insets,
 * so the floating picker sits directly above the keyboard, with no composer offset.
 * Neither opening the picker nor selecting a sticker clears the active field's focus.
 */
@Composable
internal fun AdministrationEmojiPicker(
    server: ServerState,
    model: PapoViewModel,
    enabled: Boolean,
    onPick: (EmojiChoice) -> Unit,
    modifier: Modifier = Modifier,
) {
    var open by remember(server.url, server.me) { mutableStateOf(false) }
    var query by remember(server.url, server.me) { mutableStateOf("") }
    var searching by remember(server.url, server.me) { mutableStateOf(false) }
    var detail by remember(server.url, server.me) { mutableStateOf<String?>(null) }
    fun close() {
        open = false
        query = ""
        searching = false
    }

    Box(modifier) {
        if (open && enabled) {
            FloatingEmojiPicker(
                server = server,
                model = model,
                query = query,
                onQueryChange = { query = it },
                searchFocusRequested = searching,
                onSearchFocus = { searching = true },
                onClose = ::close,
                onPick = onPick,
                onDetail = { detail = it },
                modifier =
                    Modifier.align(Alignment.BottomCenter)
                        .fillMaxSize()
                        .padding(bottom = 8.dp)
                        .testTag("admin-floating-emoji-picker"),
            )
        } else if (enabled) {
            SmallFloatingActionButton(
                onClick = { open = true },
                modifier =
                    Modifier.align(Alignment.BottomEnd)
                        .padding(end = 20.dp, bottom = 20.dp)
                        .testTag("admin-emoji-button"),
            ) {
                Icon(Icons.Outlined.SentimentSatisfiedAlt, ui(R.string.emoji_and_stickers))
            }
        }
    }
    detail?.let { id -> StickerDetails(server, model, id, onDismiss = { detail = null }) }
}
