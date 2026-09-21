package io.github.chriexpe.papo;

import android.graphics.Insets;
import android.os.Build;
import android.os.Bundle;
import android.view.View;
import android.view.WindowInsets;

import com.google.androidgamesdk.GameActivity;

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

    @Override
    protected void onCreate(Bundle savedInstanceState) {
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
