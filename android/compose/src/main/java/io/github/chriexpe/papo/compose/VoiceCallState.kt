package io.github.chriexpe.papo.compose

import android.Manifest
import io.github.chriexpe.papo.core.Action
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** Navigation and media ownership are independent: browsing never moves the active call. */
internal data class CallRoute(val serverUrl: String, val ownerId: String, val channelId: String) {
    fun server(servers: List<ServerState>): ServerState? = servers.firstOrNull {
        it.url == serverUrl &&
            it.me == ownerId &&
            it.screen == "chat" &&
            it.channels.any { channel -> channel.id == channelId && channel.kind == "voice" }
    }
}

internal enum class CallLayout {
    Voice,
    Video,
    Screen,
}

internal enum class CallAudioOutput {
    Speaker,
    Earpiece,
    Bluetooth,
    Wired,
}

internal data class CallParticipantAudio(val volumePercent: Int = 100, val muted: Boolean = false)

internal data class CallSession(
    val route: CallRoute,
    val sessionId: Long,
    val muted: Boolean = true,
    val camera: Boolean = false,
    val speakerId: String = "",
    val layout: CallLayout = CallLayout.Voice,
    val output: CallAudioOutput = CallAudioOutput.Speaker,
    val videoLimit: Int = 2,
    val videoPreviewsVisible: Boolean = true,
    val participantAudio: Map<String, CallParticipantAudio> = emptyMap(),
    val demo: Boolean = true,
    val connected: Boolean = true,
    val frontCamera: Boolean = true,
    val playbackAvailable: Boolean = true,
)

internal data class CallPermissionRequest(val serial: Long, val permission: String)

internal data class VoiceCallUiState(
    val screen: CallRoute? = null,
    val session: CallSession? = null,
    val switchingTo: CallRoute? = null,
    val participant: CallParticipantTarget? = null,
    val joining: CallRoute? = null,
    val permission: CallPermissionRequest? = null,
    val error: String = "",
)

internal data class CallParticipantTarget(
    val route: CallRoute,
    val id: String,
    val sessionId: Long?,
)

/**
 * Owns presentation only. Live session identity, membership, speaking and device results come from
 * Rust snapshots. Demo's timer/fixtures never substitute for live transport. Call actions carry
 * both owner/room and the core join attempt, so old UI and permission results cannot affect a
 * rejoin.
 */
internal class VoiceCallController(
    private val scope: CoroutineScope,
    private val request: (CallRoute, Action, () -> Unit, () -> Unit) -> Unit,
    private val granted: (String) -> Boolean,
    private val prepare: (CallRoute) -> Boolean,
    private val routeAudio: (CallAudioOutput) -> Boolean,
) {
    private val mutableState = MutableStateFlow(VoiceCallUiState())
    val state = mutableState.asStateFlow()
    private var servers = emptyList<ServerState>()
    private var speaking: Job? = null
    private var nextSessionId = 0L
    private var permissionSerial = 0L
    private var permissionAction: (() -> Unit)? = null
    private var permissionRoute: CallRoute? = null
    private var lastIssue = ""

    fun open(server: ServerState, channelId: String) {
        if (server.channels.none { it.id == channelId && it.kind == "voice" }) return
        mutableState.value =
            state.value.copy(
                screen = CallRoute(server.url, server.me, channelId),
                participant = null,
            )
    }

    fun openParticipant(server: ServerState, channelId: String, id: String) {
        val route = CallRoute(server.url, server.me, channelId)
        val session = state.value.session?.takeIf { it.route == route }
        if (callParticipants(server, channelId, session).none { it.id == id }) return
        mutableState.value =
            state.value.copy(participant = CallParticipantTarget(route, id, session?.sessionId))
    }

    fun dismissParticipant() {
        mutableState.value = state.value.copy(participant = null)
    }

    fun reopen() {
        state.value.session?.let { mutableState.value = state.value.copy(screen = it.route) }
    }

    fun minimize() {
        mutableState.value = state.value.copy(screen = null, switchingTo = null, participant = null)
    }

    fun clearError() {
        mutableState.value = state.value.copy(error = "")
    }

    fun reportError(code: String) {
        mutableState.value = state.value.copy(error = code)
    }

    fun join(route: CallRoute, servers: List<ServerState>) {
        this.servers = servers
        val server = route.server(servers) ?: return
        val active = state.value.session
        if (active != null && active.route != route) {
            mutableState.value = state.value.copy(switchingTo = route)
            return
        }
        if (active == null) {
            if (server.isDemo) startDemo(route)
            else permission(route, Manifest.permission.RECORD_AUDIO) { startLive(route) }
        }
    }

    fun confirmSwitch(servers: List<ServerState>) {
        this.servers = servers
        val route = state.value.switchingTo ?: return
        if (route.server(servers) == null) {
            cancelSwitch()
            return
        }
        val old = state.value.session
        val begin = {
            mutableState.value = state.value.copy(session = null, switchingTo = null)
            join(route, this.servers)
        }
        if (old == null || old.demo) {
            speaking?.cancel()
            begin()
        } else request(old.route, Action.LeaveVoice(old.sessionId.toULong()), begin, ::cancelSwitch)
    }

    fun cancelSwitch() {
        mutableState.value = state.value.copy(switchingTo = null)
    }

    private fun startLive(route: CallRoute) {
        if (route.server(servers) == null) return
        if (!prepare(route)) {
            reportError("audio-unavailable")
            return
        }
        mutableState.value = state.value.copy(joining = route, screen = route, error = "")
        request(
            route,
            Action.JoinVoice(route.channelId),
            {},
            {
                mutableState.value = state.value.copy(joining = null)
            },
        )
    }

    private fun startDemo(route: CallRoute) {
        speaking?.cancel()
        val started = CallSession(route, ++nextSessionId, speakerId = "ana")
        mutableState.value = state.value.copy(session = started, switchingTo = null, screen = route)
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

    private fun permission(route: CallRoute, name: String, action: () -> Unit) {
        if (granted(name)) {
            action()
            return
        }
        permissionAction = action
        permissionRoute = route
        mutableState.value =
            state.value.copy(permission = CallPermissionRequest(++permissionSerial, name))
    }

    fun permissionResult(serial: Long, allowed: Boolean) {
        val pending = state.value.permission?.takeIf { it.serial == serial } ?: return
        val action = permissionAction
        val valid = permissionRoute?.server(servers) != null
        permissionAction = null
        permissionRoute = null
        mutableState.value = state.value.copy(permission = null)
        if (allowed && valid) action?.invoke()
        else if (valid)
            reportError(
                if (pending.permission == Manifest.permission.CAMERA) "camera-permission"
                else "microphone-permission"
            )
    }

    private fun current(expected: CallSession): CallSession? =
        state.value.session?.takeIf {
            it.route == expected.route &&
                it.sessionId == expected.sessionId &&
                it.demo == expected.demo
        }

    private fun change(expected: CallSession, update: (CallSession) -> CallSession) {
        val session = current(expected) ?: return
        mutableState.value = state.value.copy(session = update(session))
    }

    private fun command(expected: CallSession, action: Action) {
        if (current(expected) != null) request(expected.route, action, {}, {})
    }

    fun toggleMute(session: CallSession) {
        if (session.demo) change(session) { it.copy(muted = !it.muted) }
        else if (session.muted)
            permission(session.route, Manifest.permission.RECORD_AUDIO) {
                command(session, Action.MuteVoice(session.sessionId.toULong(), false))
            }
        else command(session, Action.MuteVoice(session.sessionId.toULong(), true))
    }

    fun toggleCamera(session: CallSession) {
        if (session.demo)
            change(session) {
                it.copy(
                    camera = !it.camera,
                    layout = if (!it.camera) CallLayout.Video else it.layout,
                )
            }
        else if (!session.camera)
            permission(session.route, Manifest.permission.CAMERA) {
                if (current(session) != null && AndroidCallRuntime.prepareCamera())
                    command(session, Action.CameraVoice(session.sessionId.toULong(), true))
            }
        else command(session, Action.CameraVoice(session.sessionId.toULong(), false))
    }

    fun switchCamera(session: CallSession) {
        if (session.demo) change(session) { it.copy(frontCamera = !it.frontCamera) }
        else
            command(session, Action.CameraFacing(session.sessionId.toULong(), !session.frontCamera))
    }

    fun participantVolume(session: CallSession, id: String, percent: Int) {
        val audio = current(session)?.participantAudio?.get(id) ?: CallParticipantAudio()
        playback(session, id, audio.copy(volumePercent = percent.coerceIn(0, 200)))
    }

    fun participantMute(session: CallSession, id: String, muted: Boolean) {
        val audio = current(session)?.participantAudio?.get(id) ?: CallParticipantAudio()
        playback(session, id, audio.copy(muted = muted))
    }

    private fun playback(session: CallSession, id: String, audio: CallParticipantAudio) {
        if (session.demo)
            change(session) { it.copy(participantAudio = it.participantAudio + (id to audio)) }
        else
            command(
                session,
                Action.VoicePlayback(
                    session.sessionId.toULong(),
                    id,
                    audio.volumePercent.toUShort(),
                    audio.muted,
                ),
            )
    }

    fun layout(session: CallSession, layout: CallLayout) {
        if (session.demo)
            change(session) {
                it.copy(layout = layout, camera = it.camera && layout != CallLayout.Voice)
            }
    }

    fun output(session: CallSession, output: CallAudioOutput) {
        if (session.demo || routeAudio(output)) change(session) { it.copy(output = output) }
        else reportError("audio-route-unavailable")
    }

    fun audioOutputChanged(output: CallAudioOutput) {
        val session = state.value.session?.takeUnless { it.demo } ?: return
        change(session) { it.copy(output = output) }
    }

    fun cycleVideoLimit(session: CallSession) =
        change(session) { it.copy(videoLimit = it.videoLimit % 4 + 1) }

    fun hideVideoPreviews(session: CallSession) =
        change(session) { it.copy(videoPreviewsVisible = false) }

    fun showVideoPreviews(session: CallSession) =
        change(session) { it.copy(videoPreviewsVisible = true) }

    fun cancelJoin(route: CallRoute) {
        if (state.value.joining != route && AndroidCallRuntime.pendingRoute() != route) return
        val server = route.server(servers)
        mutableState.value = state.value.copy(joining = null)
        if (server != null) request(route, Action.LeaveVoice(server.call.attempt), {}, {})
        AndroidCallRuntime.failedJoin()
    }

    fun leave(session: CallSession) {
        if (current(session) == null) return
        if (!session.demo) {
            command(session, Action.LeaveVoice(session.sessionId.toULong()))
            return
        }
        speaking?.cancel()
        speaking = null
        mutableState.value =
            state.value.copy(session = null, switchingTo = null, participant = null)
    }

    /** Called by the process-owned actor, even with no Activity collecting UI state. */
    fun synchronize(servers: List<ServerState>) {
        this.servers = servers
        val before = state.value
        var session = before.session?.takeIf { it.route.server(servers) != null }
        val live = servers.firstOrNull {
            !it.isDemo &&
                it.screen == "chat" &&
                it.call.phase in listOf("joining", "in") &&
                it.call.channelId.isNotBlank()
        }
        if (live != null) {
            speaking?.cancel()
            speaking = null
            val route = CallRoute(live.url, live.me, live.call.channelId)
            val token = live.call.attempt.toLong()
            val old = session?.takeIf { !it.demo && it.route == route && it.sessionId == token }
            val members =
                live.call.rooms.firstOrNull { it.channelId == route.channelId }?.members.orEmpty()
            session =
                (old ?: CallSession(route, token, demo = false)).copy(
                    muted = live.call.muted,
                    camera = live.call.camera,
                    connected = live.call.connected,
                    output = old?.output ?: AndroidCallRuntime.output,
                    frontCamera = live.call.frontCamera,
                    playbackAvailable = live.call.playbackAvailable,
                    speakerId = members.firstOrNull { it.speaking }?.userId.orEmpty(),
                    layout =
                        when {
                            members.any { it.screen } -> CallLayout.Screen
                            live.call.camera || members.any { it.camera } -> CallLayout.Video
                            else -> CallLayout.Voice
                        },
                    participantAudio =
                        live.call.playback.associate {
                            it.userId to CallParticipantAudio(it.volumePercent.toInt(), it.muted)
                        },
                )
        } else if (session?.demo == false) session = null
        if (session == null) {
            speaking?.cancel()
            speaking = null
        }
        val issue = servers.firstOrNull {
            it.call.error.isNotEmpty() || it.call.warning.isNotEmpty()
        }
        val marker =
            issue
                ?.let {
                    "${it.url}/${it.me}/${it.call.attempt}/${it.call.error}/${it.call.warning}"
                }
                .orEmpty()
        val error =
            if (marker.isNotEmpty() && marker != lastIssue)
                issue!!.call.error.ifEmpty { issue.call.warning }
            else before.error
        lastIssue = marker
        val permissionValid = permissionRoute == null || permissionRoute?.server(servers) != null
        if (!permissionValid) {
            permissionAction = null
            permissionRoute = null
        }
        mutableState.value =
            before.copy(
                session = session,
                joining =
                    before.joining?.takeIf {
                        it.server(servers) != null && session?.route != it && issue == null
                    },
                screen = before.screen?.takeIf { it.server(servers) != null },
                switchingTo = before.switchingTo?.takeIf { it.server(servers) != null },
                participant =
                    before.participant?.takeIf {
                        it.route.server(servers) != null &&
                            (it.sessionId == null ||
                                (session?.route == it.route && session.sessionId == it.sessionId))
                    },
                permission = before.permission?.takeIf { permissionValid },
                error = error,
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
    val screen: Boolean = false,
    val stateKnown: Boolean = true,
)

internal fun callParticipants(
    server: ServerState,
    channelId: String,
    session: CallSession?,
): List<CallParticipant> {
    if (!server.isDemo) {
        val members = server.call.rooms.firstOrNull { it.channelId == channelId }?.members.orEmpty()
        val people = members.map { member ->
            val user = server.members.firstOrNull { it.id == member.userId }
            CallParticipant(
                member.userId,
                user?.name ?: member.userId,
                member.userId == server.me,
                if (member.userId == server.me && session != null) session.muted else member.muted,
                member.speaking,
                if (member.userId == server.me && session != null) session.camera
                else member.camera,
                session?.participantAudio?.get(member.userId) ?: CallParticipantAudio(),
                member.screen,
                member.stateKnown || (member.userId == server.me && session != null),
            )
        }
        return if (session != null && people.none { it.mine })
            people +
                CallParticipant(
                    server.me,
                    server.myName,
                    true,
                    session.muted,
                    false,
                    session.camera,
                )
        else people
    }
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
            if (mine) session?.muted != false else id == "dora",
            session?.speakerId == id,
            if (mine) session?.camera == true else session?.layout == CallLayout.Video,
            session?.participantAudio?.get(id) ?: CallParticipantAudio(),
            !mine && session?.layout == CallLayout.Screen && id == "ana",
        )
    }
}
