package io.github.chriexpe.papo.compose

import androidx.compose.animation.core.AnimationSpec
import androidx.compose.animation.core.FastOutLinearInEasing
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * A left-to-right swipe leaves the conversation. The screen follows the finger, slides out when
 * released past the threshold and springs back otherwise. Children see the drag first, so
 * horizontal content (carousels, media) keeps its own gestures, and message rows only claim the
 * opposite direction (reply), so the two never compete.
 */
@Composable
internal fun Modifier.swipeToGoBack(
    enabled: Boolean,
    onBack: () -> Unit,
    // True while the screen is displaced, so the parent can render what is underneath.
    onSwiping: (Boolean) -> Unit = {},
): Modifier {
    val threshold = with(LocalDensity.current) { 96.dp.toPx() }
    val back by rememberUpdatedState(onBack)
    val swiping by rememberUpdatedState(onSwiping)
    var active by remember { mutableStateOf(false) }
    fun setActive(value: Boolean) {
        if (active != value) {
            active = value
            swiping(value)
        }
    }
    // Read only in the draw phase, so moving the screen never recomposes or relayouts it.
    var offset by remember { mutableFloatStateOf(0f) }
    var running by remember { mutableStateOf<Job?>(null) }
    val scope = rememberCoroutineScope()
    fun animateTo(target: Float, spec: AnimationSpec<Float>, then: suspend () -> Unit = {}) {
        running?.cancel()
        running = scope.launch {
            animate(offset, target, animationSpec = spec) { value, _ -> offset = value }
            then()
        }
    }
    // The detector sits outside the translated layer, so the finger's coordinates never shift
    // with the content it is moving.
    return pointerInput(enabled, threshold) {
            if (!enabled) return@pointerInput
            val width = size.width.toFloat()
            var total = 0f
            detectHorizontalDragGestures(
                onDragStart = {
                    total = 0f
                    running?.cancel()
                },
                onDragCancel = {
                    total = 0f
                    animateTo(0f, spring(stiffness = Spring.StiffnessMedium)) { setActive(false) }
                },
                onDragEnd = {
                    val committed = total >= threshold
                    total = 0f
                    if (committed)
                        animateTo(width, tween(160, easing = FastOutLinearInEasing)) {
                            back()
                            // Stay off screen while the app swaps screens; only a screen that
                            // outlives the navigation is brought back.
                            delay(700)
                            offset = 0f
                            setActive(false)
                        }
                    else
                        animateTo(0f, spring(stiffness = Spring.StiffnessMedium)) {
                            setActive(false)
                        }
                },
                onHorizontalDrag = { change, delta ->
                    total += delta
                    // Only a rightward drag is ours; a leftward one belongs to swipe-to-reply.
                    if (total > 0f) {
                        change.consume()
                        setActive(true)
                        offset = total.coerceAtMost(width)
                    }
                },
            )
        }
        .graphicsLayer { translationX = offset }
}
