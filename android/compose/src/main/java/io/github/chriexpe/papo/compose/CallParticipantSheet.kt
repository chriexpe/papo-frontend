@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Message
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

internal val LocalOpenCallParticipant = compositionLocalOf<(String) -> Unit> { {} }

/** A call action drawer, separate from profile presentation and bound to its original room/join. */
@Composable
internal fun CallParticipantSheet(
    calls: VoiceCallContext,
    onProfile: (ServerState, String) -> Unit,
    onMessage: (ServerState, String) -> Unit,
) {
    val target = calls.state.participant ?: return
    val server = target.route.server(calls.servers) ?: return
    val channel = server.channels.firstOrNull { it.id == target.route.channelId } ?: return
    val session =
        calls.state.session?.takeIf {
            it.route == target.route && it.sessionId == target.sessionId
        }
    val person =
        callParticipants(server, channel.id, session).firstOrNull { it.id == target.id } ?: return
    val member = server.members.firstOrNull { it.id == person.id }
    val dismiss = calls.controller::dismissParticipant
    CompositionLocalProvider(LocalProfileContext provides ProfileContext(server, calls.model)) {
        ModalBottomSheet(
            onDismissRequest = dismiss,
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        ) {
            Column(
                Modifier.fillMaxWidth()
                    .heightIn(max = LocalConfiguration.current.screenHeightDp.dp * .85f)
                    .verticalScroll(rememberScrollState())
                    .navigationBarsPadding()
                    .padding(horizontal = 24.dp)
                    .padding(bottom = 16.dp)
                    .testTag("call-participant-sheet"),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    UserAvatar(
                        person.id,
                        person.name,
                        Modifier.size(48.dp),
                        RoundedCornerShape(16.dp),
                    )
                    Column(Modifier.weight(1f)) {
                        Text(
                            person.name,
                            style = MaterialTheme.typography.titleLarge,
                            fontWeight = FontWeight.SemiBold,
                            color = userNameColor(person.id),
                        )
                        if (!member?.username.isNullOrBlank())
                            Text(
                                "@${member!!.username}",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        Text(
                            ui(R.string.call_participant_audio_room, channel.name),
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                Text(
                    if (!person.stateKnown) ui(R.string.call_status_unknown)
                    else
                        ui(
                            R.string.call_participant_status,
                            ui(
                                if (person.muted) R.string.call_muted
                                else R.string.call_microphone_on
                            ),
                            ui(
                                if (person.camera) R.string.call_camera_enabled
                                else R.string.call_camera_disabled
                            ),
                        ),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                HorizontalDivider()
                when {
                    session == null ->
                        Text(
                            ui(R.string.call_join_for_participant_audio),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    !person.mine -> CallParticipantAudioControls(calls, session, person.id)
                    else -> {
                        SheetAction(
                            ui(if (session.muted) R.string.call_unmute else R.string.call_mute),
                            if (session.muted) Icons.Outlined.MicOff else Icons.Outlined.Mic,
                        ) {
                            calls.controller.toggleMute(session)
                        }
                        SheetAction(
                            ui(
                                if (session.camera) R.string.call_camera_off
                                else R.string.call_camera_on
                            ),
                            if (session.camera) Icons.Outlined.Videocam
                            else Icons.Outlined.VideocamOff,
                        ) {
                            calls.controller.toggleCamera(session)
                        }
                    }
                }
                if (
                    person.mine &&
                        session?.camera == true &&
                        (session.demo || AndroidCallRuntime.canSwitchCamera())
                )
                    SheetAction(ui(R.string.call_switch_camera), Icons.Outlined.Cameraswitch) {
                        calls.controller.switchCamera(session)
                    }
                HorizontalDivider()
                SheetAction(ui(R.string.call_view_profile), Icons.Outlined.Person) {
                    dismiss()
                    onProfile(server, person.id)
                }
                if (!person.mine)
                    SheetAction(
                        ui(R.string.message_person, person.name),
                        Icons.AutoMirrored.Outlined.Message,
                    ) {
                        dismiss()
                        onMessage(server, person.id)
                    }
            }
        }
    }
}
