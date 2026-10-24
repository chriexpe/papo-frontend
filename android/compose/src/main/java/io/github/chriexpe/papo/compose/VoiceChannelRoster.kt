package io.github.chriexpe.papo.compose

import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.VolumeOff
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** Membership belongs to its room; browsing another room never changes the active call. */
@Composable
internal fun VoiceChannelItem(
    server: ServerState,
    channel: Conversation,
    onOpen: (String) -> Unit,
) {
    val calls = LocalVoiceCall.current
    val session =
        calls?.state?.session?.takeIf {
            it.route.serverUrl == server.url &&
                it.route.ownerId == server.me &&
                it.route.channelId == channel.id
        }
    val people = callParticipants(server, channel.id, session)
    val count =
        if (people.isEmpty()) ui(R.string.call_room_empty)
        else uiQuantity(R.plurals.call_people_in_voice, people.size)
    Column(Modifier.fillMaxWidth().testTag("voice-channel-${channel.id}")) {
        ChannelItem(
            channel,
            onOpen,
            voiceSummary =
                if (session != null && !session.connected) ui(R.string.call_connecting)
                else if (session != null) ui(R.string.call_joined_room, count) else count,
            joined = session != null,
        )
        Column(Modifier.fillMaxWidth().padding(start = 64.dp, end = 20.dp, bottom = 8.dp)) {
            people.forEach { person ->
                key(person.id) {
                    VoiceChannelParticipant(person) {
                        calls?.controller?.openParticipant(server, channel.id, person.id)
                    }
                }
            }
        }
    }
}

/** Participant actions open independently of profile navigation and never join the room. */
@Composable
private fun VoiceChannelParticipant(person: CallParticipant, onParticipant: () -> Unit) {
    val muted = ui(if (person.muted) R.string.call_muted else R.string.call_microphone_on)
    val camera =
        ui(if (person.camera) R.string.call_camera_enabled else R.string.call_camera_disabled)
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = 48.dp)
            .clip(RoundedCornerShape(12.dp))
            .clickable(
                onClickLabel = ui(R.string.call_open_participant, person.name),
                onClick = onParticipant,
            )
            .padding(vertical = 4.dp)
            .testTag("voice-participant-${person.id}")
            .semantics(mergeDescendants = true) {
                stateDescription =
                    listOfNotNull(
                            if (person.stateKnown)
                                ui(R.string.call_participant_status, muted, camera)
                            else ui(R.string.call_status_unknown),
                            if (person.audio.muted || person.audio.volumePercent == 0)
                                ui(R.string.call_muted_for_me)
                            else null,
                            if (person.speaking) ui(R.string.call_speaking) else null,
                        )
                        .joinToString("; ")
            },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        val shape = RoundedCornerShape(10.dp)
        UserAvatar(
            person.id,
            person.name,
            Modifier.size(28.dp)
                .then(
                    if (person.speaking)
                        Modifier.border(1.5.dp, MaterialTheme.colorScheme.primary, shape)
                            .padding(3.dp)
                    else Modifier
                ),
            shape,
        )
        Text(
            if (person.mine) ui(R.string.you) else person.name,
            Modifier.weight(1f),
            style = MaterialTheme.typography.labelLarge,
            color = userNameColor(person.id),
            fontWeight = if (person.speaking) FontWeight.SemiBold else FontWeight.Normal,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (person.audio.muted || person.audio.volumePercent == 0)
            Icon(
                Icons.AutoMirrored.Outlined.VolumeOff,
                ui(R.string.call_muted_for_me),
                Modifier.size(16.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        else if (!person.mine && person.audio.volumePercent != 100)
            Text(
                ui(R.string.call_volume_percent, person.audio.volumePercent),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        if (person.stateKnown) {
            Icon(
                if (person.muted) Icons.Outlined.MicOff
                else if (person.speaking) Icons.Outlined.GraphicEq else Icons.Outlined.Mic,
                muted,
                Modifier.size(16.dp),
                tint =
                    if (person.speaking) MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Icon(
                if (person.camera) Icons.Outlined.Videocam else Icons.Outlined.VideocamOff,
                camera,
                Modifier.size(16.dp),
                tint =
                    if (person.camera) MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
