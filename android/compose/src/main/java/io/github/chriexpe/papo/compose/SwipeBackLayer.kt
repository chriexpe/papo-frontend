package io.github.chriexpe.papo.compose

import androidx.compose.animation.core.animate
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import kotlin.math.abs
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/**
 * A conversation is a foreground page. A rightward drag moves only this page to reveal the actual
 * channel list rendered underneath it by the app shell.
 *
 * This direction-specific detector never consumes a left drag: SwipeToReply remains the owner of
 * leftward gestures. Vertically scrolling or horizontally scrolling children win if they have
 * already consumed the movement.
 */
@Composable
internal fun Modifier.conversationSwipeBack(
    enabled: Boolean,
    onBack: () -> Unit,
    onRevealing: (Boolean) -> Unit,
): Modifier {
    val density = LocalDensity.current
    val threshold = with(density) { 96.dp.toPx() }
    // Pointer event handlers are restricted suspend scopes: write drag state synchronously;
    // run animations in the composable's ordinary coroutine scope after release.
    var offset by remember { mutableFloatStateOf(0f) }
    var settling by remember { mutableStateOf<Job?>(null) }
    val scope = rememberCoroutineScope()
    val navigate by rememberUpdatedState(onBack)
    val reveal by rememberUpdatedState(onRevealing)

    fun settle(target: Float, committed: Boolean = false) {
        settling?.cancel()
        settling = scope.launch {
            try {
                val start = offset
                if (committed) {
                    animate(start, target, animationSpec = tween(170)) { value, _ ->
                        offset = value
                    }
                    navigate()
                } else {
                    animate(
                        start,
                        target,
                        animationSpec = spring(stiffness = Spring.StiffnessMedium),
                    ) { value, _ -> offset = value }
                }
            } finally {
                offset = 0f
                reveal(false)
            }
        }
    }

    return this.pointerInput(enabled, threshold) {
            if (!enabled) return@pointerInput
            val screenWidth = size.width.toFloat()
            awaitEachGesture {
                val down = awaitFirstDown(requireUnconsumed = false)
                settling?.cancel()
                val slop = viewConfiguration.touchSlop
                var totalX = 0f
                var totalY = 0f
                var claimed = false
                var released = false
                while (true) {
                        val event = awaitPointerEvent()
                        val change = event.changes.firstOrNull { it.id == down.id } ?: break
                        if (!change.pressed) {
                            released = true
                            break
                        }
                        val delta = change.positionChange()
                        totalX += delta.x
                        totalY += delta.y
                        if (!claimed) {
                            // Never take a leftward reply, a vertical scroll, or
                            // a drag already claimed by a media control.
                            if (
                                change.isConsumed ||
                                    totalX <= -slop ||
                                    (abs(totalY) > slop && abs(totalY) > abs(totalX))
                            )
                                break
                            if (totalX >= slop) {
                                claimed = true
                                reveal(true)
                                change.consume()
                                offset = totalX.coerceIn(0f, screenWidth)
                            }
                        } else {
                            change.consume()
                            offset = totalX.coerceIn(0f, screenWidth)
                        }
                    }
                if (claimed && released && totalX >= threshold) {
                    settle(screenWidth, committed = true)
                } else if (claimed) {
                    settle(0f)
                }
            }
        }
        .graphicsLayer { translationX = offset }
}
