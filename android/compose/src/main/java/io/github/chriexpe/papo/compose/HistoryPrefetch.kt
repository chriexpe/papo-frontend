package io.github.chriexpe.papo.compose

import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.snapshotFlow
import kotlinx.coroutines.flow.distinctUntilChanged

/** Observe list geometry without putting scroll offsets into message composition. */
@Composable
internal fun HistoryPrefetch(
    list: LazyListState,
    channelKey: String,
    ready: Boolean,
    canLoadOlder: Boolean,
    canLoadNewer: Boolean,
    loading: Boolean,
    loadOlder: () -> Unit,
    loadNewer: () -> Unit,
) {
    val enabled by rememberUpdatedState(ready)
    val older by rememberUpdatedState(canLoadOlder)
    val newer by rememberUpdatedState(canLoadNewer)
    val busy by rememberUpdatedState(loading)
    val requestOlder by rememberUpdatedState(loadOlder)
    val requestNewer by rememberUpdatedState(loadNewer)
    LaunchedEffect(list, channelKey) {
        snapshotFlow {
            val layout = list.layoutInfo
            val items = layout.visibleItemsInfo
            if (!enabled || busy || items.isEmpty()) null
            // The list is reversed: index zero is the newer end. Start a page before reaching
            // an edge, allowing disk reads and server refreshes to overlap ordinary scrolling.
            else if (older && items.last().index >= layout.totalItemsCount - 8) true
            else if (newer && items.first().index <= 6) false else null
        }
            .distinctUntilChanged()
            .collect { direction ->
                when (direction) {
                    true -> requestOlder()
                    false -> requestNewer()
                    null -> Unit
                }
            }
    }
}
