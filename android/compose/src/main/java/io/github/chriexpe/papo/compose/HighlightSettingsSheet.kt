@file:OptIn(
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.layout.ExperimentalLayoutApi::class,
)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

/** Dedicated appearance editor: color tiles and a live sample share the same preferences as chat. */
@Composable
internal fun HighlightSettingsSheet(onDismiss: () -> Unit) {
    val prefs = LocalClientPreferences.current
    val entries = listOf(
        Triple("highlight-direct", R.string.highlight_direct, "#E7A147"),
        Triple("highlight-reply", R.string.highlight_reply, "#65A8ED"),
        Triple("highlight-role", R.string.highlight_role, "#A78BFA"),
    )
    val palette = listOf(
        "#E7A147", "#65A8ED", "#A78BFA", "#6DB89C", "#E2778C", "#D7BF62",
    )
    var previewKey by remember { mutableStateOf("highlight-direct") }
    val selectedEntry = entries.first { it.first == previewKey }
    val previewColor = prefs.highlightColor(selectedEntry.first, selectedEntry.third)

    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier.fillMaxWidth()
                .fillMaxHeight(.80f)
                .navigationBarsPadding()
                .padding(horizontal = 20.dp)
                .testTag("highlight-settings"),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                ui(R.string.message_highlighting),
                style = MaterialTheme.typography.titleLarge,
            )
            Text(
                ui(R.string.highlight_preview),
                style = MaterialTheme.typography.titleSmall,
            )
            // Select which message category the single live sample demonstrates.
            FlowRow(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                entries.forEach { (key, label, _) ->
                    FilterChip(
                        selected = previewKey == key,
                        onClick = { previewKey = key },
                        label = { Text(ui(label), maxLines = 1) },
                    )
                }
            }
            Row(
                Modifier.fillMaxWidth()
                    .clip(RoundedCornerShape(16.dp))
                    .background(previewColor.copy(alpha = .18f))
                    .testTag("highlight-preview"),
            ) {
                Box(Modifier.width(4.dp).height(100.dp).background(previewColor))
                Column(
                    Modifier.weight(1f).padding(14.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        ui(R.string.highlight_example_author),
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                    Text(
                        when (previewKey) {
                            "highlight-direct" -> ui(R.string.highlight_example_direct)
                            "highlight-role" -> ui(R.string.highlight_example_role)
                            else -> ui(R.string.highlight_example_reply)
                        },
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
            }
            Column(
                Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(18.dp),
            ) {
                entries.forEach { (key, label, fallback) ->
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(ui(label), style = MaterialTheme.typography.titleSmall)
                        Row(
                            Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            palette.forEach { hex ->
                                val color = Color(android.graphics.Color.parseColor(hex))
                                val selected = prefs.string(key, fallback) == hex
                                Box(
                                    Modifier.weight(1f)
                                        .height(48.dp)
                                        .clip(RoundedCornerShape(12.dp))
                                        .background(color)
                                        .border(
                                            if (selected) 2.dp else 0.dp,
                                            if (selected) MaterialTheme.colorScheme.onSurface
                                            else Color.Transparent,
                                            RoundedCornerShape(12.dp),
                                        )
                                        .clickable {
                                            prefs.set(key, hex)
                                            previewKey = key
                                        }
                                        .testTag("$key-$hex"),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    if (selected)
                                        Icon(
                                            Icons.Outlined.Check,
                                            contentDescription = ui(label),
                                            tint = if (color.luminance() > .45f) Color.Black else Color.White,
                                        )
                                }
                            }
                        }
                    }
                }
                SettingsSwitch(
                    ui(R.string.highlight_use_role_color),
                    ui(R.string.highlight_use_role_color_description),
                    prefs.boolean("highlight-use-role-color", true),
                    "highlight-use-role-color",
                ) {
                    prefs.set("highlight-use-role-color", it)
                }
            }
        }
    }
}
