package io.github.chriexpe.papo;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.os.IBinder;
import android.graphics.drawable.Icon;

/**
 * Mantém uma call autorizada a usar áudio/câmera quando a Activity sai da
 * frente. O transporte continua inteiro em Rust/GStreamer; este Service só
 * representa a execução para o Android e expõe controles na notificação.
 */
public final class CallService extends Service {
    public static final String ACTION_START = "io.github.chriexpe.papo.call.START";
    public static final String ACTION_UPDATE = "io.github.chriexpe.papo.call.UPDATE";
    public static final String ACTION_TOGGLE_MUTE = "io.github.chriexpe.papo.call.TOGGLE_MUTE";
    public static final String ACTION_HANGUP = "io.github.chriexpe.papo.call.HANGUP";

    public static final String EXTRA_TITLE = "title";
    public static final String EXTRA_MUTED = "muted";
    public static final String EXTRA_CAMERA = "camera";

    private static final String CHANNEL_ID = "calls";
    private static final int NOTIFICATION_ID = 7301;

    private String title = "Papo";
    private boolean muted = true;
    private boolean camera;

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

        final PendingIntent hangup = PendingIntent.getService(
                this,
                12,
                new Intent(this, CallService.class).setAction(ACTION_HANGUP),
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);

        return new Notification.Builder(this, CHANNEL_ID)
                .setSmallIcon(R.drawable.ic_call_notification)
                .setContentTitle(title)
                .setContentText(camera ? "Chamada de vídeo em andamento" : "Chamada de voz em andamento")
                .setCategory(Notification.CATEGORY_CALL)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setContentIntent(open)
                .addAction(new Notification.Action.Builder(
                        Icon.createWithResource(this, R.drawable.ic_call_notification),
                        muted ? "Ativar microfone" : "Silenciar",
                        mute).build())
                .addAction(new Notification.Action.Builder(
                        Icon.createWithResource(this, R.drawable.ic_call_notification),
                        "Desligar",
                        hangup).build())
                .build();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
