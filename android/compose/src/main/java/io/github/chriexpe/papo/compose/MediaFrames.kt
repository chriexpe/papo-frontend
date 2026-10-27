package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.abs

/** Geometry learned from the existing decoder/player, scoped to one media reservation. */
internal class MediaFrameState(private val metadataRatio: Float?) {
    var ratio by mutableStateOf(metadataRatio)
        private set

    // A returning poster must not replace geometry learned from the actual video.
    private var videoKnown = false

    fun reportImageRatio(ratio: Float) {
        if (!videoKnown) resolve(ratio)
    }

    fun reportVideoRatio(ratio: Float) {
        if (ratio.isFinite() && ratio > 0f) {
            videoKnown = true
            resolve(ratio)
        }
    }

    private fun resolve(next: Float) {
        if (metadataRatio != null || !next.isFinite() || next <= 0f) return
        // Decoder downsampling can round a dimension by one pixel. Do not feed that
        // rounding back through another measure/decode cycle.
        val previous = ratio
        if (previous == null || abs(next - previous) / previous > .005f) ratio = next
    }
}

internal val LocalMediaFrame = compositionLocalOf<MediaFrameState?> { null }

/**
 * Backend dimensions size the frame before loading. Older cached messages and bare URLs can lack
 * metadata: their temporary fallback is replaced by the shared decoder/player's geometry.
 */
@Composable
internal fun ReservedMedia(
    width: Int,
    height: Int,
    fallback: Float = 16f / 9f,
    tag: String = "",
    background: Color = Color.Transparent,
    heightLimit: Dp? = 360.dp,
    content: @Composable BoxScope.() -> Unit,
) {
    val frame =
        remember(LocalMediaVisibilityRegistry.current, tag, width, height) {
            MediaFrameState(if (width > 0 && height > 0) width.toFloat() / height else null)
        }
    BoxWithConstraints(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
        val ratio = frame.ratio ?: fallback
        val available = maxWidth.coerceAtMost(480.dp)
        // Fit both bounds together. A minimum height would letterbox very wide media.
        val frameWidth = heightLimit?.let { minOf(available, it * ratio) } ?: available
        val frameHeight = frameWidth / ratio
        CompositionLocalProvider(LocalMediaFrame provides frame) {
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
