package io.github.chriexpe.papo;

import android.content.Context;
import android.util.Log;

import androidx.annotation.NonNull;
import androidx.work.Worker;
import androidx.work.WorkerParameters;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.File;

/** One bounded per-server reconciliation pass. Not a realtime transport. */
public final class PapoReconcileWorker extends Worker {
    static final String INPUT_SERVER_KEY = "server_key";
    static final String INPUT_SERVER_URL = "server_url";
    static final String INPUT_NOTIFICATIONS_ENABLED = "notifications_enabled";

    private static native String nativeRunBackgroundReconcile(
            String appPrivateRoot,
            String serverKey,
            String serverUrl,
            boolean notificationsEnabled,
            boolean platformNotificationsAvailable);

    private static native void nativeCancelBackgroundReconcile(String serverKey);

    static {
        // libpapo links against the packaged GStreamer aggregate, but the
        // background path never initializes GStreamer or creates media state.
        System.loadLibrary("gstreamer_android");
        System.loadLibrary("papo");
    }

    public PapoReconcileWorker(
            @NonNull Context appContext,
            @NonNull WorkerParameters workerParams) {
        super(appContext, workerParams);
    }

    @NonNull
    @Override
    public Result doWork() {
        final String serverKey = getInputData().getString(INPUT_SERVER_KEY);
        final String serverUrl = getInputData().getString(INPUT_SERVER_URL);
        final boolean notificationsEnabled =
                getInputData().getBoolean(INPUT_NOTIFICATIONS_ENABLED, true);
        if (serverKey == null || serverKey.isBlank()
                || serverUrl == null || serverUrl.isBlank()) {
            return Result.failure();
        }

        try {
            // GameActivity passes getFilesDir() into its native
            // internalDataPath. The cold Worker must use that exact root so
            // sessions/ and data/papo-cache.db are shared, not duplicated.
            final File privateRoot = getApplicationContext().getFilesDir();
            final String raw = nativeRunBackgroundReconcile(
                    privateRoot.getCanonicalPath(),
                    serverKey,
                    serverUrl,
                    notificationsEnabled,
                    MessageNotifications.available(getApplicationContext()));
            final JSONObject result = new JSONObject(raw);
            final JSONArray notifications = result.optJSONArray("notifications");
            if (notifications != null) {
                for (int i = 0; i < notifications.length(); i++) {
                    MessageNotifications.show(
                            getApplicationContext(),
                            notifications.getJSONObject(i).toString());
                }
            }

            final String status = result.optString("status", "transient_failure");
            return switch (status) {
                case "completed",
                        "no_session",
                        "skipped_foreground_runtime",
                        "permanent_auth_failure",
                        "server_locked" -> Result.success();
                case "cancelled" -> isStopped() ? Result.failure() : Result.success();
                case "deadline", "transient_failure" -> Result.retry();
                default -> Result.retry();
            };
        } catch (Exception error) {
            Log.e("papo-background", "worker falhou", error);
            return Result.retry();
        }
    }

    @Override
    public void onStopped() {
        final String serverKey = getInputData().getString(INPUT_SERVER_KEY);
        if (serverKey != null && !serverKey.isBlank()) {
            nativeCancelBackgroundReconcile(serverKey);
        }
        super.onStopped();
    }
}
