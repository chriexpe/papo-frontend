@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.VolumeUp
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp

@Composable
internal fun CallDock(
    session: CallSession,
    controller: VoiceCallController,
    onAudio: () -> Unit,
) {
    GlassPill(
        Modifier.padding(horizontal = 12.dp, vertical = 8.dp).widthIn(max = 520.dp).fillMaxWidth(),
        radius = 28,
    ) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 8.dp)) {
            CallDockAction(
                if (session.muted) Icons.Outlined.MicOff else Icons.Outlined.Mic,
                ui(if (session.muted) R.string.call_unmute else R.string.call_mute),
                session.muted,
                false,
                "call-microphone",
                Modifier.weight(1f),
            ) {
                controller.toggleMute(session)
            }
            CallDockAction(
                if (session.camera) Icons.Outlined.Videocam else Icons.Outlined.VideocamOff,
                ui(R.string.camera),
                session.camera,
                false,
                "call-camera",
                Modifier.weight(1f),
                description =
                    ui(if (session.camera) R.string.call_camera_off else R.string.call_camera_on),
            ) {
                controller.toggleCamera(session)
            }
            CallDockAction(
                if (session.output == CallAudioOutput.Speaker) Icons.AutoMirrored.Outlined.VolumeUp
                else Icons.Outlined.Hearing,
                ui(R.string.call_audio),
                false,
                false,
                "call-audio",
                Modifier.weight(1f),
                onClick = onAudio,
            )
            CallDockAction(
                Icons.Outlined.CallEnd,
                ui(R.string.call_leave),
                false,
                true,
                "call-leave",
                Modifier.weight(1f),
            ) {
                controller.leave(session)
            }
        }
    }
}

@Composable
private fun CallDockAction(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    label: String,
    selected: Boolean,
    danger: Boolean,
    tag: String,
    modifier: Modifier,
    description: String = label,
    onClick: () -> Unit,
) {
    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        CallIconButton(icon, description, selected, danger, tag, onClick)
        Text(
            label,
            Modifier.padding(horizontal = 2.dp),
            style = MaterialTheme.typography.labelSmall,
            textAlign = TextAlign.Center,
        )
    }
}

@Composable
internal fun CallOptionsSheet(
    kind: String,
    session: CallSession?,
    controller: VoiceCallController,
    onDismiss: () -> Unit,
) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier.fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 24.dp)
                .padding(bottom = 24.dp)
                .selectableGroup()
        ) {
            Text(
                ui(
                    if (kind == "audio") R.string.call_audio
                    else if (session?.demo == false) R.string.call_options
                    else R.string.call_preview_layout
                ),
                style = MaterialTheme.typography.titleLarge,
            )
            if (session?.demo != false)
                Text(
                    ui(R.string.call_demo_description),
                    Modifier.padding(vertical = 12.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            if (session == null) {
                Text(ui(R.string.call_join_to_preview_layout), Modifier.padding(vertical = 16.dp))
            } else if (kind == "audio") {
                (if (session.demo) listOf(CallAudioOutput.Speaker, CallAudioOutput.Earpiece)
                    else AndroidCallRuntime.availableOutputs())
                    .forEach { output ->
                        CallOption(
                            ui(
                                when (output) {
                                    CallAudioOutput.Speaker -> R.string.call_speaker
                                    CallAudioOutput.Earpiece -> R.string.call_earpiece
                                    CallAudioOutput.Bluetooth -> R.string.call_bluetooth
                                    CallAudioOutput.Wired -> R.string.call_wired
                                }
                            ),
                            session.output == output,
                        ) {
                            controller.output(session, output)
                            onDismiss()
                        }
                    }
            } else if (!session.demo) {
                if (session.camera && AndroidCallRuntime.canSwitchCamera())
                    SheetAction(ui(R.string.call_switch_camera), Icons.Outlined.Cameraswitch) {
                        controller.switchCamera(session)
                        onDismiss()
                    }
            } else {
                CallLayout.entries.forEach { layout ->
                    val label =
                        when (layout) {
                            CallLayout.Voice -> R.string.call_voice_only
                            CallLayout.Video -> R.string.call_video
                            CallLayout.Screen -> R.string.call_screen_sharing
                        }
                    CallOption(ui(label), session.layout == layout) {
                        controller.layout(session, layout)
                        onDismiss()
                    }
                }
            }
        }
    }
}

@Composable
private fun CallOption(label: String, selected: Boolean, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = 56.dp)
            .selectable(selected = selected, role = Role.RadioButton, onClick = onClick),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RadioButton(selected, onClick = null)
        Spacer(Modifier.width(12.dp))
        Text(label)
    }
}

@Composable
internal fun CallSwitchConfirmation(calls: VoiceCallContext) {
    val target = calls.state.switchingTo ?: return
    val old = calls.state.session ?: return
    val targetServer = target.server(calls.servers) ?: return
    val oldServer = old.route.server(calls.servers) ?: return
    val currentName =
        oldServer.channels.firstOrNull { it.id == old.route.channelId }?.name.orEmpty()
    val nextName = targetServer.channels.firstOrNull { it.id == target.channelId }?.name.orEmpty()
    AlertDialog(
        onDismissRequest = calls.controller::cancelSwitch,
        title = { Text(ui(R.string.call_switch_title)) },
        text = { Text(ui(R.string.call_switch_description, currentName, nextName)) },
        confirmButton = {
            TextButton(onClick = { calls.controller.confirmSwitch(calls.servers) }) {
                Text(ui(R.string.call_switch_confirm))
            }
        },
        dismissButton = {
            TextButton(onClick = calls.controller::cancelSwitch) { Text(ui(R.string.cancel)) }
        },
    )
}
