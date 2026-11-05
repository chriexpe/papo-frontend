package io.github.chriexpe.papo.compose

import androidx.compose.runtime.MonotonicFrameClock
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PickerHeightStateTest {
    @Test
    fun collapseReleaseDoesNotRewindToExpandedHeight() = runBlocking {
        val state = PickerHeightState(1703f)
        state.startDrag(725f, 1703f)
        state.dragBy(948f, 725f, 1703f)
        assertEquals(755f, state.value, .001f)
        assertEquals(755f, state.stopDrag(725f, 1703f), .001f)
        assertFalse(state.dragging)
        assertEquals(755f, state.value, .001f)
    }

    @Test
    fun expandReleaseDoesNotRewindToPeekHeight() = runBlocking {
        val state = PickerHeightState(725f)
        state.startDrag(725f, 1703f)
        state.dragBy(-820f, 725f, 1703f)
        assertEquals(1545f, state.value, .001f)
        state.stopDrag(725f, 1703f)
        assertFalse(state.dragging)
        assertEquals(1545f, state.value, .001f)
    }

    @Test
    fun releaseClampsToShrinkingImeBounds() = runBlocking {
        val state = PickerHeightState(1703f)
        state.startDrag(725f, 1703f)
        state.dragBy(400f, 725f, 1703f)
        assertEquals(600f, state.stopDrag(100f, 600f), .001f)
        assertEquals(600f, state.value, .001f)
    }

    @Test
    fun targetAnimationDoesNotOverrideAnActiveDrag() = runBlocking {
        val state = PickerHeightState(725f)
        state.startDrag(725f, 1703f)
        state.dragBy(-100f, 725f, 1703f)
        // No frame clock is needed: the animation must not start during a gesture.
        state.animateTo(1703f)
        assertTrue(state.dragging)
        assertEquals(825f, state.value, .001f)
    }

    @Test
    fun springStartsAtReleaseAndStillReachesItsTarget() = runBlocking {
        val state = PickerHeightState(1703f)
        state.startDrag(725f, 1703f)
        state.dragBy(948f, 725f, 1703f)
        state.stopDrag(725f, 1703f)
        val samples = mutableListOf<Float>()
        val clock =
            object : MonotonicFrameClock {
                var time = 0L

                override suspend fun <R> withFrameNanos(onFrame: (Long) -> R): R {
                    samples += state.value
                    time += 16_666_667L
                    return onFrame(time)
                }
            }
        withContext(clock) { state.animateTo(1703f) }
        assertTrue(samples.isNotEmpty())
        assertEquals(755f, samples.first(), .001f)
        assertTrue(samples.all { it >= 754.99f && it <= 1703.01f })
        assertEquals(1703f, state.value, .001f)
    }
}
