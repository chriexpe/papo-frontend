package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

/**
 * Metadata owns the timeline footprint. Decode/player callbacks may fit content within this
 * reservation, but must not resize the row and disturb the viewport or reading boundaries.
 */
@Composable
internal fun ReservedMedia(
    width: Int,
    height: Int,
    fallback: Float = 16f / 9f,
    tag: String = "",
    background: Color = Color.Transparent,
    content: @Composable BoxScope.() -> Unit,
) {
    BoxWithConstraints(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
        val ratio = if (width > 0 && height > 0) width.toFloat() / height else fallback
        val available = maxWidth.coerceAtMost(480.dp)
        val frameHeight = (available / ratio).coerceIn(72.dp, 360.dp)
        val frameWidth = (frameHeight * ratio).coerceAtMost(available)
        Box(
            Modifier.width(frameWidth)
                .height(frameHeight)
                .clip(RoundedCornerShape(14.dp))
                .background(background)
                .testTag(tag),
            content = content,
        )
    }
}

/** Round the actual fitted artwork/player, rather than the transparent letterbox around it. */
@Composable
internal fun FittedMediaContent(
    ratio: Float?,
    modifier: Modifier = Modifier,
    content: @Composable BoxScope.() -> Unit,
) {
    BoxWithConstraints(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        val aspect = ratio?.takeIf { it.isFinite() && it > 0f }
        val width = if (aspect != null) minOf(maxWidth, maxHeight * aspect) else maxWidth
        val height = if (aspect != null) width / aspect else maxHeight
        Box(
            modifier.size(width, height).clip(RoundedCornerShape(14.dp)),
            content = content,
        )
    }
}
