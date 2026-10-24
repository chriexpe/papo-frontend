package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.VolumeOff
import androidx.compose.material.icons.automirrored.outlined.VolumeUp
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

/** Local playback only: never publishes a mute or changes the sender's microphone indicator. */
@Composable
internal fun CallParticipantAudioControls(
    calls: VoiceCallContext,
    session: CallPreviewSession,
    id: String,
) {
    val audio = session.participantAudio[id] ?: CallParticipantAudio()
    Column(Modifier.testTag("call-participant-audio-$id")) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(
                ui(R.string.call_participant_volume),
                Modifier.weight(1f),
                style = MaterialTheme.typography.bodyLarge,
            )
            Text(
                ui(R.string.call_volume_percent, audio.volumePercent),
                style = MaterialTheme.typography.titleMedium,
            )
        }
        Slider(
            value = audio.volumePercent.toFloat(),
            onValueChange = { calls.controller.participantVolume(session, id, it.roundToInt()) },
            valueRange = 0f..200f,
            enabled = !audio.muted,
            modifier =
                Modifier.fillMaxWidth()
                    .heightIn(min = 48.dp)
                    .testTag("call-participant-volume")
                    .semantics {
                        contentDescription = ui(R.string.call_participant_volume)
                        stateDescription = ui(R.string.call_volume_percent, audio.volumePercent)
                    },
        )
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(
                ui(R.string.call_volume_percent, 0),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            TextButton(
                onClick = { calls.controller.participantVolume(session, id, 100) },
                modifier =
                    Modifier.weight(1f)
                        .heightIn(min = 48.dp)
                        .testTag("call-participant-volume-reset"),
                enabled = audio.volumePercent != 100,
            ) {
                Text(ui(R.string.call_volume_reset, 100))
            }
            Text(
                ui(R.string.call_volume_percent, 200),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Row(
            Modifier.fillMaxWidth()
                .heightIn(min = 56.dp)
                .toggleable(
                    value = audio.muted,
                    role = Role.Switch,
                    onValueChange = { calls.controller.participantMute(session, id, it) },
                )
                .testTag("call-participant-mute"),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(
                if (audio.muted) Icons.AutoMirrored.Outlined.VolumeOff
                else Icons.AutoMirrored.Outlined.VolumeUp,
                null,
            )
            Text(
                ui(R.string.call_mute_for_me),
                Modifier.weight(1f),
                style = MaterialTheme.typography.bodyLarge,
            )
            Switch(checked = audio.muted, onCheckedChange = null)
        }
        Text(
            ui(R.string.call_participant_audio_demo),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
