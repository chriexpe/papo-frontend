package io.github.chriexpe.papo.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class ConversationMarkersTest {
    @get:Rule val compose = createComposeRule()

    @Test fun movingMarkersDoNotMoveReverseLayoutMessagesOrInterceptTouches() = exercise(true)

    @Test fun movingMarkersDoNotMoveNormalLayoutMessagesOrInterceptTouches() = exercise(false)

    @Test fun endMarkerTracksTheBottomOfAReverseLayoutMessage() = exercise(true, before = false)

    private fun exercise(reverse: Boolean, before: Boolean = true) {
        val marker = mutableStateOf("row-6")
        val label = if (before) "Unread below" else "Unread above"
        val avatars = mutableStateOf(true)
        val viewportHeight = mutableStateOf(400)
        var clicked: Int? = null
        lateinit var list: LazyListState
        compose.setContent {
            MaterialTheme {
                list = rememberLazyListState(initialFirstVisibleItemIndex = 4)
                Box(Modifier.size(320.dp, viewportHeight.value.dp)) {
                    LazyColumn(
                        state = list,
                        reverseLayout = reverse,
                        contentPadding = PaddingValues(top = 40.dp, bottom = 64.dp),
                        modifier = Modifier.testTag("marker-test-list"),
                    ) {
                        items((0..20).toList(), key = { "row-$it" }) { n ->
                            Box(
                                Modifier.fillMaxWidth().height(64.dp).testTag("row-$n").clickable {
                                    clicked = n
                                }
                            ) {
                                Text("Message $n")
                            }
                        }
                    }
                    ReadBoundaryOverlay(
                        list,
                        listOf(
                            ReadBoundary(
                                marker.value,
                                label,
                                "boundary",
                                before,
                                if (before) "↓" else "↑",
                            )
                        ),
                        40.dp,
                        64.dp,
                        avatars.value,
                    )
                }
            }
        }
        compose.onNodeWithTag("boundary").assertIsDisplayed()
        val boundsBefore =
            (4..7).associateWith {
                compose.onNodeWithTag("row-$it").fetchSemanticsNode().boundsInRoot
            }
        val index = list.firstVisibleItemIndex
        val offset = list.firstVisibleItemScrollOffset
        fun assertAnchor(row: Int) {
            val line = compose.onNodeWithTag("boundary").fetchSemanticsNode().boundsInRoot.center.y
            val rowBounds = compose.onNodeWithTag("row-$row").fetchSemanticsNode().boundsInRoot
            val top = if (before) rowBounds.top else rowBounds.bottom
            assertEquals("Overlay must follow the physical message boundary", top, line, 2f)
        }
        assertAnchor(6)
        compose.runOnIdle { marker.value = "row-5" }
        compose.waitForIdle()
        assertEquals(index, list.firstVisibleItemIndex)
        assertEquals(offset, list.firstVisibleItemScrollOffset)
        boundsBefore.forEach { (n, bounds) ->
            assertEquals(
                "Changing only the unread ID must not change row geometry",
                bounds,
                compose.onNodeWithTag("row-$n").fetchSemanticsNode().boundsInRoot,
            )
        }
        assertAnchor(5)
        compose.onNodeWithTag("boundary").performTouchInput {
            click(Offset(10f, center.y + if (before) 4f else -4f))
        }
        compose.runOnIdle {
            assertEquals("Read indicators must not consume message taps", 5, clicked)
        }
        compose.runOnIdle { avatars.value = false }
        compose.onNodeWithContentDescription(label).assertIsDisplayed()
        assertAnchor(5)
        compose.runOnIdle { viewportHeight.value = 340 }
        compose.waitForIdle()
        assertAnchor(5)
        compose.onNodeWithTag("marker-test-list").performScrollToIndex(15)
        compose.onNodeWithTag("boundary").assertDoesNotExist()
    }
}
