package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class PickerHeightTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun changingHeightRemeasuresWithoutRecomposingAndKeepsBottomAnchor() {
        val height = mutableFloatStateOf(100f)
        val compositions = AtomicInteger()
        compose.setContent {
            SideEffect { compositions.incrementAndGet() }
            Box(Modifier.size(200.dp).testTag("host"), contentAlignment = Alignment.BottomCenter) {
                Box(Modifier.width(80.dp).emojiPickerHeight { height.floatValue }.testTag("panel"))
            }
        }
        val count = compositions.get()
        val bottom = compose.onNodeWithTag("host").fetchSemanticsNode().boundsInRoot.bottom
        listOf(125f, 170f, 90f, 160f, 100f).forEach { next ->
            compose.runOnIdle { height.floatValue = next }
            val bounds = compose.onNodeWithTag("panel").fetchSemanticsNode().boundsInRoot
            assertEquals(next, bounds.height, .5f)
            assertEquals(bottom, bounds.bottom, .5f)
            assertEquals(
                "Height pixels must only invalidate measurement",
                count,
                compositions.get(),
            )
        }
    }

    @Test
    fun currentParentConstraintsClampTheHeightIncludingSmallImeSpace() {
        val available = mutableStateOf(200.dp)
        compose.setContent {
            Box(Modifier.width(100.dp).height(available.value).testTag("host")) {
                Box(Modifier.width(80.dp).emojiPickerHeight { 10000f }.testTag("panel"))
            }
        }
        listOf(200.dp, 80.dp, 24.dp).forEach { limit ->
            compose.runOnIdle { available.value = limit }
            val host = compose.onNodeWithTag("host").fetchSemanticsNode().boundsInRoot
            val panel = compose.onNodeWithTag("panel").fetchSemanticsNode().boundsInRoot
            assertEquals(host.height, panel.height, .5f)
            assertTrue(panel.bottom <= host.bottom + .5f)
        }
    }

    @Test
    fun heightUsesTheInitialValueAndClampsNegativeRequestsToZero() {
        val height = mutableFloatStateOf(123f)
        compose.setContent {
            Box(Modifier.size(200.dp)) {
                Box(Modifier.width(80.dp).emojiPickerHeight { height.floatValue }.testTag("panel"))
            }
        }
        assertEquals(
            123f,
            compose.onNodeWithTag("panel").fetchSemanticsNode().boundsInRoot.height,
            .5f,
        )
        compose.runOnIdle { height.floatValue = -20f }
        assertEquals(
            0f,
            compose.onNodeWithTag("panel").fetchSemanticsNode().boundsInRoot.height,
            .5f,
        )
    }
}
