package io.github.chriexpe.papo;

import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.graphics.Color;
import android.graphics.Insets;
import android.net.Uri;
import android.provider.OpenableColumns;
import android.os.Build;
import android.os.Bundle;
import android.text.Editable;
import android.text.InputFilter;
import android.text.InputType;
import android.text.TextWatcher;
import android.util.Log;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import android.widget.FrameLayout;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.List;

import com.google.androidgamesdk.GameActivity;

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

    /** Eventos do EditText Android que cobre o compositor/edição de mensagem. */
    private static native void nativeEditorTextChanged(String key, String text);
    private static native void nativeEditorSelectionChanged(String key, int start, int end);
    private static native void nativeEditorSubmit(String key);
    private static native void nativeEditorFocusChanged(String key, boolean focused);

    /** Responde ao Rust se a permissão saiu. Em `src/platform/permission.rs`. */
    private static native void nativePermissionResult(String permission, boolean granted);

    private static final int NATIVE_EDITOR_COMPOSER = 0;
    private static final int NATIVE_EDITOR_EDIT = 1;
    private static final int NATIVE_EDITOR_TEXT = 2;
    private static final int NATIVE_EDITOR_PASSWORD = 3;
    private static final int NATIVE_EDITOR_SEARCH = 4;

    private FrameLayout nativeEditorLayer;
    private NativeEditText nativeEditor;
    private String nativeEditorKey;
    private int nativeEditorMode = NATIVE_EDITOR_COMPOSER;
    private boolean mutatingNativeEditor;

    private int nativeEditorImeAction() {
        switch (nativeEditorMode) {
            case NATIVE_EDITOR_EDIT:
            case NATIVE_EDITOR_TEXT:
            case NATIVE_EDITOR_PASSWORD:
                return EditorInfo.IME_ACTION_DONE;
            case NATIVE_EDITOR_SEARCH:
                return EditorInfo.IME_ACTION_SEARCH;
            default:
                return EditorInfo.IME_ACTION_NONE;
        }
    }

    /**
     * O compositor Android é um EditText de verdade, não um TextEdit do egui
     * alimentado por eventos sintetizados. Isso deixa seleção, composição,
     * autocomplete, repetição de apagar, cursor pela barra de espaço e scroll
     * interno inteiramente sob responsabilidade do Android/IME.
     */
    private final class NativeEditText extends EditText {
        NativeEditText(Context context) {
            super(context);
        }

        @Override
        protected void onSelectionChanged(int start, int end) {
            super.onSelectionChanged(start, end);
            if (!mutatingNativeEditor && nativeEditorKey != null) {
                nativeEditorSelectionChanged(nativeEditorKey, start, end);
            }
        }

        @Override
        public android.view.inputmethod.InputConnection onCreateInputConnection(
                EditorInfo outAttrs) {
            final android.view.inputmethod.InputConnection connection =
                    super.onCreateInputConnection(outAttrs);
            outAttrs.imeOptions &= ~(EditorInfo.IME_MASK_ACTION
                    | EditorInfo.IME_FLAG_NO_ENTER_ACTION);
            outAttrs.imeOptions |= EditorInfo.IME_FLAG_NO_EXTRACT_UI
                    | EditorInfo.IME_FLAG_NO_FULLSCREEN
                    | nativeEditorImeAction();
            return connection;
        }
    }

    private void ensureNativeEditor() {
        if (nativeEditor != null) {
            return;
        }

        nativeEditorLayer = new FrameLayout(this);
        nativeEditorLayer.setClipChildren(false);
        nativeEditorLayer.setClipToPadding(false);
        nativeEditorLayer.setClickable(false);
        addContentView(
                nativeEditorLayer,
                new ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT));

        nativeEditor = new NativeEditText(this);
        nativeEditor.setBackground(null);
        nativeEditor.setBackgroundColor(Color.TRANSPARENT);
        nativeEditor.setIncludeFontPadding(false);
        nativeEditor.setPadding(0, 0, 0, 0);
        nativeEditor.setGravity(Gravity.START | Gravity.CENTER_VERTICAL);
        nativeEditor.setSingleLine(false);
        nativeEditor.setHorizontallyScrolling(false);
        nativeEditor.setSelectAllOnFocus(false);
        nativeEditor.setSaveEnabled(false);
        nativeEditor.setVerticalScrollBarEnabled(false);
        nativeEditor.setOverScrollMode(View.OVER_SCROLL_NEVER);
        nativeEditor.setInputType(
                InputType.TYPE_CLASS_TEXT
                        | InputType.TYPE_TEXT_VARIATION_SHORT_MESSAGE
                        | InputType.TYPE_TEXT_FLAG_MULTI_LINE
                        | InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
                        | InputType.TYPE_TEXT_FLAG_AUTO_CORRECT);

        nativeEditor.addTextChangedListener(new TextWatcher() {
            @Override
            public void beforeTextChanged(
                    CharSequence text, int start, int count, int after) {}

            @Override
            public void onTextChanged(
                    CharSequence text, int start, int before, int count) {}

            @Override
            public void afterTextChanged(Editable text) {
                if (!mutatingNativeEditor && nativeEditorKey != null) {
                    nativeEditorTextChanged(nativeEditorKey, text.toString());
                }
            }
        });

        nativeEditor.setOnFocusChangeListener((view, focused) -> {
            if (nativeEditorKey != null) {
                nativeEditorFocusChanged(nativeEditorKey, focused);
            }
        });

        nativeEditor.setOnEditorActionListener((view, actionId, event) -> {
            if (nativeEditorKey == null || nativeEditorMode == NATIVE_EDITOR_COMPOSER) {
                return false;
            }
            final boolean action = actionId == nativeEditorImeAction();
            final boolean enter = event != null
                    && event.getKeyCode() == KeyEvent.KEYCODE_ENTER
                    && event.getAction() == KeyEvent.ACTION_DOWN
                    && !event.isShiftPressed();
            if (action || enter) {
                nativeEditorSubmit(nativeEditorKey);
                return true;
            }
            return false;
        });

        nativeEditor.setVisibility(View.GONE);
        nativeEditorLayer.addView(
                nativeEditor,
                new FrameLayout.LayoutParams(1, 1));
    }

    /**
     * Posiciona o editor nativo exatamente sobre a área que o egui reservou.
     * Chamado da thread do render; toda mutação de View é repostada à UI thread.
     */
    public void showNativeEditor(
            String key,
            String text,
            String hint,
            int left,
            int top,
            int width,
            int height,
            float textSizePx,
            int textColor,
            int hintColor,
            int mode,
            int maxLines,
            int maxChars,
            boolean focus) {
        runOnUiThread(() -> {
            ensureNativeEditor();

            final boolean keyChanged = !key.equals(nativeEditorKey);
            final boolean modeChanged = nativeEditorMode != mode;
            nativeEditorKey = key;
            nativeEditorMode = mode;

            nativeEditor.setTextSize(TypedValue.COMPLEX_UNIT_PX, textSizePx);
            nativeEditor.setTextColor(textColor);
            nativeEditor.setHintTextColor(hintColor);
            nativeEditor.setHint(hint);

            final boolean singleLine = mode == NATIVE_EDITOR_TEXT
                    || mode == NATIVE_EDITOR_PASSWORD
                    || mode == NATIVE_EDITOR_SEARCH;
            nativeEditor.setSingleLine(singleLine);
            nativeEditor.setHorizontallyScrolling(singleLine);
            nativeEditor.setMinLines(1);
            nativeEditor.setMaxLines(singleLine ? 1 : Math.max(1, maxLines));
            nativeEditor.setGravity(Gravity.START | Gravity.CENTER_VERTICAL);

            if (mode == NATIVE_EDITOR_PASSWORD) {
                nativeEditor.setInputType(
                        InputType.TYPE_CLASS_TEXT
                                | InputType.TYPE_TEXT_VARIATION_PASSWORD
                                | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
            } else if (mode == NATIVE_EDITOR_SEARCH) {
                nativeEditor.setInputType(
                        InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_FILTER);
            } else if (singleLine) {
                nativeEditor.setInputType(
                        InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_AUTO_CORRECT);
            } else {
                nativeEditor.setInputType(
                        InputType.TYPE_CLASS_TEXT
                                | InputType.TYPE_TEXT_VARIATION_SHORT_MESSAGE
                                | InputType.TYPE_TEXT_FLAG_MULTI_LINE
                                | InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
                                | InputType.TYPE_TEXT_FLAG_AUTO_CORRECT);
            }

            nativeEditor.setFilters(
                    maxChars > 0
                            ? new InputFilter[] {new InputFilter.LengthFilter(maxChars)}
                            : new InputFilter[0]);
            nativeEditor.setImeOptions(
                    EditorInfo.IME_FLAG_NO_EXTRACT_UI
                            | EditorInfo.IME_FLAG_NO_FULLSCREEN
                            | nativeEditorImeAction());

            final FrameLayout.LayoutParams params =
                    (FrameLayout.LayoutParams) nativeEditor.getLayoutParams();
            params.width = Math.max(1, width);
            params.height = Math.max(1, height);
            params.leftMargin = left;
            params.topMargin = top;
            nativeEditor.setLayoutParams(params);
            nativeEditor.setVisibility(View.VISIBLE);
            nativeEditorLayer.bringToFront();
            nativeEditor.bringToFront();

            final String current = nativeEditor.getText().toString();
            if (keyChanged || !current.equals(text)) {
                final int oldSelection = Math.max(0, nativeEditor.getSelectionEnd());
                mutatingNativeEditor = true;
                nativeEditor.setText(text);
                final int target = keyChanged
                        ? text.length()
                        : Math.min(oldSelection, text.length());
                nativeEditor.setSelection(target);
                mutatingNativeEditor = false;
            }

            final InputMethodManager imm =
                    (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
            if (imm != null && (keyChanged || modeChanged) && nativeEditor.hasFocus()) {
                imm.restartInput(nativeEditor);
            }

            if (focus) {
                nativeEditor.post(() -> {
                    if (!key.equals(nativeEditorKey)) {
                        return;
                    }
                    nativeEditor.requestFocus();
                    final InputMethodManager keyboard =
                            (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
                    if (keyboard != null) {
                        keyboard.restartInput(nativeEditor);
                        keyboard.showSoftInput(nativeEditor, InputMethodManager.SHOW_IMPLICIT);
                    }
                });
            }
        });
    }

    /** Move o cursor depois de uma alteração iniciada pelo egui (ex.: :emoji:). */
    public void setNativeEditorSelection(String key, int selection) {
        runOnUiThread(() -> {
            if (nativeEditor == null || !key.equals(nativeEditorKey)) {
                return;
            }
            final int end = Math.max(
                    0,
                    Math.min(selection, nativeEditor.getText().length()));
            nativeEditor.setSelection(end);
            nativeEditor.requestFocus();
        });
    }

    /** Some com a View quando não existe campo nativo neste quadro. */
    public void hideNativeEditor() {
        runOnUiThread(() -> {
            if (nativeEditor == null || nativeEditor.getVisibility() != View.VISIBLE) {
                return;
            }
            final InputMethodManager imm =
                    (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
            nativeEditorKey = null;
            nativeEditor.clearFocus();
            if (imm != null) {
                imm.hideSoftInputFromWindow(nativeEditor.getWindowToken(), 0);
            }
            nativeEditor.setVisibility(View.GONE);
        });
    }

    /**
     * Pede uma permissão ao usuário. Chamado <b>do Rust</b>.
     *
     * <p>É a primeira coisa que anda no sentido contrário: até aqui a
     * Activity só empurrava (bordas, texto). Gravar e anexar começam do
     * outro lado, quando alguém toca no botão.
     *
     * <p>A resposta sempre volta pelo `nativePermissionResult`, inclusive
     * quando a permissão já estava dada — assim o lado Rust tem um caminho
     * só para tratar, em vez de dois.
     */
    /**
     * A permissão já está dada? Chamado <b>do Rust</b>.
     *
     * <p>O Android guarda isso entre execuções; o lado Rust, não. Sem esta
     * pergunta, a primeira vez que alguém toca em gravar depois de abrir o
     * aplicativo era sempre desperdiçada, esperando uma resposta que já
     * existia.
     */
    public boolean hasPermission(String permission) {
        return checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED;
    }

    public void requestPermission(String permission) {
        runOnUiThread(() -> {
            if (checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED) {
                nativePermissionResult(permission, true);
                return;
            }
            requestPermissions(new String[] {permission}, PERMISSION_REQUEST);
        });
    }

    private static final int PERMISSION_REQUEST = 1;
    private static final int PICK_REQUEST = 2;

    /** Entrega os anexos escolhidos ao Rust. Em `src/platform/files.rs`. */
    private static native void nativeFilesPicked(String[] paths, String[] names);

    /**
     * Abre o seletor de arquivos do sistema. Chamado <b>do Rust</b>.
     *
     * <p>O Android não entrega um caminho: entrega um `content://`, que é
     * uma porta para o arquivo de outro aplicativo e não sobrevive ao fim da
     * escolha. Como o Papo envia anexos a partir de um caminho de verdade,
     * cada escolha é copiada para o nosso cache antes de seguir.
     */
    public void pickFiles() {
        runOnUiThread(() -> {
            final Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType("*/*");
            intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
            startActivityForResult(intent, PICK_REQUEST);
        });
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != PICK_REQUEST) {
            return;
        }
        if (resultCode != RESULT_OK || data == null) {
            // Desistiu: o Rust precisa saber, senão o diálogo fica aberto
            // para sempre do lado dele.
            nativeFilesPicked(new String[0], new String[0]);
            return;
        }

        final List<Uri> chosen = new ArrayList<>();
        if (data.getClipData() != null) {
            for (int i = 0; i < data.getClipData().getItemCount(); i++) {
                chosen.add(data.getClipData().getItemAt(i).getUri());
            }
        } else if (data.getData() != null) {
            chosen.add(data.getData());
        }

        // Copiar pode demorar (o arquivo pode estar na nuvem): fora da
        // thread da interface, senão a janela congela no meio da escolha.
        new Thread(() -> copyAll(chosen)).start();
    }

    private void copyAll(List<Uri> chosen) {
        final List<String> paths = new ArrayList<>();
        final List<String> names = new ArrayList<>();
        final File dir = new File(getCacheDir(), "anexos");
        dir.mkdirs();

        for (Uri uri : chosen) {
            final String name = displayName(uri);
            final File dest = new File(dir, System.nanoTime() + "-" + name);
            try (InputStream in = getContentResolver().openInputStream(uri);
                    OutputStream out = new FileOutputStream(dest)) {
                if (in == null) {
                    continue;
                }
                final byte[] buffer = new byte[64 * 1024];
                int read;
                while ((read = in.read(buffer)) > 0) {
                    out.write(buffer, 0, read);
                }
                paths.add(dest.getAbsolutePath());
                names.add(name);
            } catch (Exception error) {
                Log.e("papo", "não deu para copiar o anexo " + uri, error);
            }
        }

        nativeFilesPicked(
                paths.toArray(new String[0]), names.toArray(new String[0]));
    }

    /** O nome que o usuário reconhece, e não o identificador do provedor. */
    private String displayName(Uri uri) {
        try (Cursor cursor =
                getContentResolver().query(uri, null, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                final int column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (column >= 0) {
                    final String name = cursor.getString(column);
                    if (name != null && !name.isEmpty()) {
                        return name;
                    }
                }
            }
        } catch (Exception error) {
            Log.w("papo", "sem nome para " + uri, error);
        }
        final String fallback = uri.getLastPathSegment();
        return fallback == null ? "anexo" : fallback;
    }

    @Override
    public void onRequestPermissionsResult(
            int requestCode, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(requestCode, permissions, results);
        for (int i = 0; i < permissions.length; i++) {
            final boolean granted = i < results.length
                    && results[i] == PackageManager.PERMISSION_GRANTED;
            nativePermissionResult(permissions[i], granted);
        }
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
