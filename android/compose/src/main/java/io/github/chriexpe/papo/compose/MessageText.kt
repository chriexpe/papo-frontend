package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.chriexpe.papo.core.EmbedView
import io.github.chriexpe.papo.core.MarkdownDocument
import io.github.chriexpe.papo.core.MarkdownSpan
import kotlinx.coroutines.launch

/**
 * The snapshot already contains the shared core parser's tree. No async parse or WebView: reading
 * boundaries see the final rich-text layout immediately.
 */
@Composable
internal fun MessageText(
    message: ChatMessage,
    server: ServerState,
    model: PapoViewModel,
    onActions: () -> Unit = {},
) {
    val document = remember(message.markdown, message.content) { message.document() }
    val hidden =
        remember(message.content, message.embeds, document) {
            message.nativeLinks().map { it.url }.toSet()
        }
    MarkdownBlocks(document, document.roots, server, model, hidden, onActions, message.embeds)
}

@Composable
private fun MarkdownBlocks(
    document: MarkdownDocument,
    indices: List<UInt>,
    server: ServerState,
    model: PapoViewModel,
    hidden: Set<String>,
    onActions: () -> Unit,
    embeds: List<EmbedView>,
    depth: Int = 0,
) {
    if (depth > 64) return
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        indices.forEach { index ->
            val node = document.nodes.getOrNull(index.toInt()) ?: return@forEach
            when (node.kind) {
                "paragraph",
                "heading",
                "cell" -> {
                    val style =
                        if (node.kind == "heading")
                            when (node.level.toInt()) {
                                1 -> MaterialTheme.typography.headlineSmall
                                2 -> MaterialTheme.typography.titleLarge
                                3 -> MaterialTheme.typography.titleMedium
                                else -> MaterialTheme.typography.titleSmall
                            }.copy(fontWeight = FontWeight.Bold)
                        else MaterialTheme.typography.bodyLarge
                    MarkdownParagraph(
                        node.spans,
                        server,
                        model,
                        hidden,
                        embeds,
                        style = style,
                        onActions = onActions,
                    )
                }
                "quote" -> {
                    val border = MaterialTheme.colorScheme.outlineVariant
                    Box(
                        Modifier.fillMaxWidth()
                            .drawBehind {
                                drawLine(border, Offset.Zero, Offset(0f, size.height), 3.dp.toPx())
                            }
                            .padding(start = 12.dp)
                    ) {
                        MarkdownBlocks(
                            document,
                            node.children,
                            server,
                            model,
                            hidden,
                            onActions,
                            embeds,
                            depth + 1,
                        )
                    }
                }
                "list" ->
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        node.children.forEachIndexed { offset, itemId ->
                            val item =
                                document.nodes.getOrNull(itemId.toInt()) ?: return@forEachIndexed
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Text(
                                    if (item.task != null) {
                                        if (item.task == true) "☑" else "☐"
                                    } else node.start?.let { "${it+offset.toULong()}." } ?: "•",
                                    modifier = Modifier.widthIn(min = 18.dp),
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    style = MaterialTheme.typography.bodyLarge,
                                )
                                Box(Modifier.weight(1f)) {
                                    MarkdownBlocks(
                                        document,
                                        item.children,
                                        server,
                                        model,
                                        hidden,
                                        onActions,
                                        embeds,
                                        depth + 1,
                                    )
                                }
                            }
                        }
                    }
                "code" -> MarkdownCode(node.text, node.language)
                "rule" -> HorizontalDivider(Modifier.padding(vertical = 8.dp))
                "spacer" -> Spacer(Modifier.height(8.dp))
                "table" ->
                    Column(
                        Modifier.fillMaxWidth()
                            .horizontalScroll(rememberScrollState())
                            .background(
                                MaterialTheme.colorScheme.surfaceContainer,
                                RoundedCornerShape(8.dp),
                            )
                            .padding(6.dp)
                    ) {
                        node.children.forEachIndexed { rowIndex, rowId ->
                            val row =
                                document.nodes.getOrNull(rowId.toInt()) ?: return@forEachIndexed
                            Row {
                                row.children.forEachIndexed { column, cellId ->
                                    val cell =
                                        document.nodes.getOrNull(cellId.toInt())
                                            ?: return@forEachIndexed
                                    Box(Modifier.width(160.dp).padding(8.dp)) {
                                        MarkdownParagraph(
                                            cell.spans,
                                            server,
                                            model,
                                            hidden,
                                            embeds,
                                            style =
                                                MaterialTheme.typography.bodyMedium.copy(
                                                    fontWeight =
                                                        if (rowIndex == 0) FontWeight.Bold
                                                        else FontWeight.Normal
                                                ),
                                            textAlign =
                                                when (node.alignments.getOrNull(column)) {
                                                    "center" -> TextAlign.Center
                                                    "right" -> TextAlign.End
                                                    else -> TextAlign.Start
                                                },
                                            onActions = onActions,
                                        )
                                    }
                                }
                            }
                            if (rowIndex == 0) HorizontalDivider()
                        }
                    }
            }
        }
    }
}

@Composable
private fun MarkdownCode(text: String, language: String) {
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    Column(
        Modifier.fillMaxWidth()
            .testTag("markdown-code")
            .background(MaterialTheme.colorScheme.surfaceContainer, RoundedCornerShape(12.dp))
            .padding(horizontal = 12.dp, vertical = 4.dp)
    ) {
        Row(
            Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(
                language,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            IconButton(
                onClick = {
                    scope.launch {
                        clipboard.setClipEntry(
                            ClipEntry(
                                android.content.ClipData.newPlainText(ui(R.string.code), text)
                            )
                        )
                    }
                }
            ) {
                Icon(Icons.Outlined.ContentCopy, ui(R.string.copy_code), Modifier.size(18.dp))
            }
        }
        Text(
            text.trimEnd('\n'),
            modifier = Modifier.horizontalScroll(rememberScrollState()).padding(bottom = 8.dp),
            fontFamily = FontFamily.Monospace,
            style = MaterialTheme.typography.bodyMedium,
            softWrap = false,
        )
    }
}

/**
 * Split image spans from text without losing their order or moving Markdown images to the end of
 * the message. Use the same cache, viewer and animation retention as ordinary attachments,
 * including server-provided dimensions.
 */
@Composable
private fun MarkdownParagraph(
    spans: List<MarkdownSpan>,
    server: ServerState,
    model: PapoViewModel,
    hidden: Set<String>,
    embeds: List<EmbedView>,
    style: TextStyle = MaterialTheme.typography.bodyLarge,
    textAlign: TextAlign = TextAlign.Start,
    onActions: () -> Unit,
) {
    var start = 0
    spans.forEachIndexed { index, span ->
        if (span.kind == "image") {
            if (index > start)
                MarkdownInline(spans.subList(start, index), server, model, hidden, style, textAlign)
            val url = markdownHref(server.url, span.link)?.takeIf(::isPublicLink)
            if (url != null) {
                val embed = embeds.firstOrNull { it.url == url && it.hasImage }
                ReservedMedia(embed?.width ?: 0, embed?.height ?: 0, tag = "markdown-image") {
                    MediaImage(
                        server,
                        model,
                        span.text.ifBlank { uiLabel("Image") },
                        embed = embed,
                        publicUrl = if (embed == null) url else "",
                        gif =
                            if (directMediaKind(url) == "gif")
                                GifChoice(
                                    "url:$url",
                                    span.text.ifBlank { "GIF" },
                                    url = url,
                                    embed = embed?.id.orEmpty(),
                                    revision = embed?.revision.orEmpty(),
                                )
                            else null,
                        onActions = onActions,
                    )
                }
            }
            start = index + 1
        }
    }
    if (start < spans.size)
        MarkdownInline(spans.subList(start, spans.size), server, model, hidden, style, textAlign)
}

private data class MarkdownPart(
    val span: MarkdownSpan,
    val text: String,
    val sticker: String = "",
    val href: String = "",
)

private fun markdownParts(
    spans: List<MarkdownSpan>,
    customs: Set<String>,
    hidden: Set<String>,
): List<MarkdownPart> = buildList {
    var hidMedia = false
    val pattern =
        Regex("https?://[^\\s<>]+|:(?<sticker>[A-Za-z0-9_+\\-]+):", RegexOption.IGNORE_CASE)
    spans.forEach { span ->
        if (span.kind != "text") {
            add(MarkdownPart(span, span.text))
            return@forEach
        }
        var end = 0
        pattern.findAll(span.text).forEach { match ->
            val sticker = match.groups["sticker"]?.value?.takeIf { it in customs }
            val link =
                if (sticker == null && span.link.isBlank())
                    messageLinks(match.value).firstOrNull()?.url
                else null
            if (sticker != null || link != null) {
                if (match.range.first > end)
                    add(MarkdownPart(span, span.text.substring(end, match.range.first)))
                if (sticker != null) add(MarkdownPart(span, match.value, sticker = sticker))
                else if (link !in hidden) add(MarkdownPart(span, requireNotNull(link), href = link))
                else hidMedia = true
                end = match.range.first + (link?.length ?: match.value.length)
            }
        }
        if (end < span.text.length) add(MarkdownPart(span, span.text.substring(end)))
    }
    if (hidMedia) {
        // Hiding an inline-media URL also removes the otherwise empty edge
        // line around it, matching ordinary attachment captions. Code spans
        // retain their literal whitespace.
        fun MarkdownPart.emptyEdge() = sticker.isBlank() && span.kind != "code" && text.isBlank()
        while (isNotEmpty() && first().emptyEdge()) removeAt(0)
        while (isNotEmpty() && last().emptyEdge()) removeAt(lastIndex)
        if (isNotEmpty() && first().span.kind == "text")
            this[0] = first().copy(text = first().text.trimStart())
        if (isNotEmpty() && last().span.kind == "text")
            this[lastIndex] = last().copy(text = last().text.trimEnd())
    }
}

@Composable
private fun MarkdownInline(
    spans: List<MarkdownSpan>,
    server: ServerState,
    model: PapoViewModel,
    hidden: Set<String>,
    style: TextStyle,
    textAlign: TextAlign,
) {
    var detail by remember(server.url, server.me) { mutableStateOf<String?>(null) }
    val openProfile by rememberUpdatedState(LocalOpenProfile.current)
    val color = MaterialTheme.colorScheme.primary
    val customs = remember(server.emojis) { server.emojis.associateBy { it.name } }
    val parts = remember(spans, customs, hidden) { markdownParts(spans, customs.keys, hidden) }
    if (parts.all { it.text.isBlank() }) return
    val jumbo =
        parts.count { it.sticker.isNotBlank() } in 1..3 &&
            parts.all { it.sticker.isNotBlank() || it.text.isBlank() }
    val size = if (jumbo) 72.sp else 24.sp
    val inline =
        parts
            .filter { it.sticker.isNotBlank() }
            .associate { part ->
                val emoji = customs.getValue(part.sticker)
                emoji.id to
                    InlineTextContent(
                        Placeholder(size, size, PlaceholderVerticalAlign.TextCenter)
                    ) {
                        Box(
                            Modifier.fillMaxSize().testTag("message-sticker-${emoji.id}").clickable(
                                onClickLabel = ui(R.string.view_sticker_1_s, emoji.name)
                            ) {
                                detail = emoji.id
                            },
                            contentAlignment = Alignment.Center,
                        ) {
                            EmojiGlyph(
                                server,
                                EmojiChoice(emoji.id, true, emoji.name),
                                if (jumbo) 72 else 24,
                                model,
                            )
                        }
                    }
            }
    val codeColor = MaterialTheme.colorScheme.surfaceContainerHighest
    val annotated =
        remember(parts, color, codeColor, server.url) {
            buildAnnotatedString {
                parts.forEach { part ->
                    val span = part.span
                    val spanStyle =
                        SpanStyle(
                            fontWeight =
                                if (span.bold || span.kind == "mention") FontWeight.Bold else null,
                            fontStyle = if (span.italic) FontStyle.Italic else null,
                            textDecoration = if (span.strike) TextDecoration.LineThrough else null,
                            fontFamily = if (span.kind == "code") FontFamily.Monospace else null,
                            background = if (span.kind == "code") codeColor else Color.Unspecified,
                        )
                    withStyle(spanStyle) {
                        if (part.sticker.isNotBlank())
                            appendInlineContent(customs.getValue(part.sticker).id, part.text)
                        else if (span.kind == "mention")
                            withLink(
                                LinkAnnotation.Clickable(
                                    "profile:${span.userId}",
                                    TextLinkStyles(
                                        SpanStyle(color = color, fontWeight = FontWeight.Bold)
                                    ),
                                ) {
                                    openProfile(span.userId)
                                }
                            ) {
                                append(part.text)
                            }
                        else {
                            val href = markdownHref(server.url, part.href.ifBlank { span.link })
                            if (href != null)
                                withLink(
                                    LinkAnnotation.Url(
                                        href,
                                        TextLinkStyles(SpanStyle(color = color)),
                                    )
                                ) {
                                    append(part.text)
                                }
                            else append(part.text)
                        }
                    }
                }
            }
        }
    Text(
        annotated,
        inlineContent = inline,
        style = style,
        lineHeight = if (jumbo) 80.sp else style.lineHeight,
        textAlign = textAlign,
        modifier =
            Modifier.then(if (textAlign != TextAlign.Start) Modifier.fillMaxWidth() else Modifier),
    )
    detail?.let { StickerDetails(server, model, it, onDismiss = { detail = null }) }
}

/** Recheck the final resolved scheme before handing anything to Android. */
internal fun markdownHref(server: String, href: String): String? = runCatching {
    if (href.isBlank()) return null
    val target = java.net.URI(href)
    val resolved =
        if (href.startsWith('/') || href.startsWith('#'))
            java.net.URI(server.trimEnd('/') + "/").resolve(target)
        else target
    resolved.toString().takeIf {
        resolved.scheme?.lowercase() in setOf("http", "https", "mailto") &&
            (resolved.scheme.equals("mailto", true) || isPublicLink(it))
    }
}
    .getOrNull()
