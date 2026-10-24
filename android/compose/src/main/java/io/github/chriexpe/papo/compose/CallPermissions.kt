package io.github.chriexpe.papo.compose

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable

/** Activity-owned permission launcher; only an id/result crosses back to the process controller. */
@Composable
internal fun CallPermissions(controller: VoiceCallController, request: CallPermissionRequest?) {
    var launched by rememberSaveable { mutableLongStateOf(-1L) }
    val launcher =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
            controller.permissionResult(launched, allowed)
        }
    LaunchedEffect(request?.serial) {
        if (request != null && request.serial != launched) {
            launched = request.serial
            launcher.launch(request.permission)
        }
    }
}

internal fun callIssueText(code: String): String =
    ui(
        when (code) {
            "leave-timeout" -> R.string.call_leave_timeout
            "audio-routing-unavailable" -> R.string.call_audio_routing_unavailable
            "local-network-permission" -> R.string.call_local_network_error
            "microphone-permission" -> R.string.call_microphone_permission_error
            "camera-permission" -> R.string.call_camera_permission_error
            "camera-background" -> R.string.call_camera_background_error
            "call-offline",
            "connection-lost" -> R.string.call_connection_error
            "call-room-unavailable",
            "voice-room-closed" -> R.string.call_room_error
            "voice-forbidden" -> R.string.call_permission_error
            "voice-room-full" -> R.string.call_full_error
            "audio-unavailable",
            "audio-route-unavailable" -> R.string.call_audio_error
            "device-unavailable" -> R.string.call_device_error
            else -> R.string.call_media_error
        }
    )
