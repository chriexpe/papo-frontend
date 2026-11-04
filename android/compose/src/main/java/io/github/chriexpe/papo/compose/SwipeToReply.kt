package io.github.chriexpe.papo.compose

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Reply
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.AbsoluteAlignment
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.PointerInputChange
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import kotlin.math.abs

/**
 * Horizontal intent must win touch slop before taking over. Child scrubbers and vertical scrolling
 * can consume the gesture first. Only release commits.
 */
@Composable
internal fun SwipeToReply(
    id: String,
    enabled: Boolean,
    onReply: () -> Unit,
    content: @Composable () -> Unit,
) {
    val threshold = with(LocalDensity.current) { 64.dp.toPx() }
    val limit = with(LocalDensity.current) { 88.dp.toPx() }
    val haptics = LocalHapticFeedback.current
    val reply by rememberUpdatedState(onReply)
    var offset by remember(id) { mutableFloatStateOf(0f) }
    var dragging by remember(id) { mutableStateOf(false) }
    var notified by remember(id) { mutableStateOf(false) }
    val position by
        animateFloatAsState(
            offset,
            if (dragging) snap() else tween(180),
            label = ui(R.string.reply_swipe),
        )
    val progress = (abs(position) / threshold).coerceIn(0f, 1f)
    Box(
        Modifier.fillMaxWidth().clipToBounds().pointerInput(id, enabled, threshold, limit) {
            try {
                if (enabled)
                    detectLeftwardDrag(
                        onDragStart = {
                            dragging = true
                            notified = false
                        },
                        onDragCancel = {
                            dragging = false
                            offset = 0f
                        },
                        onDragEnd = {
                            val committed = offset <= -threshold
                            dragging = false
                            offset = 0f
                            if (committed) reply()
                        },
                        onHorizontalDrag = { change, delta ->
                            offset = (offset + delta).coerceIn(-limit, 0f)
                            if (offset < 0f) change.consume()
                            if (offset <= -threshold && !notified) {
                                notified = true
                                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                            }
                        },
                    )
            } finally {
                dragging = false
                offset = 0f
            }
        }
    ) {
        if (progress > 0f)
            Surface(
                shape = CircleShape,
                color = MaterialTheme.colorScheme.primaryContainer,
                modifier =
                    Modifier.align(AbsoluteAlignment.CenterRight)
                        .absolutePadding(right = 24.dp)
                        .size(36.dp)
                        .graphicsLayer {
                            alpha = progress
                            scaleX = .75f + .25f * progress
                            scaleY = scaleX
                        },
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Icon(
                        Icons.Outlined.Reply,
                        null,
                        Modifier.size(20.dp),
                        tint = MaterialTheme.colorScheme.primary,
                    )
                }
            }
        Box(Modifier.fillMaxWidth().graphicsLayer { translationX = position }) { content() }
    }
}

/**
 * Own leftward drags only. A rightward gesture belongs to the conversation back layer, whereas
 * consumed gestures (media scrubbers / horizontal carousels) stay with their child. Keeping this
 * direction check before consuming touch slop is crucial: #132's generic horizontal detector could
 * win a right swipe and break reply.
 */
private suspend fun PointerInputScope.detectLeftwardDrag(
    onDragStart: () -> Unit,
    onDragCancel: () -> Unit,
    onDragEnd: () -> Unit,
    onHorizontalDrag: (PointerInputChange, Float) -> Unit,
) = awaitEachGesture {
    val down = awaitFirstDown(requireUnconsumed = false)
    val slop = viewConfiguration.touchSlop
    var dx = 0f
    var dy = 0f
    var dragging = false
    while (true) {
        val event = awaitPointerEvent()
        val change = event.changes.firstOrNull { it.id == down.id }
        if (change == null) {
            if (dragging) onDragCancel()
            return@awaitEachGesture
        }
        if (!change.pressed) {
            if (dragging) onDragEnd()
            return@awaitEachGesture
        }
        val delta = change.positionChange()
        dx += delta.x
        dy += delta.y
        if (!dragging) {
            if (change.isConsumed || (abs(dy) > slop && abs(dy) > abs(dx)) || dx >= slop)
                return@awaitEachGesture
            if (dx <= -slop) {
                dragging = true
                onDragStart()
                change.consume()
                onHorizontalDrag(change, dx)
            }
        } else {
            change.consume()
            onHorizontalDrag(change, delta.x)
        }
    }
}
