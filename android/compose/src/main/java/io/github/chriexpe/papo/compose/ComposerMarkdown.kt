package io.github.chriexpe.papo.compose

import androidx.compose.material3.Typography
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import io.github.chriexpe.papo.core.ComposerMarkdownSpan
import io.github.chriexpe.papo.core.previewComposerMarkdown

@Composable
internal fun rememberComposerMarkdown(draft: ComposerDraft): List<ComposerMarkdownSpan> =
    remember(draft.text, draft.mentions) { composerMarkdownSpans(draft) }

internal fun composerMarkdownSpans(draft: ComposerDraft): List<ComposerMarkdownSpan> {
    if (!draft.text.validUnicode()) return emptyList()
    // Bound display names can contain Markdown punctuation. Mask them without changing UTF-16
    // positions, so a name's asterisks cannot accidentally format the rest of the draft.
    val mentions = draft.mentions.filter { it.valid(draft.text) }
    val source =
        if (mentions.isEmpty()) draft.text
        else
            draft.text
                .toCharArray()
                .also { chars ->
                    mentions.forEach { mention ->
                        for (index in mention.start until mention.end) if (
                            chars[index] != '\n' && chars[index] != '\r'
                        )
                            chars[index] = 'x'
                    }
                }
                .concatToString()
    return previewComposerMarkdown(source)
}

/**
 * Visual spans only: delimiters, native selection, IME composition and sent Markdown stay intact.
 */
internal fun AnnotatedString.Builder.addComposerMarkdown(
    spans: List<ComposerMarkdownSpan>,
    offset: (Int) -> Int,
    typography: Typography,
    color: Color,
    codeBackground: Color,
    quoteColor: Color,
) {
    spans.forEach { span ->
        val start = offset(span.start.toInt())
        val end = offset(span.end.toInt())
        if (start >= end) return@forEach
        val style =
            when (span.kind) {
                "heading" -> {
                    val heading =
                        when (span.level.toInt()) {
                            1 -> typography.headlineSmall
                            2 -> typography.titleLarge
                            3 -> typography.titleMedium
                            else -> typography.titleSmall
                        }.copy(fontWeight = FontWeight.Bold)
                    // Inline size changes keep the editor's existing line breaks. Paragraph
                    // annotations would create an additional break around a styled heading.
                    heading.toSpanStyle()
                }
                "bold" -> SpanStyle(fontWeight = FontWeight.Bold)
                "italic" -> SpanStyle(fontStyle = FontStyle.Italic)
                "strike" -> SpanStyle(textDecoration = TextDecoration.LineThrough)
                "code",
                "code-block" ->
                    SpanStyle(fontFamily = FontFamily.Monospace, background = codeBackground)
                "link" -> SpanStyle(color = color)
                "quote" -> SpanStyle(color = quoteColor)
                else -> return@forEach
            }
        addStyle(style, start, end)
    }
}
