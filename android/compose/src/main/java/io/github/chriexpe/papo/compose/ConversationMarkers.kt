package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListLayoutInfo
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.structuralEqualityPolicy
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

internal data class ReadBoundary(
    val message: String,
    val label: String,
    val tag: String,
    val before: Boolean,
    val symbol: String,
)

private fun ReadBoundary.position(layout: LazyListLayoutInfo): Int? {
    val row = layout.visibleItemsInfo.firstOrNull { it.key == message } ?: return null
    val top =
        if (layout.reverseLayout)
            layout.viewportSize.height - layout.beforeContentPadding - row.offset - row.size
        else layout.beforeContentPadding + row.offset
    return if (before) top else top + row.size
}

/**
 * Read markers are siblings of the list, never part of a measured message. Scroll coordinates are
 * read in placement, so pixel movement only repositions three small labels. Composition changes
 * only when a boundary enters/leaves the viewport or its identity/text changes.
 */
@Composable
internal fun ReadBoundaryOverlay(
    list: LazyListState,
    boundaries: List<ReadBoundary>,
    topPadding: Dp,
    bottomPadding: Dp,
    avatars: Boolean,
) {
    val visible by
        remember(list, boundaries) {
            derivedStateOf(structuralEqualityPolicy()) {
                val layout = list.layoutInfo
                val top =
                    if (layout.reverseLayout) layout.afterContentPadding
                    else layout.beforeContentPadding
                val bottom =
                    layout.viewportSize.height -
                        if (layout.reverseLayout) layout.beforeContentPadding
                        else layout.afterContentPadding
                boundaries.filter { it.position(layout)?.let { y -> y in top..bottom } == true }
            }
        }
    val drawn = visible
    Layout(
        modifier =
            Modifier.fillMaxSize().padding(top = topPadding, bottom = bottomPadding).clipToBounds(),
        content = {
            drawn.forEach { boundary ->
                Row(
                    Modifier.fillMaxWidth().testTag(boundary.tag).semantics(
                        mergeDescendants = true
                    ) {
                        contentDescription = boundary.label
                    },
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    // The avatar gutter keeps labels clear of message text. With avatars hidden,
                    // a compact arrow/dot fits the existing left inset; accessibility keeps the
                    // label.
                    Text(
                        if (avatars) boundary.label else boundary.symbol,
                        modifier =
                            Modifier.width(if (avatars) 68.dp else 12.dp)
                                .background(
                                    MaterialTheme.colorScheme.surfaceContainer,
                                    RoundedCornerShape(5.dp),
                                )
                                .padding(horizontal = if (avatars) 3.dp else 0.dp, vertical = 2.dp)
                                .clearAndSetSemantics {},
                        color = MaterialTheme.colorScheme.primary,
                        style = MaterialTheme.typography.labelSmall,
                        textAlign = TextAlign.Center,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    HorizontalDivider(color = MaterialTheme.colorScheme.primary.copy(alpha = .5f))
                }
            }
        },
    ) { measurables, constraints ->
        val inset = 4.dp.roundToPx()
        val children = measurables.map {
            it.measure(Constraints(maxWidth = (constraints.maxWidth - inset * 2).coerceAtLeast(0)))
        }
        layout(constraints.maxWidth, constraints.maxHeight) {
            val info = list.layoutInfo
            val origin = topPadding.roundToPx()
            children.forEachIndexed { index, child ->
                val y = drawn[index].position(info)?.minus(origin) ?: return@forEachIndexed
                if (y in 0..constraints.maxHeight) {
                    child.placeRelative(inset, (y - child.height / 2f).roundToInt())
                }
            }
        }
    }
}
