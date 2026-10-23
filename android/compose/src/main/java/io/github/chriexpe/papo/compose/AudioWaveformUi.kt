package io.github.chriexpe.papo.compose

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.height
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import kotlin.math.max

internal data class WaveformState(val data: AudioWaveform? = null, val failed: Boolean = false)

/**
 * The neutral line during loading/error is intentionally not a fake envelope. Both pointer and
 * accessibility seeking use the same normalized time axis.
 */
@Composable
internal fun AudioWaveformSeek(
    state: WaveformState,
    progress: Float,
    enabled: Boolean,
    id: String,
    onSeek: (Float) -> Unit,
    modifier: Modifier = Modifier,
) {
    val played = MaterialTheme.colorScheme.primary
    val unplayed = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = .38f)
    val seek by rememberUpdatedState(onSeek)
    val levels = state.data?.levels
    Canvas(
        modifier
            .height(48.dp)
            .testTag("waveform-$id")
            .semantics {
                contentDescription = ui(R.string.audio_position)
                progressBarRangeInfo = ProgressBarRangeInfo(progress.coerceIn(0f, 1f), 0f..1f)
                stateDescription =
                    when {
                        levels != null -> ui(R.string.waveform_ready)
                        state.failed -> ui(R.string.waveform_unavailable)
                        else -> ui(R.string.loading_waveform)
                    }
                if (enabled)
                    setProgress {
                        seek(it.coerceIn(0f, 1f))
                        true
                    }
                else disabled()
            }
            .pointerInput(enabled) {
                if (enabled)
                    detectTapGestures {
                        seek((it.x / size.width.coerceAtLeast(1)).coerceIn(0f, 1f))
                    }
            }
            .pointerInput(enabled) {
                if (enabled)
                    detectDragGestures(
                        onDragStart = {
                            seek((it.x / size.width.coerceAtLeast(1)).coerceIn(0f, 1f))
                        }
                    ) { change, _ ->
                        change.consume()
                        seek((change.position.x / size.width.coerceAtLeast(1)).coerceIn(0f, 1f))
                    }
            }
    ) {
        val split = size.width * progress.coerceIn(0f, 1f)
        if (levels == null) {
            drawLine(
                unplayed,
                Offset(0f, size.height / 2),
                Offset(size.width, size.height / 2),
                2.dp.toPx(),
                StrokeCap.Round,
            )
            if (split > 0)
                drawLine(
                    played,
                    Offset(0f, size.height / 2),
                    Offset(split, size.height / 2),
                    2.dp.toPx(),
                    StrokeCap.Round,
                )
        } else {
            val bars = (size.width / 4.dp.toPx()).toInt().coerceIn(1, levels.size)
            val gap = size.width / bars
            repeat(bars) { bar ->
                val start = bar * levels.size / bars
                val end = ((bar + 1) * levels.size / bars).coerceAtLeast(start + 1)
                var level = 0f
                for (sample in start until end) level = max(level, levels[sample])
                val half = (level * 16.dp.toPx()).coerceAtLeast(1.dp.toPx())
                val x = gap * (bar + .5f)
                drawLine(
                    if (x <= split) played else unplayed,
                    Offset(x, size.height / 2 - half),
                    Offset(x, size.height / 2 + half),
                    (gap * .5f).coerceIn(1.dp.toPx(), 3.dp.toPx()),
                    StrokeCap.Round,
                )
            }
        }
    }
}
