@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
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
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

internal val LocalSettingTarget = compositionLocalOf { "" }

@Composable
internal fun settingTarget(id: String): Modifier {
    val selected = id.isNotBlank() && LocalSettingTarget.current == id
    val requester = remember { BringIntoViewRequester() }
    var highlighted by remember { mutableStateOf(false) }
    LaunchedEffect(selected) {
        if (selected) {
            withFrameNanos {}
            requester.bringIntoView()
            highlighted = true
            kotlinx.coroutines.delay(2200)
            highlighted = false
        }
    }
    return Modifier.bringIntoViewRequester(requester)
        .background(
            if (highlighted) MaterialTheme.colorScheme.primaryContainer else Color.Transparent,
            RoundedCornerShape(14.dp),
        )
}

@Composable
internal fun SettingsHeader(
    title: String,
    onBack: () -> Unit,
    close: Boolean = false,
    enabled: Boolean = true,
    actions: @Composable RowScope.() -> Unit = {},
) {
    Row(
        Modifier.fillMaxWidth()
            .statusBarsPadding()
            .padding(
                horizontal = LocalUiDimensions.current.floatingPadding,
                vertical = LocalUiDimensions.current.rowVerticalPadding,
            ),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        GlassPill {
            IconButton(enabled = enabled, onClick = onBack) {
                Icon(
                    if (close) Icons.Outlined.Close else Icons.Outlined.ArrowBack,
                    if (close) ui(R.string.close_server_settings)
                    else ui(R.string.back_from_1_s, title),
                )
            }
        }
        Text(
            title,
            Modifier.weight(1f).padding(horizontal = 16.dp),
            style = MaterialTheme.typography.headlineSmall,
            fontWeight = FontWeight.SemiBold,
        )
        actions()
    }
}

@Composable
internal fun SettingsCard(content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier.fillMaxWidth()
            .clip(RoundedCornerShape(24.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerLow),
        content = content,
    )
}

@Composable
internal fun SettingsLink(
    title: String,
    subtitle: String,
    icon: ImageVector,
    onClick: () -> Unit,
    leading: (@Composable () -> Unit)? = null,
    target: String = "",
) {
    DensityListItem(
        headlineContent = { Text(title, fontWeight = FontWeight.Medium) },
        supportingContent = { Text(subtitle) },
        leadingContent = {
            if (leading != null) leading()
            else
                Surface(
                    shape = RoundedCornerShape(14.dp),
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                ) {
                    Icon(icon, null, Modifier.padding(LocalUiDimensions.current.contentGap))
                }
        },
        trailingContent = { Icon(Icons.Outlined.ChevronRight, null) },
        modifier =
            Modifier.then(settingTarget(target))
                .clickable(onClick = onClick)
                .heightIn(min = LocalUiDimensions.current.settingsLinkHeight),
    )
}

@Composable
internal fun SettingsLabel(text: String) {
    Text(
        text,
        Modifier.padding(start = 12.dp, top = 8.dp),
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.primary,
    )
}

@Composable
internal fun SettingsSwitch(
    title: String,
    subtitle: String,
    value: Boolean,
    id: String,
    onChange: (Boolean) -> Unit,
) {
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = { Text(subtitle) },
        trailingContent = {
            Switch(value, onChange, Modifier.semantics { contentDescription = title })
        },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.then(settingTarget(id)).heightIn(min = 72.dp),
    )
}

@Composable
internal fun SettingsDivider() {
    HorizontalDivider(
        Modifier.padding(horizontal = 16.dp),
        color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = .5f),
    )
}

@Composable
internal fun SettingsError(message: String, retry: (() -> Unit)? = null) {
    Surface(
        color = MaterialTheme.colorScheme.errorContainer,
        shape = RoundedCornerShape(18.dp),
        modifier = Modifier.fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                message,
                color = MaterialTheme.colorScheme.onErrorContainer,
                style = MaterialTheme.typography.bodyMedium,
            )
            if (retry != null) TextButton(onClick = retry) { Text(ui(R.string.try_again)) }
        }
    }
}
