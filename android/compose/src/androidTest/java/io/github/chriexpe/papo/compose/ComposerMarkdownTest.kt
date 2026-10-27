package io.github.chriexpe.papo.compose

import androidx.compose.material3.Typography
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.sp
import io.github.chriexpe.papo.core.NativeClient
import org.junit.Assert.*
import org.junit.Test

class ComposerMarkdownTest {
    @Test
    fun previewStylesSourceWithoutChangingNativeOffsetsOrComposition() {
        val text = "# Test\n🙂 **bold** *italic* ~~gone~~ `**literal**`"
        val value = TextFieldValue(text, TextRange(12, 16), TextRange(12, 16))
        val draft = ComposerDraft(value)
        val projection = ComposerProjection(draft, emptyList(), emptySet(), Color.Blue)
        assertEquals(text, projection.annotated.text)
        assertEquals(value, draft.value)
        for (index in 0..text.length) {
            assertEquals(index, projection.mapping.originalToTransformed(index))
            assertEquals(index, projection.mapping.transformedToOriginal(index))
        }
        val styles = projection.annotated.spanStyles
        assertTrue(styles.any { it.start == 0 && it.end == 6 && it.item.fontSize == 24.sp })
        assertTrue(
            styles.any {
                text.substring(it.start, it.end) == "**bold**" &&
                    it.item.fontWeight == FontWeight.Bold
            }
        )
        assertTrue(
            styles.any {
                text.substring(it.start, it.end) == "*italic*" &&
                    it.item.fontStyle == FontStyle.Italic
            }
        )
        assertTrue(
            styles.any {
                text.substring(it.start, it.end) == "~~gone~~" &&
                    it.item.textDecoration == TextDecoration.LineThrough
            }
        )
        assertTrue(
            styles.any {
                text.substring(it.start, it.end) == "`**literal**`" &&
                    it.item.fontFamily == FontFamily.Monospace
            }
        )
        assertFalse(
            styles.any { it.start > text.indexOf('`') && it.item.fontWeight == FontWeight.Bold }
        )
        assertTrue(projection.annotated.paragraphStyles.isEmpty())
    }

    @Test
    fun boundNamesCannotIntroduceMarkdownAndTransientSurrogatesStayEditable() {
        val text = "@**Ana** plain *italic*"
        val draft =
            ComposerDraft(TextFieldValue(text), listOf(ComposerMention(0, "**Ana**", "ana")))
        val spans = composerMarkdownSpans(draft)
        assertEquals(listOf("italic"), spans.map { it.kind })
        assertEquals(
            "*italic*",
            text.substring(spans.single().start.toInt(), spans.single().end.toInt()),
        )
        assertTrue(
            composerMarkdownSpans(ComposerDraft(TextFieldValue("\uD83D **bold**"))).isEmpty()
        )
        assertTrue(composerMarkdownSpans(ComposerDraft(TextFieldValue("**unfinished"))).isEmpty())
        assertTrue(composerMarkdownSpans(ComposerDraft(TextFieldValue("\\*literal\\*"))).isEmpty())
    }

    @Test
    fun stickerCompressionKeepsFollowingStylesAndCodeShortcodesLiteral() {
        val client = NativeClient.demo()
        try {
            val text = ":papo_wave: 🙂 **bold** `:papo_wave:`\n\n```\n:papo_wave:\n```"
            val draft = ComposerDraft(TextFieldValue(text))
            val markdown = composerMarkdownSpans(draft)
            val stickers = composerStickers(text, client.poll().emojis, markdown)
            assertEquals(1, stickers.size)
            val projection =
                ComposerProjection(
                    draft,
                    stickers,
                    setOf("demo-wave"),
                    Color.Blue,
                    markdown = markdown,
                )
            assertEquals("\u3000" + text.substring(11), projection.annotated.text)
            val bold =
                projection.annotated.spanStyles.single { it.item.fontWeight == FontWeight.Bold }
            assertEquals("**bold**", projection.annotated.text.substring(bold.start, bold.end))
            assertEquals(5, bold.start)
            val mapping = projection.mapping
            assertEquals(
                (0..text.length).map(mapping::originalToTransformed).sorted(),
                (0..text.length).map(mapping::originalToTransformed),
            )
            for (offset in 11..text.length) {
                assertEquals(
                    offset,
                    mapping.transformedToOriginal(mapping.originalToTransformed(offset)),
                )
            }
        } finally {
            client.shutdown()
            client.close()
        }
    }

    @Test
    fun headingUsesAccountTypographyAndDoesNotBleedIntoNextParagraph() {
        val text = "## Title\nbody\n\n### Smaller"
        val typography =
            Typography().let {
                it.copy(titleLarge = it.titleLarge.copy(fontSize = 33.sp, lineHeight = 42.sp))
            }
        val projection =
            ComposerProjection(
                ComposerDraft(TextFieldValue(text)),
                emptyList(),
                emptySet(),
                Color.Blue,
                typography = typography,
            )
        val heading = projection.annotated.spanStyles.first()
        assertEquals(33.sp, heading.item.fontSize)
        assertEquals("## Title", text.substring(heading.start, heading.end))
        assertTrue(projection.annotated.paragraphStyles.isEmpty())
        assertTrue(projection.annotated.spanStyles.none { it.start <= 9 && it.end > 9 })
    }
}
