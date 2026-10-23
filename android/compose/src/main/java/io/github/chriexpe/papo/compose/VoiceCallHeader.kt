package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.border
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

/** Resolves avatars against the call's account, even while another server's chat is on screen. */
@Composable
internal fun SpeakingCallAvatar(modifier: Modifier = Modifier) {
    val calls = LocalVoiceCall.current ?: return
    val session = calls.state.session ?: return
    val server = calls.server ?: return
    val speaker = server.members.firstOrNull { it.id == session.speakerId }
    CompositionLocalProvider(LocalProfileContext provides ProfileContext(server, calls.model)) {
        IconButton(
            onClick = calls.controller::reopen,
            modifier =
                modifier.testTag("call-speaking-avatar").semantics {
                    contentDescription = ui(R.string.call_open)
                    stateDescription = ui(R.string.call_person_speaking, speaker?.name.orEmpty())
                },
        ) {
            if (speaker != null)
                UserAvatar(
                    speaker.id,
                    speaker.name,
                    Modifier.size(32.dp)
                        .border(2.dp, MaterialTheme.colorScheme.primary, RoundedCornerShape(12.dp))
                        .padding(3.dp),
                    RoundedCornerShape(8.dp),
                )
            else Icon(Icons.AutoMirrored.Outlined.VolumeUp, ui(R.string.call_open))
        }
    }
}

/** A second row of the existing header, not an overlay on top of message content. */
@Composable
internal fun ActiveCallHeaderRow(showAvatar: Boolean = true) {
    val calls = LocalVoiceCall.current ?: return
    val session = calls.state.session ?: return
    val server = calls.server ?: return
    val channel = server.channels.firstOrNull { it.id == session.route.channelId } ?: return
    Row(
        Modifier.fillMaxWidth()
            .testTag("active-call-header")
            .padding(horizontal = 4.dp, vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (showAvatar) SpeakingCallAvatar()
        TextButton(
            onClick = calls.controller::reopen,
            modifier = Modifier.weight(1f).heightIn(min = 48.dp).testTag("call-reopen"),
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

@Composable
internal fun CallBrowsingHeader(modifier: Modifier = Modifier) {
    if (LocalVoiceCall.current?.active != true) return
    GlassPill(modifier.fillMaxWidth(), radius = 24) { ActiveCallHeaderRow() }
}

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
