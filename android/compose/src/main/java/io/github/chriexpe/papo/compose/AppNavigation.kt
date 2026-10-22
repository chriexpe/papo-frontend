@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource

@Composable
internal fun FloatingNavigation(
    selected: String,
    onSelect: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier.widthIn(max = 400.dp).fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        GlassPill(Modifier.weight(1f).testTag("chat-navigation")) {
            Row(Modifier.padding(6.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                NavigationDestination(
                    "Chats",
                    Icons.Outlined.Forum,
                    selected == "Chats",
                    { onSelect("Chats") },
                    Modifier.weight(1f),
                )
                NavigationDestination(
                    "Direct",
                    Icons.Outlined.ChatBubbleOutline,
                    selected == "Direct",
                    { onSelect("Direct") },
                    Modifier.weight(1f),
                )
            }
        }
        GlassPill(Modifier.size(72.dp).testTag("servers-navigation"), radius = 24) {
            NavigationDestination(
                "Servers",
                Icons.Outlined.Dns,
                selected == "Servers",
                { onSelect("Servers") },
                Modifier.fillMaxSize().padding(6.dp),
                radius = 18,
            )
        }
    }
}

@Composable
private fun NavigationDestination(
    name: String,
    icon: ImageVector,
    active: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    radius: Int = 25,
) {
    Column(
        modifier
            .clip(RoundedCornerShape(radius.dp))
            .background(
                if (active) MaterialTheme.colorScheme.primaryContainer else Color.Transparent
            )
            .selectable(selected = active, onClick = onClick, role = Role.Tab)
            .heightIn(min = 58.dp)
            .padding(horizontal = 10.dp, vertical = 8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(3.dp, Alignment.CenterVertically),
    ) {
        Icon(
            icon,
            null,
            Modifier.size(22.dp),
            tint =
                if (active) MaterialTheme.colorScheme.primary
                else MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            uiLabel(name),
            style = MaterialTheme.typography.labelSmall,
            fontWeight = if (active) FontWeight.Bold else FontWeight.Normal,
            maxLines = 1,
        )
    }
}

@Composable
internal fun ServerList(
    state: PapoState,
    model: PapoViewModel,
    onAdd: () -> Unit,
    onSelect: (String) -> Unit,
    onDirect: () -> Unit,
    onNotifications: () -> Unit,
) {
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current).statusBarsPadding(),
        contentPadding = PaddingValues(24.dp, 24.dp, 24.dp, 140.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            Row(
                Modifier.fillMaxWidth().testTag("global-actions"),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                GlobalAction(
                    ui(R.string.direct_messages),
                    Icons.Outlined.ChatBubbleOutline,
                    state.servers
                        .filter { it.screen == "chat" }
                        .sumOf { s -> s.dms.count { it.unread } },
                    onDirect,
                    Modifier.weight(1f),
                )
                GlobalAction(
                    ui(R.string.notifications),
                    Icons.Outlined.NotificationsNone,
                    state.servers
                        .filter { it.screen == "chat" }
                        .sumOf { it.notificationCount.toLong() }
                        .coerceAtMost(Int.MAX_VALUE.toLong())
                        .toInt(),
                    onNotifications,
                    Modifier.weight(1f),
                )
            }
        }
        item {
            Text(
                ui(R.string.your_servers),
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 16.dp, start = 8.dp, bottom = 4.dp),
            )
        }
        items(state.servers, key = { it.url }) { server ->
            ListItem(
                headlineContent = { Text(server.name, fontWeight = FontWeight.SemiBold) },
                supportingContent = {
                    Text(server.url, maxLines = 1, overflow = TextOverflow.Ellipsis)
                },
                leadingContent = { ServerIcon(server, model) },
                trailingContent = {
                    Icon(
                        if (state.activeUrl == server.url) Icons.Outlined.CheckCircle
                        else Icons.Outlined.ChevronRight,
                        null,
                    )
                },
                modifier =
                    Modifier.clip(RoundedCornerShape(22.dp)).clickable { onSelect(server.url) },
            )
        }
        item {
            OutlinedButton(
                onClick = onAdd,
                modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp),
            ) {
                Icon(Icons.Outlined.Add, null)
                Spacer(Modifier.width(8.dp))
                Text(ui(R.string.add_server))
            }
        }
    }
}
