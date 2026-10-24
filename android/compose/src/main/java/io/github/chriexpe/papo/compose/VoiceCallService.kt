package io.github.chriexpe.papo.compose

import android.Manifest
import android.annotation.SuppressLint
import android.app.*
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.IBinder
import android.os.PowerManager

/**
 * Keeps native workers alive independently of Activity/Compose collection. Never restores a call.
 */
class VoiceCallService : Service() {
    companion object {
        internal const val START = "papo.call.START"
        private const val MUTE = "papo.call.MUTE"
        private const val LEAVE = "papo.call.LEAVE"
        private const val CHANNEL = "calls"
        private const val ID = 125
        internal var active: VoiceCallService? = null
            private set
    }

    private val model
        get() = (application as PapoApplication).model

    private var cameraForeground = false
    private var wakeLock: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    @SuppressLint("WakelockTimeout") // Bounded by the foreground call; released in onDestroy.
    override fun onCreate() {
        super.onCreate()
        active = this
        getSystemService(NotificationManager::class.java)
            .createNotificationChannel(
                NotificationChannel(
                    CHANNEL,
                    ui(R.string.call_notification_channel),
                    NotificationManager.IMPORTANCE_LOW,
                )
            )
        wakeLock =
            getSystemService(PowerManager::class.java)
                .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "$packageName:call")
                .apply { acquire() }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val session = model.voiceCalls.state.value.session?.takeUnless { it.demo }
        if (intent?.action != START) {
            val pending = AndroidCallRuntime.pendingRoute()
            if (
                session == null &&
                    pending != null &&
                    intent?.action == LEAVE &&
                    intent.getBooleanExtra("pending", false) &&
                    intent.getLongExtra("attempt", -1) == AndroidCallRuntime.pendingToken() &&
                    intent.getStringExtra("server") == pending.serverUrl &&
                    intent.getStringExtra("owner") == pending.ownerId
            ) {
                model.voiceCalls.cancelJoin(pending)
                stopSelf()
                return START_NOT_STICKY
            }
            // PendingIntent identity and checks include the join attempt; an old action is inert.
            if (
                session != null &&
                    intent?.getBooleanExtra("pending", false) != true &&
                    intent?.getStringExtra("server") == session.route.serverUrl &&
                    intent.getStringExtra("owner") == session.route.ownerId &&
                    intent.getLongExtra("attempt", -1) == session.sessionId
            ) {
                when (intent.action) {
                    MUTE -> model.voiceCalls.toggleMute(session)
                    LEAVE -> model.voiceCalls.leave(session)
                }
            } else if (session == null && pending == null) {
                stopSelf()
                return START_NOT_STICKY
            }
        }
        if (session == null && AndroidCallRuntime.pendingRoute() == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        publish()
        return START_NOT_STICKY
    }

    internal fun promoteCamera(): Boolean {
        if (!AndroidCallRuntime.granted(Manifest.permission.CAMERA)) return false
        cameraForeground = true
        return runCatching {
            publish()
            true
        }
            .getOrElse {
                cameraForeground = false
                model.voiceCalls.reportError("camera-background")
                false
            }
    }

    internal fun publish() {
        val session = model.voiceCalls.state.value.session?.takeUnless { it.demo }
        val route = session?.route ?: AndroidCallRuntime.pendingRoute() ?: return
        val server = route.server(model.state.value.servers) ?: return
        val channel = server.channels.firstOrNull { it.id == route.channelId } ?: return
        val title = "${channel.name} · ${server.name}"
        val people = callParticipants(server, route.channelId, session)
        val speaker = people.firstOrNull { it.speaking }
        val detail =
            when {
                session?.connected != true -> ui(R.string.call_connecting)
                speaker != null -> ui(R.string.call_person_speaking, speaker.name)
                else -> uiQuantity(R.plurals.call_people_in_room, people.size)
            }
        val openIntent =
            Intent(this, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                .putExtra("openCall", true)
        val open =
            PendingIntent.getActivity(
                this,
                125,
                openIntent,
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        val leave = action(LEAVE, session, route)
        val notification =
            Notification.Builder(this, CHANNEL)
                .setSmallIcon(R.drawable.ic_call_notification)
                .setContentTitle(title)
                .setContentText(detail)
                .setContentIntent(open)
                .setCategory(Notification.CATEGORY_CALL)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setStyle(
                    Notification.CallStyle.forOngoingCall(
                        android.app.Person.Builder().setName(title).build(),
                        leave,
                    )
                )
        if (session != null)
            notification.addAction(
                Notification.Action.Builder(
                        Icon.createWithResource(
                            this,
                            if (session.muted) R.drawable.ic_mic_off_notification
                            else R.drawable.ic_mic_notification,
                        ),
                        ui(if (session.muted) R.string.call_unmute else R.string.call_mute),
                        action(MUTE, session, route),
                    )
                    .build()
            )
        var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
        if (AndroidCallRuntime.granted(Manifest.permission.RECORD_AUDIO))
            types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        if (cameraForeground) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        startForeground(ID, notification.build(), types)
    }

    internal fun control(session: CallSession, leave: Boolean): PendingIntent =
        action(if (leave) LEAVE else MUTE, session, session.route)

    private fun action(name: String, session: CallSession?, route: CallRoute): PendingIntent {
        val token = session?.sessionId ?: AndroidCallRuntime.pendingToken()
        val identity =
            Uri.Builder()
                .scheme("papo-call")
                .authority("control")
                .appendPath(route.serverUrl)
                .appendPath(route.ownerId)
                .appendPath(if (session == null) "pending" else "session")
                .appendPath(token.toString())
                .appendPath(name)
                .build()
        return PendingIntent.getService(
            this,
            0,
            Intent(this, VoiceCallService::class.java)
                .setAction(name)
                .setData(identity)
                .putExtra("server", route.serverUrl)
                .putExtra("owner", route.ownerId)
                .putExtra("attempt", token)
                .putExtra("pending", session == null),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    override fun onDestroy() {
        active = null
        if (wakeLock?.isHeld == true) wakeLock?.release()
        wakeLock = null
        AndroidCallRuntime.releaseAudio()
        val session = model.voiceCalls.state.value.session?.takeUnless { it.demo }
        if (session != null) model.voiceCalls.leave(session)
        else AndroidCallRuntime.pendingRoute()?.let(model.voiceCalls::cancelJoin)
        super.onDestroy()
    }
}
