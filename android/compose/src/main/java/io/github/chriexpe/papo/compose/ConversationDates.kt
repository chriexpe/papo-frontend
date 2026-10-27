package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import java.time.LocalDate
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

/** Match egui's local-calendar boundaries; an idle gap within one day needs no date heading. */
@Composable
internal fun ConversationDateHeader(date: LocalDate) {
    val today = LocalDate.now()
    val label =
        when (date) {
            today -> ui(R.string.today)
            today.minusDays(1) -> ui(R.string.yesterday)
            else ->
                date.format(
                    DateTimeFormatter.ofLocalizedDate(FormatStyle.LONG)
                        .withLocale(AppLanguage.locale)
                )
        }
    Text(
        label,
        modifier =
            Modifier.fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 16.dp)
                .testTag("conversation-date-$date")
                .semantics { heading() },
        textAlign = TextAlign.Center,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}
