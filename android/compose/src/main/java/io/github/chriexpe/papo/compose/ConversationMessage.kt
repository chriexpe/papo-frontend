@file:OptIn(
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.ExperimentalFoundationApi::class,
)

package io.github.chriexpe.papo.compose

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.text.SimpleDateFormat
import java.util.Date

@Composable
internal fun MessageRow(
    message: ChatMessage,
    server: ServerState,
    model: PapoViewModel,
    grouped: Boolean,
    canReply: Boolean,
    onReply: () -> Unit,
    highlight: Boolean,
    onMention: (Person) -> Unit,
    onJump: (String) -> Unit,
    onActions: () -> Unit,
    onReaction: (Reaction) -> Unit,
    onReactionPeople: (Reaction) -> Unit,
    media: @Composable () -> Unit,
    replyMessage: ChatMessage? = server.messages.firstOrNull { it.id == message.replyTo },
) {
    val appearance = LocalClientPreferences.current
    val messageAppearance = LocalMessageAppearance.current
    val headingHeight =
        maxOf(
            24.dp,
            48.dp -
                with(androidx.compose.ui.platform.LocalDensity.current) {
                    messageAppearance.lineHeight.toDp()
                } -
                messageAppearance.headingGap,
        )
    val openProfile = LocalOpenProfile.current
    val openExternal = LocalOpenExternalProfile.current
    val external = message.externalAuthor
    val person =
        if (external == null) server.members.firstOrNull { it.id == message.authorId } else null
    val authorAction =
        Modifier.combinedClickable(
            onClickLabel = ui(R.string.open_1_s_profile, message.author),
            onLongClickLabel = ui(R.string.mention_named, message.author),
            onClick = {
                if (external != null) openExternal(external) else openProfile(message.authorId)
            },
            onLongClick = {
                if (external != null) openExternal(external) else person?.let(onMention)
            },
        )
    val time =
        remember(message.at, AppLanguage.locale) {
            SimpleDateFormat("HH:mm", AppLanguage.locale).format(Date(message.at))
        }
    val spotlight by
        animateColorAsState(
            if (highlight) MaterialTheme.colorScheme.primary.copy(alpha = .14f)
            else if (message.mentionsMe) MaterialTheme.colorScheme.primary.copy(alpha = .08f)
            else Color.Transparent,
            animationSpec = tween(250),
            label = ui(R.string.original_message_highlight),
        )
    SwipeToReply(message.id, canReply && !message.pending, onReply) {
        Row(
            Modifier.fillMaxWidth()
                .padding(horizontal = 8.dp)
                .testTag("message-${message.id}")
                .background(spotlight, RoundedCornerShape(18.dp))
                .semantics {
                    if (highlight) stateDescription = ui(R.string.original_message)
                    else if (message.mentionsMe)
                        stateDescription = ui(R.string.message_mentions_you)
                    else if (grouped)
                        stateDescription =
                            ui(R.string.continued_message_from_1_s_at_2_s, message.author, time)
                    if (canReply && !message.pending)
                        customActions =
                            listOf(
                                CustomAccessibilityAction(ui(R.string.reply)) {
                                    onReply()
                                    true
                                }
                            )
                }
                .combinedClickable(
                    onClick = {},
                    onLongClick = onActions,
                    onLongClickLabel = ui(R.string.message_actions),
                )
                .padding(
                    start = 12.dp,
                    end = 12.dp,
                    // Put all separation before a message. Padding both sides made the first
                    // continuation inherit the author row's larger bottom padding.
                    top =
                        if (grouped) messageAppearance.continuationGap
                        else messageAppearance.authorGap,
                ),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (appearance.avatars) {
                if (grouped) Spacer(Modifier.width(48.dp))
                else if (external != null)
                    ExternalAvatar(
                        server,
                        external,
                        Modifier.size(48.dp)
                            .then(authorAction)
                            .padding(LocalUiDimensions.current.avatarInset),
                    )
                else
                    UserAvatar(
                        message.authorId,
                        message.author,
                        Modifier.size(48.dp)
                            .then(authorAction)
                            .padding(LocalUiDimensions.current.avatarInset),
                    )
            }
            Column(
                Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(messageAppearance.headingGap),
            ) {
                if (!grouped)
                    MessageAuthorHeading(
                        message,
                        time,
                        external != null,
                        authorAction,
                        headingHeight,
                        appearance.timestamps,
                        onActions,
                    )
                if (message.replyTo.isNotBlank())
                    Row(
                        Modifier.testTag("reply-preview-${message.id}")
                            .clip(RoundedCornerShape(8.dp))
                            .clickable(
                                role = Role.Button,
                                onClickLabel = ui(R.string.open_original_message),
                            ) {
                                onJump(message.replyTo)
                            }
                            .background(MaterialTheme.colorScheme.surfaceContainer)
                            .padding(8.dp),
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Icon(
                            Icons.Outlined.Reply,
                            null,
                            Modifier.size(16.dp),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            remember(replyMessage, message.replyContent) {
                                    replyMessage?.previewText() ?: message.replyContent
                                }
                                .ifBlank { ui(R.string.reply_to_an_earlier_message) },
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                if (message.pinned)
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Icon(
                            Icons.Outlined.PushPin,
                            ui(R.string.pinned_message),
                            Modifier.size(14.dp),
                            tint = MaterialTheme.colorScheme.primary,
                        )
                        Text(
                            ui(R.string.pinned),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.primary,
                        )
                    }
                MessageContent(message, server, model, onActions, media)
                if (grouped && message.edited)
                    Text(
                        if (appearance.timestamps) "${ui(R.string.edited)} · $time"
                        else ui(R.string.edited),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                if (message.reactions.isNotEmpty())
                    MessageReactions(server, model, message.reactions, onReaction, onReactionPeople)
                if (message.pending)
                    Text(
                        when (message.delivery) {
                            "unknown" -> ui(R.string.delivery_unconfirmed_tap_to_retry)
                            "failed" -> ui(R.string.not_sent_tap_to_retry)
                            "sending" -> ui(R.string.sending)
                            else -> ui(R.string.queued_saved_on_this_device)
                        },
                        style = MaterialTheme.typography.labelSmall,
                        color =
                            if (message.delivery in listOf("failed", "unknown"))
                                MaterialTheme.colorScheme.error
                            else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
            }
        }
    }
}

@Composable
private fun MessageAuthorHeading(
    message: ChatMessage,
    time: String,
    external: Boolean,
    authorAction: Modifier,
    headingHeight: androidx.compose.ui.unit.Dp,
    timestamps: Boolean,
    onActions: () -> Unit,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            message.author,
            color =
                if (!external) userNameColor(message.authorId)
                else MaterialTheme.colorScheme.onSurface,
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier =
                Modifier.weight(1f, fill = false)
                    // The heading absorbs any extra avatar height at small text sizes;
                    // the first continuation must have the same gap as later ones.
                    .heightIn(min = headingHeight)
                    .then(authorAction)
                    .wrapContentHeight(Alignment.CenterVertically)
                    .testTag("author-${message.id}"),
        )
        if (external) AppBadge()
        if (timestamps)
            Text(
                time,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        if (message.edited)
            Text(
                ui(R.string.edited),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        CompositionLocalProvider(LocalMinimumInteractiveComponentSize provides 0.dp) {
            IconButton(
                onClick = onActions,
                modifier = Modifier.size(headingHeight),
            ) {
                Icon(
                    Icons.Outlined.MoreHoriz,
                    ui(R.string.message_actions),
                    Modifier.size(18.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun MessageReactions(
    server: ServerState,
    model: PapoViewModel,
    reactions: List<Reaction>,
    onReaction: (Reaction) -> Unit,
    onReactionPeople: (Reaction) -> Unit,
) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        reactions.forEach { reaction ->
            val name =
                if (reaction.custom)
                    server.emojis.firstOrNull { it.id == reaction.emoji }?.name
                        ?: ui(R.string.server_sticker)
                else reaction.emoji
            Surface(
                shape = RoundedCornerShape(16.dp),
                color =
                    if (reaction.mine) MaterialTheme.colorScheme.primaryContainer
                    else if (MaterialTheme.colorScheme.surface.luminance() < .5f) Color(0xFF2E2E32)
                    else Color(0xFFECECEF),
                modifier =
                    Modifier.padding(vertical = 4.dp)
                        .clip(RoundedCornerShape(16.dp))
                        .combinedClickable(
                            onClick = { onReaction(reaction) },
                            onLongClick = { onReactionPeople(reaction) },
                            onLongClickLabel = ui(R.string.see_who_reacted),
                            role = Role.Button,
                        )
                        .semantics {
                            contentDescription = ui(R.string.reaction_named, name)
                            stateDescription =
                                if (reaction.mine)
                                    ui(
                                        R.string.you_reacted_count,
                                        uiQuantity(
                                            R.plurals.reaction_count,
                                            reaction.count.toInt(),
                                        ),
                                    )
                                else
                                    uiQuantity(
                                        R.plurals.reaction_count,
                                        reaction.count.toInt(),
                                    )
                        },
            ) {
                Row(
                    Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    EmojiGlyph(
                        server,
                        EmojiChoice(reaction.emoji, reaction.custom),
                        22,
                        model,
                    )
                    Text(" ${reaction.count}")
                }
            }
        }
    }
}
