package io.github.chriexpe.papo.compose

import android.app.RemoteAction
import android.content.Context
import android.graphics.drawable.Icon
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle

internal fun callPictureInPictureActions(
    context: Context,
    session: CallSession,
): List<RemoteAction> {
    val service = VoiceCallService.active ?: return emptyList()
    val mute = ui(if (session.muted) R.string.call_unmute else R.string.call_mute)
    val leave = ui(R.string.call_leave)
    return listOf(
        RemoteAction(
            Icon.createWithResource(
                context,
                if (session.muted) R.drawable.ic_mic_off_notification
                else R.drawable.ic_mic_notification,
            ),
            mute,
            mute,
            service.control(session, false),
        ),
        RemoteAction(
            Icon.createWithResource(context, R.drawable.ic_call_notification),
            leave,
            leave,
            service.control(session, true),
        ),
    )
}

/** System PiP renders a single real source; it never carries chat or profile controls offscreen. */
@Composable
internal fun CallPictureInPicture(model: PapoViewModel) {
    val state by model.state.collectAsStateWithLifecycle()
    val ui by model.voiceCalls.state.collectAsStateWithLifecycle()
    val session = ui.session?.takeUnless { it.demo } ?: return
    val server = session.route.server(state.servers) ?: return
    val people = callParticipants(server, session.route.channelId, session)
    var lastSpeaker by remember(session.route, session.sessionId) { mutableStateOf("") }
    val speaking = people.firstOrNull { it.speaking && !it.mine }
    SideEffect { if (speaking != null) lastSpeaker = speaking.id }
    val person =
        people.firstOrNull { it.screen }
            ?: speaking
            ?: people.firstOrNull { it.id == lastSpeaker }
            ?: people.firstOrNull { it.camera && !it.mine }
            ?: people.firstOrNull { it.camera }
            ?: people.firstOrNull()
    val calls = VoiceCallContext(ui, model.voiceCalls, state.servers, model)
    MaterialTheme(colorScheme = darkColorScheme()) {
        CompositionLocalProvider(
            LocalVoiceCall provides calls,
            LocalProfileContext provides ProfileContext(server, model),
        ) {
            Box(
                Modifier.fillMaxSize().background(Color.Black),
                contentAlignment = Alignment.Center,
            ) {
                if (person != null && (person.camera || person.screen)) {
                    CallVideoImage(person.id, person.screen, Modifier.fillMaxSize())
                } else if (person != null) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        UserAvatar(person.id, person.name, Modifier.size(48.dp))
                        Text(
                            person.name,
                            color = Color.White,
                            style = MaterialTheme.typography.labelMedium,
                        )
                    }
                }
            }
        }
    }
}
