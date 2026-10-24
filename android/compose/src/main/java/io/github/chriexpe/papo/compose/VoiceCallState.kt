package io.github.chriexpe.papo.compose

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** Navigation and session identity are independent: browsing never moves the active call. */
internal data class CallRoute(val serverUrl: String, val ownerId: String, val channelId: String) {
    fun server(servers: List<ServerState>): ServerState? = servers.firstOrNull {
        it.url == serverUrl &&
            it.me == ownerId &&
            it.screen == "chat" &&
            it.channels.any { channel -> channel.id == channelId && channel.kind == "voice" }
    }
}

internal enum class CallPreviewLayout {
    Voice,
    Video,
    Screen,
}

internal enum class CallPreviewOutput {
    Speaker,
    Earpiece,
}

/** Receiver-side settings, independent of the participant's published microphone state. */
internal data class CallParticipantAudio(val volumePercent: Int = 100, val muted: Boolean = false)

internal data class CallPreviewSession(
    val route: CallRoute,
    val sessionId: Long,
    val muted: Boolean = true,
    val camera: Boolean = false,
    val speakerId: String = "ana",
    val layout: CallPreviewLayout = CallPreviewLayout.Voice,
    val output: CallPreviewOutput = CallPreviewOutput.Speaker,
    val videoLimit: Int = 2,
    val videoPreviewsVisible: Boolean = true,
    val participantAudio: Map<String, CallParticipantAudio> = emptyMap(),
)

internal data class VoiceCallUiState(
    val screen: CallRoute? = null,
    val session: CallPreviewSession? = null,
    val switchingTo: CallRoute? = null,
)

/**
 * Explicit, memory-only Demo interaction model. It opens no sockets, requests no permissions and
 * never captures audio/camera. Real call integration follows the UI review; none of these preview
 * transitions are dispatched to papo-core or treated as a connected server session.
 *
 * Owned by the ViewModel so minimizing, changing chats and Activity recreation retain the session.
 * Process death deliberately ends a preview instead of restoring a fictitious connection.
 */
internal class VoiceCallPreviewController(private val scope: CoroutineScope) {
    private val mutableState = MutableStateFlow(VoiceCallUiState())
    val state = mutableState.asStateFlow()
    private var speaking: Job? = null
    private var nextSessionId = 0L

    fun open(server: ServerState, channelId: String) {
        if (!server.isDemo || server.channels.none { it.id == channelId && it.kind == "voice" })
            return
        mutableState.value = state.value.copy(screen = CallRoute(server.url, server.me, channelId))
    }

    fun reopen() {
        state.value.session?.let {
            mutableState.value = state.value.copy(screen = it.route)
        }
    }

    fun minimize() {
        mutableState.value = state.value.copy(screen = null, switchingTo = null)
    }

    fun join(route: CallRoute, servers: List<ServerState>) {
        if (route.server(servers)?.isDemo != true) return
        val active = state.value.session
        if (active != null && active.route != route) {
            mutableState.value = state.value.copy(switchingTo = route)
        } else if (active == null) start(route)
    }

    fun confirmSwitch(servers: List<ServerState>) {
        val route = state.value.switchingTo ?: return
        if (route.server(servers)?.isDemo == true) start(route) else cancelSwitch()
    }

    fun cancelSwitch() {
        mutableState.value = state.value.copy(switchingTo = null)
    }

    private fun start(route: CallRoute) {
        speaking?.cancel()
        val started = CallPreviewSession(route, ++nextSessionId)
        mutableState.value =
            state.value.copy(
                session = started,
                switchingTo = null,
                screen = route,
            )
        // Demo voices alternate without reordering tiles, exercising the same header as the grid.
        speaking = scope.launch {
            while (true) {
                delay(5_000)
                change(started) {
                    it.copy(
                        speakerId =
                            if (route.channelId == "voice-lounge" && it.speakerId == "ana") "bruno"
                            else "ana"
                    )
                }
            }
        }
    }

    /** A session token also distinguishes leaving and rejoining the same room. */
    private fun change(
        expected: CallPreviewSession,
        update: (CallPreviewSession) -> CallPreviewSession,
    ) {
        val session = state.value.session?.takeIf { it.sessionId == expected.sessionId } ?: return
        mutableState.value = state.value.copy(session = update(session))
    }

    fun toggleMute(session: CallPreviewSession) = change(session) { it.copy(muted = !it.muted) }

    /**
     * Session-scoped: leaving/replacing the call discards overrides; stale sheets cannot edit it.
     */
    fun participantVolume(session: CallPreviewSession, id: String, percent: Int) =
        change(session) {
            val audio = it.participantAudio[id] ?: CallParticipantAudio()
            it.copy(
                participantAudio =
                    it.participantAudio +
                        (id to audio.copy(volumePercent = percent.coerceIn(0, 200)))
            )
        }

    fun participantMute(session: CallPreviewSession, id: String, muted: Boolean) =
        change(session) {
            val audio = it.participantAudio[id] ?: CallParticipantAudio()
            it.copy(participantAudio = it.participantAudio + (id to audio.copy(muted = muted)))
        }

    fun toggleCamera(session: CallPreviewSession) =
        change(session) {
            it.copy(
                camera = !it.camera,
                layout = if (!it.camera) CallPreviewLayout.Video else it.layout,
            )
        }

    fun layout(session: CallPreviewSession, layout: CallPreviewLayout) =
        change(session) {
            it.copy(layout = layout, camera = it.camera && layout != CallPreviewLayout.Voice)
        }

    fun output(session: CallPreviewSession, output: CallPreviewOutput) =
        change(session) { it.copy(output = output) }

    fun cycleVideoLimit(session: CallPreviewSession) =
        change(session) { it.copy(videoLimit = it.videoLimit % 4 + 1) }

    /** Closing the video surface never switches off a camera or ends the call. */
    fun hideVideoPreviews(session: CallPreviewSession) =
        change(session) { it.copy(videoPreviewsVisible = false) }

    fun showVideoPreviews(session: CallPreviewSession) =
        change(session) { it.copy(videoPreviewsVisible = true) }

    fun leave(session: CallPreviewSession) {
        if (state.value.session?.sessionId != session.sessionId) return
        speaking?.cancel()
        speaking = null
        mutableState.value = state.value.copy(session = null, switchingTo = null)
    }

    /**
     * Forgetting/signing out invalidates preview ownership; switching the selected server doesn't.
     */
    fun synchronize(servers: List<ServerState>) {
        val current = state.value
        val active = current.session?.takeIf { it.route.server(servers)?.isDemo == true }
        if (active == null) {
            speaking?.cancel()
            speaking = null
        }
        mutableState.value =
            current.copy(
                session = active,
                screen = current.screen?.takeIf { it.server(servers)?.isDemo == true },
                switchingTo = current.switchingTo?.takeIf { it.server(servers)?.isDemo == true },
            )
    }
}

internal data class CallParticipant(
    val id: String,
    val name: String,
    val mine: Boolean,
    val muted: Boolean,
    val speaking: Boolean,
    val camera: Boolean,
    val audio: CallParticipantAudio = CallParticipantAudio(),
)

internal fun previewCallParticipants(
    server: ServerState,
    channelId: String,
    session: CallPreviewSession?,
): List<CallParticipant> {
    val ids =
        (if (channelId == "voice-studio") listOf("ana") else listOf("ana", "bruno", "dora")) +
            if (session != null) listOf(server.me) else emptyList()
    return ids.mapNotNull { id ->
        val person = server.members.firstOrNull { it.id == id } ?: return@mapNotNull null
        val mine = id == server.me
        CallParticipant(
            id,
            person.name,
            mine,
            muted = if (mine) session?.muted != false else id == "dora",
            speaking = session?.speakerId == id,
            camera =
                if (mine) session?.camera == true else session?.layout == CallPreviewLayout.Video,
            audio = session?.participantAudio?.get(id) ?: CallParticipantAudio(),
        )
    }
}
