package io.github.chriexpe.papo.compose

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.spring
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/** One height owner across gesture and spring phases; no stale-endpoint handoff on release. */
internal class PickerHeightState(initialHeight: Float) {
    private val animation = Animatable(initialHeight)
    private var draggedHeight by mutableFloatStateOf(initialHeight)
    var dragging by mutableStateOf(false)
        private set

    // Consumers read this in measurement, not composition.
    val value: Float
        get() = if (dragging) draggedHeight else animation.value

    suspend fun startDrag(minHeight: Float, maxHeight: Float) {
        animation.stop()
        draggedHeight = value.coerceIn(minHeight, maxHeight)
        dragging = true
    }

    fun dragBy(delta: Float, minHeight: Float, maxHeight: Float) {
        draggedHeight = (draggedHeight - delta).coerceIn(minHeight, maxHeight)
    }

    suspend fun stopDrag(minHeight: Float, maxHeight: Float): Float {
        val released = draggedHeight.coerceIn(minHeight, maxHeight)
        // Seed the spring BEFORE dropping the gesture override. Otherwise the next
        // measurement sees the previous endpoint and reactivates offscreen grid items.
        animation.snapTo(released)
        dragging = false
        return released
    }

    suspend fun animateTo(targetHeight: Float) {
        if (!dragging) animation.animateTo(targetHeight, animationSpec = spring())
    }
}
