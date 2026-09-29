package io.github.chriexpe.papo;

import android.content.Context;
import android.os.Build;
import android.util.Log;

import com.google.firebase.FirebaseApp;
import com.google.firebase.messaging.FirebaseMessaging;

/**
 * Token FCM deste aparelho, levado ao Rust.
 *
 * <p>O Firebase só existe quando o APK foi montado com a configuração do
 * projeto (ver {@code build.gradle.kts}). Sem ela não há token, e o Papo
 * segue só com a reconciliação periódica.
 */
final class PapoPush {
    // O libpapo é carregado pelo GameActivity; antes dele não há para quem
    // entregar. Depois de carregado, fica até o processo morrer.
    private static volatile boolean nativeReady;

    private PapoPush() {}

    static boolean available(Context context) {
        return !FirebaseApp.getApps(context).isEmpty();
    }

    /** Chamado pela Activity depois de o libpapo carregar. */
    static void start(Context context) {
        nativeReady = true;
        if (!available(context)) {
            Log.i("papo-push", "Firebase não configurado neste APK");
            return;
        }
        FirebaseMessaging.getInstance()
                .getToken()
                .addOnSuccessListener(PapoPush::deliver)
                .addOnFailureListener(error ->
                        Log.w("papo-push", "sem token FCM", error));
    }

    /** Token novo do Firebase; com o app fechado, o próximo start o busca. */
    static void deliver(String token) {
        if (!nativeReady || token == null || token.isBlank()) {
            return;
        }
        PapoActivity.deliverPushToken(token, Build.MODEL);
    }
}
