package io.github.chriexpe.papo;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.service.notification.StatusBarNotification;
import android.util.Log;

import org.json.JSONObject;

/** Context-only message notification renderer shared by Activity and WorkManager. */
final class MessageNotifications {
    static final String CHANNEL_ID = "messages";
    static final String EXTRA_SERVER = "papo.message.server";
    static final String EXTRA_CHANNEL = "papo.message.channel";
    static final String EXTRA_MESSAGE_ID = "papo.message.id";
    static final String EXTRA_NOTIFICATION_ID = "papo.notification.id";

    private MessageNotifications() {}

    static void ensureChannel(Context context) {
        final NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager == null || manager.getNotificationChannel(CHANNEL_ID) != null) {
            return;
        }
        final NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID,
                "Mensagens",
                NotificationManager.IMPORTANCE_DEFAULT);
        channel.setDescription("Mensagens e menções recebidas no Papo");
        manager.createNotificationChannel(channel);
    }

    static boolean available(Context context) {
        return Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU
                || context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                        == PackageManager.PERMISSION_GRANTED;
    }

    static void show(Context context, String payload) {
        try {
            ensureChannel(context);
            if (!available(context)) {
                return;
            }

            final JSONObject json = new JSONObject(payload);
            final String title = json.optString("title", "Papo");
            final String body = json.optString("body", "");
            final String serverUrl = json.getString("server_url");
            final String channelId = json.getString("channel_id");
            final String messageId = json.getString("message_id");
            final String notificationId = json.getString("notification_id");

            final Intent openIntent = new Intent(context, PapoActivity.class)
                    .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP)
                    .putExtra(EXTRA_SERVER, serverUrl)
                    .putExtra(EXTRA_CHANNEL, channelId)
                    .putExtra(EXTRA_MESSAGE_ID, messageId)
                    .putExtra(EXTRA_NOTIFICATION_ID, notificationId);

            final int requestCode =
                    (serverUrl + "\n" + notificationId).hashCode() & 0x7fffffff;
            final PendingIntent open = PendingIntent.getActivity(
                    context,
                    requestCode,
                    openIntent,
                    PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);

            final Notification notification = new Notification.Builder(context, CHANNEL_ID)
                    .setSmallIcon(R.drawable.ic_message_notification)
                    .setContentTitle(title)
                    .setContentText(body.isBlank() ? "Nova mensagem" : body)
                    .setStyle(new Notification.BigTextStyle()
                            .bigText(body.isBlank() ? "Nova mensagem" : body))
                    .setCategory(Notification.CATEGORY_MESSAGE)
                    .setAutoCancel(true)
                    .setContentIntent(open)
                    .setGroup("papo:" + serverUrl + ":" + channelId)
                    .build();

            final NotificationManager manager =
                    context.getSystemService(NotificationManager.class);
            if (manager != null) {
                manager.notify(
                        "papo-message:" + serverUrl,
                        notificationId.hashCode(),
                        notification);
            }
        } catch (Exception error) {
            Log.e("papo-notify", "falha ao publicar notificação", error);
        }
    }

    static void clear(Context context, String payload) {
        try {
            final JSONObject json = new JSONObject(payload);
            final String group = "papo:"
                    + json.getString("server_url")
                    + ":"
                    + json.getString("channel_id");
            final NotificationManager manager =
                    context.getSystemService(NotificationManager.class);
            if (manager == null) {
                return;
            }
            for (StatusBarNotification active : manager.getActiveNotifications()) {
                if (group.equals(active.getNotification().getGroup())) {
                    manager.cancel(active.getTag(), active.getId());
                }
            }
        } catch (Exception error) {
            Log.e("papo-notify", "falha ao limpar notificações do canal", error);
        }
    }
}
