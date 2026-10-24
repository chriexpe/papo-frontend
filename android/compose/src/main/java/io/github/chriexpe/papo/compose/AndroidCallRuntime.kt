package io.github.chriexpe.papo.compose

import android.Manifest
import android.app.Application
import android.content.Intent
import android.content.pm.PackageManager
import android.hardware.Camera
import android.hardware.display.DisplayManager
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.view.Display
import android.view.Surface
import io.github.chriexpe.papo.core.CallHost
import java.lang.ref.WeakReference
import org.freedesktop.gstreamer.GStreamer

/** Android device/lifetime adapter. It owns no signaling or media session state. */
internal object AndroidCallRuntime {
    private lateinit var application: Application
    private var initialized = false
    private var model = WeakReference<PapoViewModel>(null)
    private var focus: AudioFocusRequest? = null
    private var pending: CallRoute? = null
    private var pendingSerial = 0L
    private var lastNotification = ""
    var output = CallAudioOutput.Speaker
        private set

    private val audio
        get() = application.getSystemService(AudioManager::class.java)

    fun initialize(application: Application) {
        this.application = application
        audio.registerAudioDeviceCallback(
            object : AudioDeviceCallback() {
                override fun onAudioDevicesAdded(devices: Array<out AudioDeviceInfo>) {
                    if (focus == null) return
                    val target =
                        devices.mapNotNull(::category).firstOrNull {
                            it == CallAudioOutput.Bluetooth || it == CallAudioOutput.Wired
                        }
                    if (target != null) routeAudio(target)
                }

                override fun onAudioDevicesRemoved(devices: Array<out AudioDeviceInfo>) {
                    if (focus != null && output !in availableOutputs())
                        routeAudio(CallAudioOutput.Speaker)
                }
            },
            Handler(Looper.getMainLooper()),
        )
        audio.addOnCommunicationDeviceChangedListener(application.mainExecutor) { device ->
            if (focus != null && device != null)
                category(device)?.let { selected ->
                    output = selected
                    model.get()?.voiceCalls?.audioOutputChanged(selected)
                }
        }
        initialized =
            runCatching {
                System.loadLibrary("gstreamer_android")
                GStreamer.init(application)
                true
            }
                .getOrElse {
                    android.util.Log.e("PapoCall", "Cannot initialize GStreamer", it)
                    false
                }
    }

    fun attach(model: PapoViewModel) {
        this.model = WeakReference(model)
    }

    fun granted(permission: String) =
        application.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    fun host(wake: () -> Unit) =
        object : CallHost {
            override fun ready() = initialized

            override fun microphoneAllowed() = granted(Manifest.permission.RECORD_AUDIO)

            override fun cameraAllowed() = granted(Manifest.permission.CAMERA)

            override fun cameraDevice(front: Boolean) = cameraDeviceId(front)

            override fun displayRotation(): Int =
                when (
                    application
                        .getSystemService(DisplayManager::class.java)
                        .getDisplay(Display.DEFAULT_DISPLAY)
                        ?.rotation
                ) {
                    Surface.ROTATION_90 -> 90
                    Surface.ROTATION_180 -> 180
                    Surface.ROTATION_270 -> 270
                    else -> 0
                }

            override fun stateChanged() = wake()
        }

    @Suppress("DEPRECATION")
    private fun cameraDeviceId(front: Boolean): String {
        val wanted =
            if (front) Camera.CameraInfo.CAMERA_FACING_FRONT
            else Camera.CameraInfo.CAMERA_FACING_BACK
        return runCatching {
                (0 until Camera.getNumberOfCameras())
                    .firstOrNull { index ->
                        val info = Camera.CameraInfo()
                        Camera.getCameraInfo(index, info)
                        info.facing == wanted
                    }
                    ?.toString() ?: "0"
            }
            .getOrDefault("0")
    }

    @Suppress("DEPRECATION")
    fun canSwitchCamera() = runCatching { Camera.getNumberOfCameras() > 1 }.getOrDefault(false)

    fun prepare(route: CallRoute): Boolean {
        if (!initialized) {
            model.get()?.voiceCalls?.reportError("media-unavailable")
            return false
        }
        val request =
            AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE)
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                        .build()
                )
                .setOnAudioFocusChangeListener { change ->
                    if (
                        change == AudioManager.AUDIOFOCUS_LOSS ||
                            change == AudioManager.AUDIOFOCUS_LOSS_TRANSIENT
                    ) {
                        val controller = model.get()?.voiceCalls
                        controller
                            ?.state
                            ?.value
                            ?.session
                            ?.takeIf { !it.demo && !it.muted }
                            ?.let(controller::toggleMute)
                    }
                }
                .build()
        if (audio.requestAudioFocus(request) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED)
            return false
        focus = request
        audio.mode = AudioManager.MODE_IN_COMMUNICATION
        pendingSerial++
        pending = route
        val outputs = availableOutputs()
        routeAudio(
            when {
                CallAudioOutput.Bluetooth in outputs -> CallAudioOutput.Bluetooth
                CallAudioOutput.Wired in outputs -> CallAudioOutput.Wired
                else -> CallAudioOutput.Speaker
            }
        )
        return runCatching {
            application.startForegroundService(
                Intent(application, VoiceCallService::class.java).setAction(VoiceCallService.START)
            )
            true
        }
            .getOrElse {
                stop()
                false
            }
    }

    fun prepareCamera(): Boolean = VoiceCallService.active?.promoteCamera() == true

    private fun category(device: AudioDeviceInfo): CallAudioOutput? =
        when (device.type) {
            AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> CallAudioOutput.Speaker
            AudioDeviceInfo.TYPE_BUILTIN_EARPIECE -> CallAudioOutput.Earpiece
            AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
            AudioDeviceInfo.TYPE_BLE_HEADSET -> CallAudioOutput.Bluetooth
            AudioDeviceInfo.TYPE_WIRED_HEADSET,
            AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
            AudioDeviceInfo.TYPE_USB_HEADSET,
            AudioDeviceInfo.TYPE_USB_DEVICE -> CallAudioOutput.Wired
            else -> null
        }

    fun availableOutputs(): List<CallAudioOutput> = runCatching {
        audio.availableCommunicationDevices.mapNotNull(::category).distinct()
    }
        .getOrDefault(emptyList())

    fun routeAudio(target: CallAudioOutput): Boolean {
        val device =
            audio.availableCommunicationDevices.firstOrNull { category(it) == target }
                ?: return false
        return audio.setCommunicationDevice(device).also { if (it) output = target }
    }

    fun sync(servers: List<ServerState>, ui: VoiceCallUiState) {
        val session = ui.session?.takeUnless { it.demo }
        if (session != null) {
            pending = null
            val server = session.route.server(servers) ?: return
            val key =
                "${session.route}/${session.sessionId}/${session.muted}/${session.camera}/${session.connected}/${server.call.rooms}"
            if (key != lastNotification) {
                lastNotification = key
                VoiceCallService.active?.publish()
            }
        } else if (
            ui.joining == null &&
                ui.switchingTo == null &&
                pending == null &&
                (focus != null || VoiceCallService.active != null)
        )
            stop()
    }

    fun pendingRoute() = pending

    fun pendingToken() = pendingSerial

    fun failedJoin() {
        pending = null
        stop()
    }

    fun stop() {
        pending = null
        lastNotification = ""
        application.stopService(Intent(application, VoiceCallService::class.java))
        releaseAudio()
    }

    fun releaseAudio() {
        if (focus != null) {
            focus?.let { audio.abandonAudioFocusRequest(it) }
            focus = null
            audio.clearCommunicationDevice()
            audio.mode = AudioManager.MODE_NORMAL
        }
    }
}
