package io.github.chriexpe.papo.compose

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** The Demo roster is room-scoped: a session in Lounge must not mute/add You in Studio. */
@Composable
internal fun DemoVoiceChannelItem(
    server: ServerState,
    channel: Conversation,
    onOpen: (String) -> Unit,
) {
    val session =
        LocalVoiceCall.current?.state?.session?.takeIf {
            it.route.serverUrl == server.url &&
                it.route.ownerId == server.me &&
                it.route.channelId == channel.id
        }
    val people = previewCallParticipants(server, channel.id, session)
    val count =
        if (people.isEmpty()) ui(R.string.call_room_empty)
        else uiQuantity(R.plurals.call_people_in_voice, people.size)
    Column(Modifier.fillMaxWidth().testTag("voice-channel-${channel.id}")) {
        ChannelItem(
            channel,
            onOpen,
            voiceSummary = if (session != null) ui(R.string.call_joined_room, count) else count,
            joined = session != null,
        )
        Column(Modifier.fillMaxWidth().padding(start = 64.dp, end = 20.dp, bottom = 8.dp)) {
            people.forEach { person ->
                key(person.id) { VoiceChannelParticipant(person) }
            }
        }
    }
}

/** Status indicators are descriptive; the channel row remains the explicit route into the call. */
@Composable
private fun VoiceChannelParticipant(person: CallParticipant) {
    val muted = ui(if (person.muted) R.string.call_muted else R.string.call_microphone_on)
    val camera =
        ui(if (person.camera) R.string.call_camera_enabled else R.string.call_camera_disabled)
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = 36.dp)
            .padding(vertical = 4.dp)
            .testTag("voice-participant-${person.id}")
            .semantics(mergeDescendants = true) {
                stateDescription =
                    ui(R.string.call_participant_status, muted, camera) +
                        if (person.speaking) "; " + ui(R.string.call_speaking) else ""
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
