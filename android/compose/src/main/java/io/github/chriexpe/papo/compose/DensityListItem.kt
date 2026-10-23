package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier

/** Shared channel, person, DM and server row spacing; content can grow with system text size. */
@Composable
internal fun DensityListItem(
    headlineContent: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    supportingContent: (@Composable () -> Unit)? = null,
    leadingContent: (@Composable () -> Unit)? = null,
    trailingContent: (@Composable () -> Unit)? = null,
) {
    val dimensions = LocalUiDimensions.current
    Row(
        modifier
            .fillMaxWidth()
            .heightIn(min = dimensions.rowHeight)
            .padding(horizontal = dimensions.rowPadding, vertical = dimensions.rowVerticalPadding),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(dimensions.contentGap),
    ) {
        leadingContent?.invoke()
        Column(Modifier.weight(1f)) {
            ProvideTextStyle(MaterialTheme.typography.bodyLarge) { headlineContent() }
            if (supportingContent != null)
                CompositionLocalProvider(
                    LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant
                ) {
                    ProvideTextStyle(MaterialTheme.typography.bodyMedium) { supportingContent() }
                }
        }
        trailingContent?.invoke()
    }
}
