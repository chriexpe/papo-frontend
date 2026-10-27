package io.github.chriexpe.papo.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Reply
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** This prefix uses the existing rich text renderer, including its links, mentions and stickers. */
@Composable
internal fun compactAuthorPrefix(message: ChatMessage): MessageTextPrefix {
    val openProfile by rememberUpdatedState(LocalOpenProfile.current)
    val openExternal by rememberUpdatedState(LocalOpenExternalProfile.current)
    val external = message.externalAuthor
    val color =
        if (external == null) userNameColor(message.authorId)
        else MaterialTheme.colorScheme.onSurface
    val text = buildAnnotatedString {
        withLink(
            LinkAnnotation.Clickable(
                "author:${message.id}",
                TextLinkStyles(SpanStyle(color = color, fontWeight = FontWeight.SemiBold)),
            ) {
                if (external == null) openProfile(message.authorId) else openExternal(external)
            }
        ) {
            append(message.author)
        }
        if (external != null) {
            append(" ")
            appendInlineContent("compact-app-badge", "APP")
        }
        append("  ")
    }
    val inline =
        if (external == null) emptyMap()
        else {
            val density = LocalDensity.current
            // Reserve the real label plus AppBadge's padding at the current font and Android scale.
            val badgeSize =
                rememberTextMeasurer()
                    .measure(
                        "APP",
                        style =
                            MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.Bold),
                    )
                    .size
            // Padding rounds each edge separately; leave one pixel per edge for the
            // placeholder's sp-to-pixel rounding as well.
            val badgeWidth = with(density) { (badgeSize.width + 2 * 5.dp.roundToPx() + 2).toSp() }
            val badgeHeight = with(density) { (badgeSize.height + 2 * 2.dp.roundToPx() + 2).toSp() }
            mapOf(
                "compact-app-badge" to
                    InlineTextContent(
                        Placeholder(badgeWidth, badgeHeight, PlaceholderVerticalAlign.TextCenter)
                    ) {
                        AppBadge()
                    }
            )
        }
    return MessageTextPrefix(text, inline)
}

@Composable
internal fun CompactMessageTime(time: String, show: Boolean, reply: Boolean, id: String) {
    val appearance = LocalMessageAppearance.current
    Text(
        if (show) time else "00:00",
        modifier =
            Modifier.widthIn(min = 34.dp)
                .padding(
                    top =
                        if (reply)
                            with(androidx.compose.ui.platform.LocalDensity.current) {
                                appearance.lineHeight.toDp()
                            } + 3.dp
                        else 3.dp
                )
                .testTag("compact-time-$id")
                .then(if (!show) Modifier.clearAndSetSemantics {} else Modifier),
        style = MaterialTheme.typography.labelSmall.copy(fontFeatureSettings = "tnum"),
        color = if (show) MaterialTheme.colorScheme.onSurfaceVariant else Color.Transparent,
    )
}

@Composable
internal fun CompactReplyPreview(message: ChatMessage, original: ChatMessage?, onJump: () -> Unit) {
    val color = MaterialTheme.colorScheme.outlineVariant
    Row(
        Modifier.testTag("reply-preview-${message.id}")
            .drawBehind {
                val left = -12.dp.toPx()
                val middle = size.height / 2
                drawPath(
                    Path().apply {
                        moveTo(left, size.height)
                        lineTo(left, middle + 4.dp.toPx())
                        quadraticBezierTo(left, middle, left + 4.dp.toPx(), middle)
                        lineTo(0f, middle)
                    },
                    color,
                    style = Stroke(1.dp.toPx()),
                )
            }
            .clickable(
                role = Role.Button,
                onClickLabel = ui(R.string.open_original_message),
                onClick = onJump,
            ),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Icon(
            Icons.Outlined.Reply,
            null,
            Modifier.size(12.dp),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            buildAnnotatedString {
                original?.let {
                    withStyle(SpanStyle(fontWeight = FontWeight.Medium)) { append(it.author) }
                    append("  ")
                }
                append(
                    (original?.previewText() ?: message.replyContent).ifBlank {
                        ui(R.string.reply_to_an_earlier_message)
                    }
                )
            },
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            lineHeight = LocalMessageAppearance.current.lineHeight,
        )
    }
}
