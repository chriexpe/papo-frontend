package io.github.chriexpe.papo;

import android.util.Log;

import androidx.annotation.NonNull;

import com.google.firebase.messaging.FirebaseMessagingService;
import com.google.firebase.messaging.RemoteMessage;

/**
 * Recebe o FCM do backend.
 *
 * <p>O backend manda mensagens com bloco {@code notification}: com o app fora
 * da tela o próprio Android as desenha, sem passar por aqui. Este método só é
 * chamado com a Activity visível, e aí a conexão ao vivo já notifica pelo
 * coordenador — mostrar de novo seria duplicata.
 */
public final class PapoMessagingService extends FirebaseMessagingService {
    @Override
    public void onNewToken(@NonNull String token) {
        PapoPush.deliver(token);
    }

    @Override
    public void onMessageReceived(@NonNull RemoteMessage message) {
        Log.d("papo-push", "push em primeiro plano ignorado: a conexão ao vivo notifica");
    }
}
