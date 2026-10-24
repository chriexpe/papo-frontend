package io.github.chriexpe.papo.compose

import android.animation.ValueAnimator
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.VolumeUp
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

internal data class VoiceCallContext(
    val state: VoiceCallUiState,
    val controller: VoiceCallPreviewController,
    val servers: List<ServerState>,
    val model: PapoViewModel,
) {
    val server
        get() = state.session?.route?.server(servers)

    val active
        get() = state.session != null && server != null
}

internal val LocalVoiceCall = compositionLocalOf<VoiceCallContext?> { null }

/** All call identity, participants and controls stay in their own pill while browsing. */
@Composable
private fun ActiveCallHeaderRow(
    calls: VoiceCallContext,
    session: CallPreviewSession,
    server: ServerState,
) {
    val channel = server.channels.firstOrNull { it.id == session.route.channelId } ?: return
    val people = previewCallParticipants(server, channel.id, session)
    val reopenVideos =
        !session.videoPreviewsVisible &&
            (people.any { it.camera } || session.layout == CallPreviewLayout.Screen)
    Row(
        Modifier.fillMaxWidth()
            .testTag("active-call-header")
            .padding(horizontal = 4.dp, vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        TextButton(
            onClick = calls.controller::reopen,
            modifier =
                Modifier.widthIn(min = 64.dp, max = 112.dp)
                    .heightIn(min = 48.dp)
                    .testTag("call-reopen"),
            contentPadding = PaddingValues(horizontal = 8.dp),
        ) {
            Column(Modifier.fillMaxWidth()) {
                Text(
                    channel.name,
                    style = MaterialTheme.typography.labelLarge,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    ui(R.string.call_preview_server, server.name),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        MinimizedCallParticipants(
            people,
            onOpen = {
                if (reopenVideos) calls.controller.showVideoPreviews(session)
                else calls.controller.reopen()
            },
            showVideoBadge = reopenVideos,
            modifier = Modifier.weight(1f),
        )
        CallIconButton(
            if (session.muted) Icons.Outlined.MicOff else Icons.Outlined.Mic,
            ui(if (session.muted) R.string.call_unmute else R.string.call_mute),
            selected = session.muted,
            tag = "call-header-microphone",
        ) {
            calls.controller.toggleMute(session)
        }
        CallIconButton(
            if (session.camera) Icons.Outlined.Videocam else Icons.Outlined.VideocamOff,
            ui(if (session.camera) R.string.call_camera_off else R.string.call_camera_on),
            selected = session.camera,
            tag = "call-header-camera",
        ) {
            calls.controller.toggleCamera(session)
        }
        CallIconButton(
            Icons.Outlined.CallEnd,
            ui(R.string.call_leave),
            danger = true,
            tag = "call-header-leave",
        ) {
            calls.controller.leave(session)
        }
    }
}

/** Avatar slots use the available width; speaker priority never changes the full call grid. */
@Composable
private fun MinimizedCallParticipants(
    people: List<CallParticipant>,
    onOpen: () -> Unit,
    showVideoBadge: Boolean,
    modifier: Modifier,
) {
    val ordered = people.sortedByDescending { it.speaking }
    BoxWithConstraints(
        modifier
            .height(48.dp)
            .clip(RoundedCornerShape(12.dp))
            .clickable(
                onClickLabel =
                    ui(if (showVideoBadge) R.string.call_show_videos else R.string.call_open),
                onClick = onOpen,
            )
            .testTag("call-header-participants")
            .semantics(mergeDescendants = true) {
                contentDescription = uiQuantity(R.plurals.call_people_in_room, people.size)
                ordered
                    .firstOrNull { it.speaking }
                    ?.let {
                        stateDescription = ui(R.string.call_person_speaking, it.name)
                    }
            },
        contentAlignment = Alignment.CenterStart,
    ) {
        val slots = ((maxWidth + 3.dp) / 33.dp).toInt().coerceAtLeast(0)
        val overflow = slots >= 2 && ordered.size > slots
        val visible = ordered.take(if (overflow) slots - 1 else slots)
        Row(
            horizontalArrangement = Arrangement.spacedBy(3.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            visible.forEach { person ->
                key(person.id) {
                    UserAvatar(
                        person.id,
                        person.name,
                        Modifier.size(30.dp)
                            .then(
                                if (person.speaking)
                                    Modifier.border(
                                            2.dp,
                                            MaterialTheme.colorScheme.primary,
                                            RoundedCornerShape(11.dp),
                                        )
                                        .padding(3.dp)
                                else Modifier.padding(2.dp)
                            ),
                        RoundedCornerShape(8.dp),
                    )
                }
            }
            if (overflow)
                Box(
                    Modifier.size(30.dp)
                        .clip(RoundedCornerShape(10.dp))
                        .background(MaterialTheme.colorScheme.surfaceContainerHighest),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        "+${ordered.size - visible.size}",
                        style = MaterialTheme.typography.labelSmall,
                    )
                }
        }
        if (showVideoBadge)
            Icon(
                Icons.Outlined.PictureInPictureAlt,
                null,
                Modifier.align(Alignment.BottomEnd)
                    .size(18.dp)
                    .clip(RoundedCornerShape(4.dp))
                    .background(MaterialTheme.colorScheme.surfaceContainerHighest)
                    .padding(2.dp),
                tint = MaterialTheme.colorScheme.primary,
            )
    }
}

@Composable
internal fun CallBrowsingHeader(modifier: Modifier = Modifier) {
    val calls = LocalVoiceCall.current ?: return
    val session = calls.state.session ?: return
    val server = calls.server ?: return
    val people = previewCallParticipants(server, session.route.channelId, session)
    CompositionLocalProvider(LocalProfileContext provides ProfileContext(server, calls.model)) {
        GlassPill(modifier.fillMaxWidth().padding(top = 4.dp), radius = 24) {
            Column {
                ActiveCallHeaderRow(calls, session, server)
                MinimizedCallVideos(
                    calls,
                    session,
                    people,
                )
            }
        }
    }
}

/** App and Android motion settings also apply to the glass pill's video expansion transition. */
@Composable
internal fun callMotionEnabled(): Boolean =
    LocalClientPreferences.current.animations && ValueAnimator.areAnimatorsEnabled()

@Composable
internal fun CallIconButton(
    icon: ImageVector,
    label: String,
    selected: Boolean = false,
    danger: Boolean = false,
    tag: String,
    onClick: () -> Unit,
) {
    IconButton(
        onClick = onClick,
        modifier = Modifier.size(48.dp).testTag(tag),
    ) {
        Box(
            Modifier.size(38.dp)
                .clip(RoundedCornerShape(14.dp))
                .background(
                    when {
                        danger -> MaterialTheme.colorScheme.errorContainer
                        selected -> MaterialTheme.colorScheme.secondaryContainer
                        else -> Color.Transparent
                    }
                ),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                icon,
                label,
                tint =
                    when {
                        danger -> MaterialTheme.colorScheme.onErrorContainer
                        selected -> MaterialTheme.colorScheme.onSecondaryContainer
                        else -> MaterialTheme.colorScheme.onSurface
                    },
            )
        }
    }
}

@Composable
internal fun VoiceChannelTabs(chat: Boolean, onChat: () -> Unit, onCall: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp).selectableGroup(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        VoiceChannelTab(
            ui(R.string.call_chat_tab),
            Icons.Outlined.ChatBubbleOutline,
            chat,
            onChat,
            Modifier.weight(1f),
        )
        VoiceChannelTab(
            ui(R.string.call_tab),
            Icons.AutoMirrored.Outlined.VolumeUp,
            !chat,
            onCall,
            Modifier.weight(1f),
        )
    }
}

@Composable
private fun VoiceChannelTab(
    label: String,
    icon: ImageVector,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier,
) {
    // Text buttons retain 48dp hit regions while keeping the visible tab treatment quiet.
    TextButton(
        onClick = onClick,
        modifier = modifier.heightIn(min = 48.dp).semantics { this.selected = selected },
        colors =
            ButtonDefaults.textButtonColors(
                containerColor =
                    if (selected) MaterialTheme.colorScheme.secondaryContainer
                    else Color.Transparent,
                contentColor =
                    if (selected) MaterialTheme.colorScheme.onSecondaryContainer
                    else MaterialTheme.colorScheme.onSurfaceVariant,
            ),
    ) {
        Icon(icon, null, Modifier.size(18.dp))
        Spacer(Modifier.width(8.dp))
        Text(label)
    }
}
