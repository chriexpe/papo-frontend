package io.github.chriexpe.papo;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Person;
import android.app.Service;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.os.IBinder;
import android.graphics.drawable.Icon;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;

/**
 * Mantém uma call autorizada a usar áudio/câmera quando a Activity sai da
 * frente. O transporte continua inteiro em Rust/GStreamer; este Service só
 * representa a execução para o Android e expõe controles na notificação.
 */
public final class CallService extends Service {
    public static final String ACTION_START = "io.github.chriexpe.papo.call.START";
    public static final String ACTION_UPDATE = "io.github.chriexpe.papo.call.UPDATE";
    public static final String ACTION_TOGGLE_MUTE = "io.github.chriexpe.papo.call.TOGGLE_MUTE";
    public static final String ACTION_TOGGLE_CAMERA = "io.github.chriexpe.papo.call.TOGGLE_CAMERA";
    public static final String ACTION_HANGUP = "io.github.chriexpe.papo.call.HANGUP";

    public static final String EXTRA_TITLE = "title";
    public static final String EXTRA_MUTED = "muted";
    public static final String EXTRA_CAMERA = "camera";
    public static final String EXTRA_MEMBERS = "members";
    public static final String EXTRA_SPEAKER = "speaker";

    private static final String CHANNEL_ID = "calls";
    private static final int NOTIFICATION_ID = 7301;

    private String title = "Papo";
    private boolean muted = true;
    private boolean camera;
    private int members;
    private String speaker = "";
    private AudioManager audioManager;
    private AudioFocusRequest audioFocus;

    private static native void nativeCallAction(String action);

    static {
        // O libpapo depende do GStreamer. Normalmente ambos já estão
        // carregados pela Activity, mas isto também deixa a classe segura
        // contra inicialização fora de ordem no mesmo processo.
        System.loadLibrary("gstreamer_android");
        System.loadLibrary("papo");
    }

    @Override
    public void onCreate() {
        super.onCreate();

        audioManager = getSystemService(AudioManager.class);
        if (audioManager != null) {
            audioFocus = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
                    .setAudioAttributes(new AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                            .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                            .build())
                    .setAcceptsDelayedFocusGain(false)
                    .build();
            audioManager.requestAudioFocus(audioFocus);
            audioManager.setMode(AudioManager.MODE_IN_COMMUNICATION);
        }

        final NotificationManager manager = getSystemService(NotificationManager.class);
        if (manager != null) {
            final NotificationChannel channel = new NotificationChannel(
                    CHANNEL_ID,
                    "Chamadas",
                    NotificationManager.IMPORTANCE_LOW);
            channel.setDescription("Chamadas de voz e vídeo em andamento");
            channel.setSound(null, null);
            manager.createNotificationChannel(channel);
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null) {
            stopSelf();
            return START_NOT_STICKY;
        }

        final String action = intent.getAction();
        if (ACTION_HANGUP.equals(action)) {
            nativeCallAction("hangup");
            stopForeground(STOP_FOREGROUND_REMOVE);
            stopSelf();
            return START_NOT_STICKY;
        }

        if (ACTION_TOGGLE_MUTE.equals(action)) {
            muted = !muted;
            nativeCallAction(muted ? "mute=1" : "mute=0");
            publish();
            return START_NOT_STICKY;
        }

        if (ACTION_TOGGLE_CAMERA.equals(action)) {
            camera = !camera;
            nativeCallAction(camera ? "camera=1" : "camera=0");
            publish();
            return START_NOT_STICKY;
        }

        if (intent.hasExtra(EXTRA_TITLE)) {
            title = intent.getStringExtra(EXTRA_TITLE);
            if (title == null || title.isBlank()) {
                title = "Papo";
            }
        }
        if (intent.hasExtra(EXTRA_MUTED)) {
            muted = intent.getBooleanExtra(EXTRA_MUTED, true);
        }
        if (intent.hasExtra(EXTRA_CAMERA)) {
            camera = intent.getBooleanExtra(EXTRA_CAMERA, false);
        }
        if (intent.hasExtra(EXTRA_MEMBERS)) {
            members = intent.getIntExtra(EXTRA_MEMBERS, 0);
        }
        if (intent.hasExtra(EXTRA_SPEAKER)) {
            final String value = intent.getStringExtra(EXTRA_SPEAKER);
            speaker = value == null ? "" : value;
        }

        publish();
        return START_NOT_STICKY;
    }

    private void publish() {
        int types = ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK;
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO)
                == PackageManager.PERMISSION_GRANTED) {
            types |= ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE;
        }
        if (camera && checkSelfPermission(Manifest.permission.CAMERA)
                == PackageManager.PERMISSION_GRANTED) {
            types |= ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA;
        }

        startForeground(NOTIFICATION_ID, notification(), types);
    }

    private Notification notification() {
        final Intent openIntent = new Intent(this, PapoActivity.class)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        final PendingIntent open = PendingIntent.getActivity(
                this,
                10,
                openIntent,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);

        final PendingIntent mute = PendingIntent.getService(
                this,
                11,
                new Intent(this, CallService.class).setAction(ACTION_TOGGLE_MUTE),
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);

        final PendingIntent cameraAction = PendingIntent.getService(
                this,
                12,
                new Intent(this, CallService.class).setAction(ACTION_TOGGLE_CAMERA),
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);

        final PendingIntent hangup = PendingIntent.getService(
                this,
                13,
                new Intent(this, CallService.class).setAction(ACTION_HANGUP),
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);

        final String detail;
        if (!speaker.isBlank()) {
            detail = speaker + " está falando";
        } else if (members > 0) {
            detail = members + (members == 1 ? " participante" : " participantes");
        } else {
            detail = camera ? "Chamada de vídeo em andamento" : "Chamada de voz em andamento";
        }

        final Person person = new Person.Builder()
                .setName(title)
                .setImportant(true)
                .build();

        final Notification.CallStyle style = Notification.CallStyle.forOngoingCall(person, hangup);

        return new Notification.Builder(this, CHANNEL_ID)
                .setSmallIcon(R.drawable.ic_call_notification)
                .setContentTitle(title)
                .setContentText(detail)
                .setCategory(Notification.CATEGORY_CALL)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setContentIntent(open)
                .setStyle(style)
                .addAction(new Notification.Action.Builder(
                        Icon.createWithResource(
                                this,
                                muted ? R.drawable.ic_mic_off_notification : R.drawable.ic_mic_notification),
                        muted ? "Ativar microfone" : "Silenciar",
                        mute).build())
                .addAction(new Notification.Action.Builder(
                        Icon.createWithResource(
                                this,
                                camera ? R.drawable.ic_camera_off_notification : R.drawable.ic_camera_notification),
                        camera ? "Desligar câmera" : "Ligar câmera",
                        cameraAction).build())
                .build();
    }

    @Override
    public void onDestroy() {
        if (audioManager != null) {
            if (audioFocus != null) {
                audioManager.abandonAudioFocusRequest(audioFocus);
            }
            audioManager.setMode(AudioManager.MODE_NORMAL);
        }
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
