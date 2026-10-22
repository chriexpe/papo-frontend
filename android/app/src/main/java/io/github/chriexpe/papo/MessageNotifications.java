package io.github.chriexpe.papo;

import android.content.Context;

/** egui entry points use the same renderer as the Compose client. */
final class MessageNotifications {
    static final String CHANNEL_ID = io.github.chriexpe.papo.notifications.MessageNotifications.CHANNEL_ID;
    static final String EXTRA_SERVER = io.github.chriexpe.papo.notifications.MessageNotifications.EXTRA_SERVER;
    static final String EXTRA_CHANNEL = io.github.chriexpe.papo.notifications.MessageNotifications.EXTRA_CHANNEL;
    static final String EXTRA_MESSAGE_ID = io.github.chriexpe.papo.notifications.MessageNotifications.EXTRA_MESSAGE_ID;
    static final String EXTRA_NOTIFICATION_ID = io.github.chriexpe.papo.notifications.MessageNotifications.EXTRA_NOTIFICATION_ID;
    static void ensureChannel(Context context) { io.github.chriexpe.papo.notifications.MessageNotifications.ensureChannel(context); }
    static boolean available(Context context) { return io.github.chriexpe.papo.notifications.MessageNotifications.available(context); }
    static void show(Context context, String payload) { io.github.chriexpe.papo.notifications.MessageNotifications.show(context,payload,PapoActivity.class,R.drawable.ic_message_notification); }
    static void clear(Context context, String payload) { io.github.chriexpe.papo.notifications.MessageNotifications.clear(context,payload); }
}
