package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class HistoryPrefetchTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun edgesLoadWithoutButtonsAndPagesKeepTheVisibleAnchor() {
        var ready by mutableStateOf(false)
        var busy by mutableStateOf(false)
        var count by mutableIntStateOf(80)
        var start by mutableIntStateOf(0)
        var older = 0
        var newer = 0
        lateinit var list: LazyListState
        compose.setContent {
            list = rememberLazyListState(initialFirstVisibleItemIndex = 40)
            HistoryPrefetch(
                list,
                "account/channel",
                ready,
                true,
                true,
                busy,
                loadOlder = {
                    older++
                    busy = true
                },
                loadNewer = {
                    newer++
                    busy = true
                },
            )
            LazyColumn(
                Modifier.size(320.dp, 300.dp).testTag("history"),
                state = list,
                reverseLayout = true,
            ) {
                items(count, key = { "message-${it + start}" }) { n ->
                    Box(Modifier.height(50.dp)) { Text("Message ${n + start}") }
                }
            }
        }
        compose.onNodeWithTag("history").performScrollToIndex(76)
        compose.runOnIdle {
            assertEquals(0, older)
            ready = true
        }
        compose.waitForIdle()
        compose.runOnIdle {
            assertEquals(1, older)
            assertEquals(0, newer)
        }
        val index = list.firstVisibleItemIndex
        val offset = list.firstVisibleItemScrollOffset
        compose.runOnIdle {
            count = 160
            busy = false
        }
        compose.waitForIdle()
        compose.runOnIdle {
            assertEquals(
                "Adding older cached rows must keep the current anchor",
                index,
                list.firstVisibleItemIndex,
            )
            assertEquals(offset, list.firstVisibleItemScrollOffset)
            assertEquals("Do not repeat a request while busy or away from the edge", 1, older)
        }
        compose.onNodeWithTag("history").performScrollToIndex(1)
        compose.waitForIdle()
        compose.runOnIdle {
            assertEquals(1, newer)
            assertEquals(1, older)
        }
        val key = list.layoutInfo.visibleItemsInfo.first().key
        val newerOffset = list.firstVisibleItemScrollOffset
        compose.runOnIdle {
            start = -20
            count = 180
            busy = false
        }
        compose.waitForIdle()
        compose.runOnIdle {
            assertEquals(
                "Prepending newer rows must preserve the visible message",
                key,
                list.layoutInfo.visibleItemsInfo.first().key,
            )
            assertEquals(newerOffset, list.firstVisibleItemScrollOffset)
            assertEquals(1, newer)
        }
    }
}
