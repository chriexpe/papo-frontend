package io.github.chriexpe.papo;

import android.content.Context;
import android.content.SharedPreferences;
import android.util.Log;

import androidx.work.BackoffPolicy;
import androidx.work.Data;
import androidx.work.ExistingPeriodicWorkPolicy;
import androidx.work.PeriodicWorkRequest;
import androidx.work.WorkManager;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.HashSet;
import java.util.Set;
import java.util.concurrent.TimeUnit;

/** Mirrors the foreground server configuration into persistent unique WorkManager jobs. */
final class PapoWorkScheduler {
    private static final String PREFS = "papo-work";
    private static final String KEY_NAMES = "periodic-names";
    private static final String PREFIX = "papo-reconcile-";

    private PapoWorkScheduler() {}

    static void sync(Context context, String payload) {
        try {
            final JSONObject config = new JSONObject(payload);
            final boolean notificationsEnabled =
                    config.optBoolean("notifications_enabled", true);
            final JSONArray servers = config.getJSONArray("servers");
            final WorkManager workManager = WorkManager.getInstance(context);
            final Set<String> wanted = new HashSet<>();

            for (int i = 0; i < servers.length(); i++) {
                final JSONObject server = servers.getJSONObject(i);
                final String serverKey = server.getString("server_key");
                final String serverUrl = server.getString("server_url");
                if (serverKey.isBlank() || serverUrl.isBlank()) {
                    continue;
                }

                final String name = PREFIX + serverKey;
                wanted.add(name);
                final Data input = new Data.Builder()
                        .putString(PapoReconcileWorker.INPUT_SERVER_KEY, serverKey)
                        .putString(PapoReconcileWorker.INPUT_SERVER_URL, serverUrl)
                        .putBoolean(
                                PapoReconcileWorker.INPUT_NOTIFICATIONS_ENABLED,
                                notificationsEnabled)
                        .build();
                final PeriodicWorkRequest request =
                        new PeriodicWorkRequest.Builder(
                                PapoReconcileWorker.class,
                                15,
                                TimeUnit.MINUTES)
                                // No NetworkType constraint: a local Papo backend may be
                                // reachable on unvalidated Wi-Fi.
                                .setInputData(input)
                                .setBackoffCriteria(
                                        BackoffPolicy.EXPONENTIAL,
                                        30,
                                        TimeUnit.SECONDS)
                                .addTag("papo-reconcile")
                                .build();
                workManager.enqueueUniquePeriodicWork(
                        name,
                        ExistingPeriodicWorkPolicy.UPDATE,
                        request);
            }

            final SharedPreferences prefs =
                    context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            final Set<String> previous =
                    new HashSet<>(prefs.getStringSet(KEY_NAMES, Set.of()));
            for (String old : previous) {
                if (!wanted.contains(old)) {
                    workManager.cancelUniqueWork(old);
                }
            }
            prefs.edit().putStringSet(KEY_NAMES, wanted).apply();
        } catch (Exception error) {
            Log.e("papo-background", "falha ao sincronizar WorkManager", error);
        }
    }
}
