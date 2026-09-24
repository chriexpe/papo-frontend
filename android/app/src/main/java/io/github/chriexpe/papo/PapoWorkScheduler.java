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

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.concurrent.TimeUnit;

/** Mirrors the foreground server configuration into persistent unique WorkManager jobs. */
final class PapoWorkScheduler {
    private static final String PREFS = "papo-work";
    // Historical registry, deliberately not just the current desired set.
    //
    // We persist a work name here *before* it can be enqueued. Keeping old
    // names forever makes reconciliation crash-safe: after a process death in
    // the middle of sync(), the next foreground launch can still cancel every
    // periodic job that is no longer represented by Settings.servers.
    private static final String KEY_NAMES = "periodic-names";
    private static final String PREFIX = "papo-reconcile-";

    private PapoWorkScheduler() {}

    private static final class DesiredServer {
        final String serverKey;
        final String serverUrl;
        final String workName;

        DesiredServer(String serverKey, String serverUrl) {
            this.serverKey = serverKey;
            this.serverUrl = serverUrl;
            this.workName = PREFIX + serverKey;
        }
    }

    static boolean sync(Context context, String payload) {
        try {
            final JSONObject config = new JSONObject(payload);
            final boolean notificationsEnabled =
                    config.optBoolean("notifications_enabled", true);
            final JSONArray servers = config.getJSONArray("servers");
            final List<DesiredServer> desired = new ArrayList<>();
            final Set<String> wanted = new HashSet<>();

            for (int i = 0; i < servers.length(); i++) {
                final JSONObject server = servers.getJSONObject(i);
                final String serverKey = server.getString("server_key");
                final String serverUrl = server.getString("server_url");
                if (serverKey.isBlank() || serverUrl.isBlank()) {
                    continue;
                }
                final DesiredServer entry = new DesiredServer(serverKey, serverUrl);
                desired.add(entry);
                wanted.add(entry.workName);
            }

            final SharedPreferences prefs =
                    context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
            final Set<String> known =
                    new HashSet<>(prefs.getStringSet(KEY_NAMES, Set.of()));
            known.addAll(wanted);

            // Commit the historical registry before creating any new work. If
            // the process dies after this point, a later sync still knows every
            // name that may have reached WorkManager and can reconcile it.
            if (!prefs.edit().putStringSet(KEY_NAMES, known).commit()) {
                Log.e("papo-background", "falha ao persistir registro do WorkManager");
                return false;
            }

            final WorkManager workManager = WorkManager.getInstance(context);
            for (DesiredServer server : desired) {
                final Data input = new Data.Builder()
                        .putString(PapoReconcileWorker.INPUT_SERVER_KEY, server.serverKey)
                        .putString(PapoReconcileWorker.INPUT_SERVER_URL, server.serverUrl)
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
                                        15,
                                        TimeUnit.MINUTES)
                                .addTag("papo-reconcile")
                                .build();
                workManager.enqueueUniquePeriodicWork(
                        server.workName,
                        ExistingPeriodicWorkPolicy.UPDATE,
                        request);
            }

            for (String old : known) {
                if (!wanted.contains(old)) {
                    workManager.cancelUniqueWork(old);
                }
            }
            return true;
        } catch (Exception error) {
            Log.e("papo-background", "falha ao sincronizar WorkManager", error);
            return false;
        }
    }
}
