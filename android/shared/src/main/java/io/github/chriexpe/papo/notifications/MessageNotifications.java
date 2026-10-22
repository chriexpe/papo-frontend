package io.github.chriexpe.papo.notifications;

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
public final class MessageNotifications {
    public static final String CHANNEL_ID = "messages";
    public static final String EXTRA_SERVER = "papo.message.server";
    public static final String EXTRA_CHANNEL = "papo.message.channel";
    public static final String EXTRA_MESSAGE_ID = "papo.message.id";
    public static final String EXTRA_NOTIFICATION_ID = "papo.notification.id";

    public static final String EXTRA_OWNER = "papo.message.owner";

    private MessageNotifications() {}

    public static void ensureChannel(Context context) {
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

    public static boolean available(Context context) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
                && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                        != PackageManager.PERMISSION_GRANTED) {
            return false;
        }
        final NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager == null || !manager.areNotificationsEnabled()) {
            return false;
        }
        final NotificationChannel channel = manager.getNotificationChannel(CHANNEL_ID);
        return channel == null || channel.getImportance() != NotificationManager.IMPORTANCE_NONE;
    }

    public static void show(Context context, String payload, Class<?> activity, int icon) {
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
            final String notificationId = json.optString("notification_id", "");
            final String owner = json.optString("owner_user_id", "");

            final Intent openIntent = new Intent(context, activity)
                    .setData(new android.net.Uri.Builder().scheme("papo").authority(context.getPackageName())
                            .appendPath(serverUrl).appendPath(owner).appendPath(channelId).appendPath(messageId).build())
                    .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP)
                    .putExtra(EXTRA_OWNER, owner)
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
                    .setSmallIcon(icon)
                    .setContentTitle(title)
                    .setContentText(body.isBlank() ? "Nova mensagem" : body)
                    .setStyle(new Notification.BigTextStyle()
                            .bigText(body.isBlank() ? "Nova mensagem" : body))
                    .setCategory(Notification.CATEGORY_MESSAGE)
                    .setVisibility(Notification.VISIBILITY_PRIVATE)
                    .setAutoCancel(true)
                    .setContentIntent(open)
                    .setGroup("papo:" + serverUrl + ":" + channelId)
                    .build();

            final NotificationManager manager =
                    context.getSystemService(NotificationManager.class);
            if (manager != null) {
                manager.notify(
                        "papo-message:" + serverUrl + ":" + owner + ":" + messageId,
                        0,
                        notification);
            }
        } catch (Exception error) {
            Log.e("papo-notify", "falha ao publicar notificação", error);
        }
    }

    public static void clear(Context context, String payload) {
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
                if (group.equals(active.getNotification().getGroup())
                        && (json.optString("owner_user_id", "").isBlank()
                            || active.getTag() != null && active.getTag().startsWith("papo-message:" + json.getString("server_url") + ":" + json.optString("owner_user_id") + ":"))) {
                    manager.cancel(active.getTag(), active.getId());
                }
            }
        } catch (Exception error) {
            Log.e("papo-notify", "falha ao limpar notificações do canal", error);
        }
    }
}
