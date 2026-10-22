package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable
internal fun TypingIndicator(names: List<String>, phrase: String) {
    if (names.isEmpty()) return
    val verb =
        if (names.size == 1) phrase.ifBlank { ui(R.string.is_typing) } else ui(R.string.are_typing)
    GlassPill(
        Modifier.padding(bottom = 6.dp).testTag("typing-indicator").semantics {
            liveRegion = LiveRegionMode.Polite
        },
        radius = 16,
    ) {
        Text(
            "${names.sortedWith(String.CASE_INSENSITIVE_ORDER).joinToString()} $verb",
            Modifier.padding(horizontal = 12.dp, vertical = 5.dp),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
