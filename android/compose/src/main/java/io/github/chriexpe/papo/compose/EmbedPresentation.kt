package io.github.chriexpe.papo.compose

import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.EmbedDetailsView
import io.github.chriexpe.papo.core.EmbedQuoteView
import io.github.chriexpe.papo.core.EmbedView
import java.util.Locale
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay

internal data class EmbedPresentation(
    val author: String,
    val handle: String,
    val title: String,
    val caption: String,
    val statistics: String,
    val quote: EmbedQuoteView? = null,
)

private fun embedCount(value: ULong): String {
    val (divisor, suffix) =
        when {
            value >= 1_000_000_000UL -> 1_000_000_000.0 to "B"
            value >= 1_000_000UL -> 1_000_000.0 to "M"
            value >= 1_000UL -> 1_000.0 to "K"
            else -> return value.toString()
        }
    return String.format(Locale.ROOT, "%.1f", value.toDouble() / divisor).removeSuffix(".0") +
        suffix
}

internal fun EmbedView.presentation(extra: EmbedDetailsView? = null): EmbedPresentation {
    val socialTitle =
        Regex("^(.+?)\\s*\\((@[^()\\s]+)\\)(?:\\s+on\\s+.+)?$").matchEntire(title.trim())
    val firstLine = description.lineSequence().firstOrNull().orEmpty().trim()
    val suppliedStats =
        firstLine
            .takeIf { it.startsWith("❤️") || it.startsWith("❤") || it.startsWith("💬") }
            .orEmpty()
    val counts =
        listOf(
                "💬" to extra?.comments,
                "🔁" to extra?.shares,
                "❤️" to extra?.likes,
                "👁" to extra?.views,
            )
            .mapNotNull { (symbol, count) -> count?.let { "$symbol ${embedCount(it)}" } }
            .joinToString("   ")
    return EmbedPresentation(
        author
            .ifBlank { extra?.author.orEmpty() }
            .ifBlank { socialTitle?.groupValues?.get(1)?.trim().orEmpty() },
        extra?.handle.orEmpty().ifBlank { socialTitle?.groupValues?.get(2).orEmpty() },
        if (socialTitle != null || title == author) "" else title,
        extra?.caption
            ?: if (suppliedStats.isNotEmpty()) description.substringAfter('\n', "").trim()
            else description.trim(),
        counts.ifBlank { suppliedStats },
        extra?.quote,
    )
}

/** Follow the actual link that produced a canonical backend embed, including embed-fix mirrors. */
internal fun embedSourceUrl(message: ChatMessage, embed: EmbedView): String {
    val links = message.contextLinks()
    val path = Uri.parse(embed.url).encodedPath
    return links.firstOrNull { it == embed.url }
        ?: links.firstOrNull { path?.isNotBlank() == true && Uri.parse(it).encodedPath == path }
        ?: embed.url
}

@Composable
internal fun rememberEmbedDetails(
    embed: EmbedView,
    server: ServerState,
    model: PapoViewModel,
    source: String,
    enabled: Boolean,
): EmbedDetailsView? {
    val details by
        produceState<EmbedDetailsView?>(null, server.url, server.me, embed.id, source, enabled) {
            if (enabled && !server.isDemo && isPublicLink(source)) {
                try {
                    repeat(150) {
                        val next = model.embedDetails(server, embed.id, source)
                        if (!next.pending) {
                            value = next
                            return@produceState
                        }
                        delay(200)
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    /* Backend fields remain usable when page enrichment is unavailable. */
                }
            }
        }
    return details
}

@Composable
internal fun EmbedCaption(
    text: String,
    style: TextStyle = MaterialTheme.typography.bodyMedium,
    maxLines: Int = Int.MAX_VALUE,
    modifier: Modifier = Modifier,
) {
    val handler = LocalUriHandler.current
    val color = MaterialTheme.colorScheme.primary
    val annotated =
        remember(text, color, handler) {
            buildAnnotatedString {
                append(text)
                messageLinks(text).forEach { link ->
                    addLink(
                        LinkAnnotation.Clickable(
                            link.url,
                            TextLinkStyles(SpanStyle(color = color)),
                        ) {
                            handler.openUri(link.url)
                        },
                        link.start,
                        link.end,
                    )
                }
            }
        }
    Text(
        annotated,
        style = style,
        maxLines = maxLines,
        overflow = TextOverflow.Ellipsis,
        modifier = modifier,
    )
}

@Composable
internal fun EmbedAuthor(
    embed: EmbedView,
    server: ServerState,
    model: PapoViewModel,
    modifier: Modifier = Modifier,
    extra: EmbedDetailsView? = null,
) {
    val details = remember(embed, extra) { embed.presentation(extra) }
    val handler = LocalUriHandler.current
    val authorUrl = embed.authorUrl.ifBlank { extra?.authorUrl.orEmpty() }
    if (details.author.isBlank()) return
    Row(
        modifier.then(
            if (isPublicLink(authorUrl)) Modifier.clickable { handler.openUri(authorUrl) }
            else Modifier
        ),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Box(
            Modifier.size(36.dp)
                .testTag("embed-avatar-${embed.id}")
                .clip(CircleShape)
                .background(MaterialTheme.colorScheme.surfaceContainerHighest),
            contentAlignment = Alignment.Center,
        ) {
            when {
                embed.hasAuthorImage ->
                    MediaImage(
                        server,
                        model,
                        details.author,
                        embed = embed,
                        author = true,
                        viewer = false,
                    )
                extra?.avatar?.let(::isPublicLink) == true ->
                    MediaImage(
                        server,
                        model,
                        details.author,
                        publicUrl = extra.avatar,
                        viewer = false,
                    )
                else ->
                    Text(
                        details.author
                            .substring(0, details.author.offsetByCodePoints(0, 1))
                            .uppercase(),
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.primary,
                    )
            }
        }
        Column(Modifier.weight(1f)) {
            Text(
                details.author,
                style = MaterialTheme.typography.titleSmall,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            if (details.handle.isNotBlank())
                Text(
                    details.handle,
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
        }
    }
}

@Composable
private fun EmbedQuote(quote: EmbedQuoteView) {
    Row(
        Modifier.fillMaxWidth().height(IntrinsicSize.Min).testTag("embed-quote"),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Box(
            Modifier.width(3.dp)
                .fillMaxHeight()
                .clip(RoundedCornerShape(2.dp))
                .background(MaterialTheme.colorScheme.outlineVariant)
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                ui(
                    R.string.embed_quoting,
                    quote.author + if (quote.handle.isNotBlank()) " (${quote.handle})" else "",
                ),
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.primary,
            )
            EmbedCaption(quote.caption)
        }
    }
}

@Composable
internal fun SocialEmbedCard(
    embed: EmbedView,
    server: ServerState,
    model: PapoViewModel,
    extra: EmbedDetailsView? = null,
    media: @Composable () -> Unit,
) {
    val details = remember(embed, extra) { embed.presentation(extra) }
    Surface(
        Modifier.fillMaxWidth().testTag("social-embed-${embed.id}"),
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surfaceContainer,
    ) {
        Column(
            verticalArrangement = Arrangement.spacedBy(14.dp),
            modifier = Modifier.padding(14.dp),
        ) {
            EmbedAuthor(embed, server, model, extra = extra)
            if (details.title.isNotBlank())
                EmbedCaption(details.title, MaterialTheme.typography.titleSmall)
            if (details.caption.isNotBlank()) EmbedCaption(details.caption)
            details.quote?.let { EmbedQuote(it) }
            embed.fields.forEach { field ->
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(field.name, style = MaterialTheme.typography.labelLarge)
                    EmbedCaption(field.value)
                }
            }
            if (details.statistics.isNotBlank())
                Text(
                    details.statistics,
                    modifier = Modifier.testTag("embed-statistics"),
                    style = MaterialTheme.typography.labelLarge,
                    fontWeight = FontWeight.SemiBold,
                )
            if (embed.kind in listOf("image", "video", "iframe")) media()
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                val provider = extra?.provider.orEmpty().ifBlank { embed.provider }
                Text(
                    listOf(provider, embed.footer)
                        .filter(String::isNotBlank)
                        .distinct()
                        .joinToString(" · "),
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                EmbedOriginalLink(embed)
            }
        }
    }
}

@Composable
internal fun EmbedOriginalLink(embed: EmbedView) {
    val handler = LocalUriHandler.current
    if (isPublicLink(embed.url))
        IconButton(onClick = { handler.openUri(embed.url) }) {
            Icon(Icons.AutoMirrored.Outlined.OpenInNew, ui(R.string.open_original))
        }
}

@Composable
internal fun BoxScope.EmbedVideoOverlay(
    embed: EmbedView,
    server: ServerState,
    model: PapoViewModel,
    extra: EmbedDetailsView? = null,
) {
    val details = remember(embed, extra) { embed.presentation(extra) }
    BoxWithConstraints(Modifier.matchParentSize()) {
        val compact = maxHeight < 400.dp
        Column(
            Modifier.align(if (compact) Alignment.TopStart else Alignment.BottomStart)
                .fillMaxWidth()
                .background(
                    Brush.verticalGradient(
                        if (compact) listOf(Color.Black.copy(alpha = .75f), Color.Transparent)
                        else listOf(Color.Transparent, Color.Black.copy(alpha = .82f))
                    )
                )
                .padding(
                    start = 18.dp,
                    end = if (compact) 130.dp else 18.dp,
                    top = if (compact) 16.dp else 36.dp,
                    bottom = if (compact) 20.dp else 104.dp,
                )
                .testTag("viewer-embed-details"),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            EmbedAuthor(embed, server, model, extra = extra)
            if (details.title.isNotBlank())
                EmbedCaption(details.title, MaterialTheme.typography.titleSmall, 2)
            if (!compact && details.caption.isNotBlank())
                EmbedCaption(details.caption, maxLines = 3)
            if (!compact)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                        if (details.statistics.isNotBlank())
                            Text(details.statistics, style = MaterialTheme.typography.labelLarge)
                        val provider = extra?.provider.orEmpty().ifBlank { embed.provider }
                        if (provider.isNotBlank())
                            Text(
                                provider,
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                    }
                    EmbedOriginalLink(embed)
                }
        }
    }
}
