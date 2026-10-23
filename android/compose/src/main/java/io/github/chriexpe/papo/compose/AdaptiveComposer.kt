package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp

/**
 * Measure against the compact editor's width even after expansion, avoiding a wrap/unwrap loop. The
 * editor keeps one composition slot: changing its placement must not recreate IME/focus state.
 */
@Composable
internal fun AdaptiveComposer(
    draft: ComposerDraft,
    editor: @Composable (Modifier) -> Unit,
    actions: @Composable () -> Unit,
) {
    val dimensions = LocalUiDimensions.current
    val density = LocalDensity.current
    val measurer = rememberTextMeasurer()
    val style = MaterialTheme.typography.bodyLarge
    BoxWithConstraints(Modifier.fillMaxWidth().padding(dimensions.surfaceInset)) {
        val actionWidth = dimensions.touchSize * 3
        val compactTextWidth =
            with(density) { (maxWidth - actionWidth - 22.dp).roundToPx().coerceAtLeast(1) }
        // Plain shortcode text is conservative while its inline drawable is still loading.
        val expanded =
            draft.text.isNotEmpty() &&
                ('\n' in draft.text ||
                    measurer
                        .measure(
                            draft.text,
                            style,
                            maxLines = 2,
                            constraints = Constraints(maxWidth = compactTextWidth),
                        )
                        .lineCount > 1)
        Layout(
            modifier = Modifier.fillMaxWidth().testTag("adaptive-composer"),
            content = {
                editor(Modifier.testTag("composer-editor-region"))
                Row(Modifier.width(actionWidth).testTag("composer-actions")) { actions() }
            },
        ) { children, constraints ->
            val width = constraints.maxWidth
            val controls = children[1].measure(Constraints(maxWidth = width))
            val editorWidth = if (expanded) width else (width - controls.width).coerceAtLeast(1)
            val input =
                children[0].measure(Constraints(minWidth = editorWidth, maxWidth = editorWidth))
            val height =
                if (expanded) input.height + controls.height
                else maxOf(input.height, controls.height)
            layout(width, height) {
                input.placeRelative(0, if (expanded) 0 else (height - input.height) / 2)
                controls.placeRelative(
                    width - controls.width,
                    if (expanded) input.height else (height - controls.height) / 2,
                )
            }
        }
    }
}
