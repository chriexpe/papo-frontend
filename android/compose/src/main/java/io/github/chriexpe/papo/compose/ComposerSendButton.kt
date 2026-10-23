@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp

/** An empty composer's Send remains an accessible entry to message options. */
@Composable
internal fun ComposerSendButton(
    canSend: Boolean,
    enabled: Boolean,
    busy: Boolean,
    onSend: () -> Unit,
    onEmbeds: () -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    Box {
        Surface(
            modifier =
                Modifier.size(48.dp)
                    .combinedClickable(
                        enabled = enabled,
                        role = Role.Button,
                        onClickLabel =
                            if (canSend) ui(R.string.send_message)
                            else ui(R.string.embed_message_options),
                        onClick = { if (canSend) onSend() else menu = true },
                        onLongClickLabel = ui(R.string.embed_message_options),
                        onLongClick = { menu = true },
                    ),
            shape = CircleShape,
            color =
                if (enabled && canSend) MaterialTheme.colorScheme.primary
                else MaterialTheme.colorScheme.surfaceContainerHigh,
        ) {
            Box(contentAlignment = androidx.compose.ui.Alignment.Center) {
                if (busy) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                else
                    Icon(
                        if (canSend) Icons.Outlined.ArrowUpward else Icons.Outlined.MoreHoriz,
                        if (canSend) ui(R.string.send_message)
                        else ui(R.string.embed_message_options),
                    )
            }
        }
        DropdownMenu(menu && enabled, onDismissRequest = { menu = false }) {
            DropdownMenuItem(
                text = { Text(ui(R.string.embed_manage)) },
                leadingIcon = { Icon(Icons.Outlined.ViewAgenda, null) },
                onClick = {
                    menu = false
                    onEmbeds()
                },
            )
        }
    }
}
