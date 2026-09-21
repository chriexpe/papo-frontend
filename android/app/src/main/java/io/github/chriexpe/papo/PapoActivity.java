package io.github.chriexpe.papo;

import android.graphics.Insets;
import android.os.Build;
import android.os.Bundle;
import android.view.View;
import android.view.WindowInsets;

import android.util.Log;

import com.google.androidgamesdk.GameActivity;
import com.google.androidgamesdk.gametextinput.State;

import org.freedesktop.gstreamer.GStreamer;

/**
 * A Activity do Papo.
 *
 * <p>De propósito, quase nada acontece aqui: o aplicativo inteiro é Rust, e
 * esta classe existe só como cola com a plataforma. A única coisa que ela faz
 * é medir as bordas que o sistema ocupa — barra de status, barra de
 * navegação e o recorte da câmera — e empurrar esses valores para o lado
 * nativo. Do Android 15 em diante a janela é sempre de borda a borda, então
 * sem essa medida a conversa ficaria por baixo do relógio.
 *
 * <p>O caminho é de mão única (Java chama o Rust) justamente para não custar
 * nada: o Rust guarda quatro inteiros e lê na hora de desenhar, sem precisar
 * chamar de volta para cá a cada quadro.
 */
public class PapoActivity extends GameActivity {

    /** Envia as bordas ao Rust. Implementada em `src/platform/safe_area.rs`. */
    private static native void nativeSetInsets(int left, int top, int right, int bottom);

    /** Envia o texto do teclado ao Rust. Implementada em `src/platform/ime.rs`. */
    private static native void nativeSetText(String text);

    /**
     * O teclado mudou o texto.
     *
     * <p>É daqui que o texto digitado chega ao Rust. O caminho natural seria
     * o lado nativo ler o buffer do GameTextInput sozinho, mas a função que
     * faz isso no `android-activity` 0.6.1 monta uma fatia sobre um ponteiro
     * nulo enquanto ninguém digitou nada, e aborta o processo assim que o
     * teclado sobe. Empurrar daqui contorna isso e ainda sai mais barato:
     * não é preciso olhar nada a cada quadro.
     */
    @Override
    public void stateChanged(State state, boolean dismissed) {
        super.stateChanged(state, dismissed);
        nativeSetText(state.text == null ? "" : state.text);
    }

    /**
     * O GStreamer precisa estar carregado antes do Papo.
     *
     * <p>Carregar a biblioteca dispara o `JNI_OnLoad` dela, que é onde os
     * métodos nativos do {@link GStreamer} são registrados — sem isso, o
     * `init` abaixo não acharia o método e estouraria.
     */
    static {
        System.loadLibrary("gstreamer_android");
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // Antes do `super`, que é quando o GameActivity carrega o libpapo e
        // o Rust começa a andar: quando o `android_main` chamar `gst_init`,
        // o ambiente do GStreamer já tem de estar de pé.
        try {
            GStreamer.init(this);
        } catch (Exception error) {
            // Não é motivo para não abrir: sem GStreamer o Papo ainda é um
            // chat, só que sem mídia. Quem depende dele já sabe lidar com a
            // ausência.
            Log.e("papo", "o GStreamer não iniciou", error);
        }

        super.onCreate(savedInstanceState);

        final View root = getWindow().getDecorView();
        root.setOnApplyWindowInsetsListener((view, insets) -> {
            publish(insets);
            return view.onApplyWindowInsets(insets);
        });
        root.requestApplyInsets();
    }

    private void publish(WindowInsets insets) {
        final int left;
        final int top;
        final int right;
        final int bottom;

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            final Insets bars = insets.getInsets(
                    WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
            // O teclado entra na conta para a caixa de escrever subir junto
            // com ele. É o **maior** dos dois, não a soma: enquanto o
            // teclado está aberto ele cobre a barra de navegação, e somar
            // deixaria uma faixa morta do tamanho da barra.
            //
            // Tomar o maior também resolve sozinho a dúvida de quem
            // redimensiona a janela. Se o sistema já a encolheu por causa do
            // teclado, esta janela não é mais coberta por ele e a medida do
            // teclado chega zerada — sobra a das barras, e nada é contado
            // duas vezes.
            final Insets ime = insets.getInsets(WindowInsets.Type.ime());
            final Insets room = Insets.max(bars, ime);
            left = room.left;
            top = room.top;
            right = room.right;
            bottom = room.bottom;
        } else {
            // Nos aparelhos antigos só existe esta medida, e ela já inclui
            // as barras do sistema e o teclado — ali o `adjustResize` ainda
            // encolhe a janela como sempre encolheu.
            left = insets.getSystemWindowInsetLeft();
            top = insets.getSystemWindowInsetTop();
            right = insets.getSystemWindowInsetRight();
            bottom = insets.getSystemWindowInsetBottom();
        }

        nativeSetInsets(left, top, right, bottom);
    }
}
