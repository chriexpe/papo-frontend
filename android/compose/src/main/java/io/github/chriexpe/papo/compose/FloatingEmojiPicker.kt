@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.constrainHeight
import androidx.compose.ui.unit.dp
import kotlin.math.min
import kotlin.math.roundToInt

/** Resize in measurement: drag/spring pixels do not invalidate the surrounding composition. */
internal fun Modifier.emojiPickerHeight(heightPx: () -> Float): Modifier =
    layout { measurable, constraints ->
        val height = constraints.constrainHeight(heightPx().roundToInt())
        val child = measurable.measure(constraints.copy(minHeight = height, maxHeight = height))
        layout(child.width, child.height) { child.placeRelative(0, 0) }
    }

/**
 * A non-modal overlay attached to the TOP of the composer, not a Scaffold sheet. Only compose this
 * when the user opens the picker. The parent has already applied IME insets; keyboard/composer
 * movement automatically moves this overlay with it.
 */
@Composable
internal fun FloatingEmojiPicker(
    server: ServerState,
    model: PapoViewModel,
    query: String,
    onQueryChange: (String) -> Unit,
    searchFocusRequested: Boolean,
    onSearchFocus: () -> Unit,
    onClose: () -> Unit,
    onPick: (EmojiChoice) -> Unit,
    onDetail: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    BoxWithConstraints(modifier, contentAlignment = Alignment.BottomCenter) {
        val density = LocalDensity.current
        val availablePx = with(density) { maxHeight.toPx() }
        // During IME animation, never demand more space than exists above the composer.
        val expandedPx = (availablePx * .88f).coerceAtLeast(1f)
        val peekPx = min(with(density) { 276.dp.toPx() }, expandedPx)
        var expanded by remember { mutableStateOf(false) }
        // Initial height seeds one controller per open picker; bound changes must
        // update its target, not recreate/reset it during IME or window resizing.
        val height = remember { PickerHeightState(peekPx) }
        val targetHeight = if (expanded || searchFocusRequested) expandedPx else peekPx
        LaunchedEffect(height, targetHeight, height.dragging) { height.animateTo(targetHeight) }
        val dragState = rememberDraggableState { delta ->
            height.dragBy(delta, peekPx, expandedPx)
        }
        // LazyVerticalGrid participates in nested scrolling. When the user swipes up
        // inside the compact grid, expand the panel before consuming sticker scroll.
        // When the expanded grid reaches its top, downward scrolling can collapse it.
        val nestedScroll =
            remember(searchFocusRequested) {
                object : NestedScrollConnection {
                    override fun onPreScroll(
                        available: Offset,
                        source: NestedScrollSource,
                    ): Offset {
                        if (
                            source == NestedScrollSource.UserInput &&
                                !searchFocusRequested &&
                                !expanded &&
                                available.y < 0f
                        ) {
                            expanded = true
                            return Offset(0f, available.y)
                        }
                        return Offset.Zero
                    }

                    override fun onPostScroll(
                        consumed: Offset,
                        available: Offset,
                        source: NestedScrollSource,
                    ): Offset {
                        if (
                            source == NestedScrollSource.UserInput &&
                                !searchFocusRequested &&
                                expanded &&
                                available.y > 0f
                        ) {
                            expanded = false
                            return Offset(0f, available.y)
                        }
                        return Offset.Zero
                    }
                }
            }

        Surface(
            modifier =
                Modifier.fillMaxWidth()
                    .padding(horizontal = 8.dp)
                    .emojiPickerHeight {
                        val pixels = height.value
                        if (height.dragging) pixels.coerceIn(peekPx, expandedPx) else pixels
                    }
                    .nestedScroll(nestedScroll)
                    .testTag("floating-emoji-picker"),
            shape = RoundedCornerShape(24.dp),
            color = MaterialTheme.colorScheme.surface,
            tonalElevation = 4.dp,
            shadowElevation = 8.dp,
        ) {
            Column {
                Box(
                    Modifier.fillMaxWidth()
                        .height(24.dp)
                        .draggable(
                            state = dragState,
                            orientation = Orientation.Vertical,
                            enabled = !searchFocusRequested,
                            onDragStarted = { height.startDrag(peekPx, expandedPx) },
                            onDragStopped = { velocity ->
                                val releasedHeight = height.stopDrag(peekPx, expandedPx)
                                expanded =
                                    velocity < -250f ||
                                        (velocity < 250f &&
                                            releasedHeight > (peekPx + expandedPx) / 2f)
                            },
                        ),
                    contentAlignment = Alignment.Center,
                ) {
                    Box(
                        Modifier.width(36.dp)
                            .height(4.dp)
                            .background(
                                MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = .5f),
                                RoundedCornerShape(8.dp),
                            )
                    )
                }
                EmojiPickerBody(
                    server = server,
                    model = model,
                    query = query,
                    onQueryChange = onQueryChange,
                    onPick = onPick,
                    onDetail = onDetail,
                    showFavorites = true,
                    // Expansion only changes panel height. It never changes the header/grid UI.
                    compact = true,
                    searchFocusRequested = searchFocusRequested,
                    onExpand = {
                        expanded = true
                        onSearchFocus()
                    },
                    onClose = onClose,
                    modifier = Modifier.fillMaxWidth().weight(1f),
                )
            }
        }
    }
}
