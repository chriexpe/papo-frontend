@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.chrisbanes.haze.HazeState
import dev.chrisbanes.haze.HazeStyle
import dev.chrisbanes.haze.HazeTint
import dev.chrisbanes.haze.hazeEffect
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map

internal val LocalGlass = compositionLocalOf { true }
internal val LocalHaze = compositionLocalOf<HazeState> { error(ui(R.string.missing_content_layer)) }

/** One content capture, sampled only behind floating controls. */
@Composable
internal fun GlassPill(
    modifier: Modifier = Modifier,
    radius: Int = 32,
    content: @Composable () -> Unit,
) {
    val color = MaterialTheme.colorScheme.surface
    val glass = LocalGlass.current
    val shape = RoundedCornerShape(radius.dp)
    val haze = LocalHaze.current
    Box(
        modifier
            .clip(shape)
            .then(
                if (glass)
                    Modifier.hazeEffect(
                        haze,
                        style =
                            HazeStyle(
                                backgroundColor = MaterialTheme.colorScheme.background,
                                tint = HazeTint(color.copy(alpha = 0.78f)),
                                blurRadius = 24.dp,
                                noiseFactor = 0.035f,
                                fallbackTint = HazeTint(color),
                            ),
                    )
                else Modifier.background(color)
            )
            .border(
                BorderStroke(0.7.dp, MaterialTheme.colorScheme.onSurface.copy(alpha = 0.12f)),
                shape,
            )
    ) {
        content()
    }
}

@Composable
internal fun EmptyState(title: String, detail: String) {
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 48.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Icon(
            Icons.Outlined.ChatBubbleOutline,
            null,
            Modifier.size(36.dp),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(title, style = MaterialTheme.typography.titleLarge)
        Text(
            detail,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
internal fun Avatar(
    name: String,
    modifier: Modifier = Modifier,
    shape: androidx.compose.ui.graphics.Shape = CircleShape,
) {
    val colors = listOf(Color(0xFF305C88), Color(0xFF76503C), Color(0xFF5B537E), Color(0xFF386858))
    val initials =
        name
            .split(' ')
            .filter { it.isNotBlank() }
            .take(2)
            .map { it.first() }
            .joinToString("")
            .uppercase()
    Box(
        modifier
            .size(40.dp)
            .clip(shape)
            .background(colors[(name.hashCode() and Int.MAX_VALUE) % colors.size]),
        contentAlignment = Alignment.Center,
    ) {
        Text(initials, color = Color.White, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
    }
}

@Composable
internal fun ConnectionBanner(connection: String, onRetry: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            Icons.Outlined.CloudOff,
            null,
            Modifier.size(16.dp),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            if (connection == "connecting") ui(R.string.reconnecting)
            else ui(R.string.offline_saved_messages_are_available),
            style = MaterialTheme.typography.labelMedium,
            modifier = Modifier.weight(1f).padding(horizontal = 8.dp),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        TextButton(onClick = onRetry) { Text(ui(R.string.retry)) }
    }
}

/** All app-wide failures share one material, spacing, icon and dismissal. */
@Composable
internal fun FeedbackPill(message: String, onDismiss: () -> Unit, modifier: Modifier = Modifier) {
    GlassPill(modifier.fillMaxWidth().widthIn(max = 520.dp), radius = 24) {
        Row(
            Modifier.padding(start = 16.dp, top = 8.dp, bottom = 8.dp, end = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(
                Icons.Outlined.ErrorOutline,
                null,
                Modifier.size(22.dp),
                tint = MaterialTheme.colorScheme.error,
            )
            Text(
                message,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.weight(1f),
                color = MaterialTheme.colorScheme.onSurface,
            )
            IconButton(onClick = onDismiss) {
                Icon(Icons.Outlined.Close, ui(R.string.dismiss_notification), Modifier.size(20.dp))
            }
        }
    }
}
