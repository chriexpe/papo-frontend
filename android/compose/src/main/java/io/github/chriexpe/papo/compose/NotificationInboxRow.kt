@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.InboxItem
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

@Composable
internal fun NotificationInboxRow(
    notice: ScopedNotice,
    model: PapoViewModel,
    reading: Boolean,
    onRead: () -> Unit,
    onOpen: (ServerState, InboxItem) -> Unit,
) {
    val server = notice.server
    val item = notice.item
    val canOpen =
        (server.channels + server.dms).any { it.id == item.channelId && it.kind != "category" }
    CompositionLocalProvider(LocalProfileContext provides ProfileContext(server, model)) {
        ListItem(
            overlineContent = {
                Text(server.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
            },
            headlineContent = {
                Text(
                    item.author.ifBlank { ui(R.string.message) } +
                        (if (item.channelName.isNotBlank()) " · ${item.channelName}" else ""),
                    fontWeight = if (item.read) FontWeight.Normal else FontWeight.SemiBold,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            },
            supportingContent = {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(
                        item.content.ifBlank { ui(R.string.new_message) },
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (item.at > 0)
                        Text(
                            Instant.ofEpochMilli(item.at)
                                .atZone(ZoneId.systemDefault())
                                .format(
                                    DateTimeFormatter.ofLocalizedDateTime(FormatStyle.SHORT)
                                        .withLocale(AppLanguage.locale)
                                ),
                            style = MaterialTheme.typography.labelSmall,
                        )
                    if (!canOpen)
                        Text(
                            ui(R.string.conversation_unavailable),
                            style = MaterialTheme.typography.labelSmall,
                        )
                }
            },
            leadingContent = {
                if (item.authorId.isNotBlank()) UserAvatar(item.authorId, item.author)
                else Icon(Icons.Outlined.NotificationsNone, null)
            },
            trailingContent = {
                if (!item.read)
                    IconButton(
                        enabled = !reading,
                        onClick = {
                            onRead()
                        },
                    ) {
                        Icon(
                            Icons.Outlined.Done,
                            ui(R.string.mark_notification_as_read),
                        )
                    }
                else Icon(Icons.Outlined.ChevronRight, null)
            },
            colors =
                ListItemDefaults.colors(
                    containerColor =
                        if (item.read) MaterialTheme.colorScheme.surfaceContainerLow
                        else MaterialTheme.colorScheme.primaryContainer.copy(alpha = .4f)
                ),
            modifier =
                Modifier.testTag("notification-${server.key}-${item.id}")
                    .clip(RoundedCornerShape(20.dp))
                    .semantics {
                        stateDescription = if (item.read) uiLabel("Read") else ui(R.string.unread)
                    }
                    .clickable(
                        enabled = canOpen,
                        onClickLabel = ui(R.string.open_notification_message),
                    ) {
                        onOpen(server, item)
                    },
        )
    }
}
