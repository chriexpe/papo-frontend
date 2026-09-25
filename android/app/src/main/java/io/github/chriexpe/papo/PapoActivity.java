package io.github.chriexpe.papo;

import android.Manifest;
import android.content.ComponentCallbacks2;
import android.content.Context;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.PictureInPictureParams;
import android.app.RemoteAction;
import android.service.notification.StatusBarNotification;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.res.Configuration;
import android.database.Cursor;
import android.graphics.Color;
import android.graphics.Insets;
import android.graphics.drawable.Icon;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.Uri;
import android.provider.OpenableColumns;
import android.os.Build;
import android.os.Bundle;
import android.text.Editable;
import android.text.InputFilter;
import android.text.InputType;
import android.text.Selection;
import android.text.TextWatcher;
import android.text.method.PasswordTransformationMethod;
import android.util.Log;
import android.util.Rational;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.ViewConfiguration;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.TextView;
import android.webkit.CookieManager;
import android.webkit.GeolocationPermissions;
import android.webkit.PermissionRequest;
import android.webkit.RenderProcessGoneDetail;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceError;
import android.webkit.WebResourceRequest;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;

import org.json.JSONObject;
import java.util.Map;

import com.google.androidgamesdk.GameActivity;
import com.google.androidgamesdk.gametextinput.InputConnection;
import com.google.androidgamesdk.gametextinput.Settings;
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

    /** Ciclo de vida/PiP da call. Implementados em `src/platform/android_call.rs`. */
    private static native void nativeSetPictureInPictureMode(boolean enabled);
    private static native void nativeLifecycleChanged(boolean foreground);
    private static native void nativeNetworkChanged(boolean available, long epoch, String transport);
    private static native void nativeSetPipSurface(Surface surface);
    private static native void nativeMessageNotificationTapped(String payload);

    /** Pressão de memória do sistema. Em `src/platform/memory_pressure.rs`. */
    private static native void nativeTrimMemory(int level);

    /** Envia o documento completo do IME ao Rust. */
    private static native void nativeSetText(
            String text,
            int selectionStart,
            int selectionEnd,
            int composingRegionStart,
            int composingRegionEnd);

    /** Eventos do EditText Android que cobre o compositor/edição de mensagem. */
    private static native void nativeEditorTextChanged(String key, String text);
    private static native void nativeEditorSelectionChanged(String key, int start, int end);
    private static native void nativeEditorSubmit(String key);
    private static native void nativeEditorFocusChanged(String key, boolean focused);

    /** Eventos de campos nativos comuns. Cada chave possui sua própria View/estado. */
    private static native void nativeFieldTextChanged(String key, String text);
    private static native void nativeFieldSubmit(String key);
    private static native void nativeFieldFocusChanged(String key, boolean focused);

    /** Responde ao Rust se a permissão saiu. Em `src/platform/permission.rs`. */
    private static native void nativePermissionResult(String permission, boolean granted);

    /** Eventos estreitos do browser embed. Não existe ponte JS para o Papo. */
    private static native void nativeWebEmbedExternal(String id, String url);
    private static native void nativeWebEmbedFailed(String id);
    private static native void nativeWebEmbedScroll(String id, float deltaYPx);

    private static final String EXTRA_MESSAGE_SERVER = MessageNotifications.EXTRA_SERVER;
    private static final String EXTRA_MESSAGE_CHANNEL = MessageNotifications.EXTRA_CHANNEL;
    private static final String EXTRA_MESSAGE_ID = MessageNotifications.EXTRA_MESSAGE_ID;
    private static final String EXTRA_NOTIFICATION_ID = MessageNotifications.EXTRA_NOTIFICATION_ID;

    private static final int NATIVE_EDITOR_COMPOSER = 0;
    private static final int NATIVE_EDITOR_EDIT = 1;

    private FrameLayout nativeEditorLayer;
    private NativeEditText nativeEditor;
    private String nativeEditorKey;
    private int nativeEditorMode = NATIVE_EDITOR_COMPOSER;
    private boolean nativeEditorAutocomplete;
    private boolean mutatingNativeEditor;
    private boolean imeWasVisible;
    private String callPresentation = "off";
    private ConnectivityManager connectivityManager;
    private ConnectivityManager.NetworkCallback networkCallback;
    private long currentNetworkHandle = -1L;
    private long networkEpoch;
    private boolean networkSnapshotKnown;
    private boolean networkAvailable;
    private FrameLayout pipLayer;
    private SurfaceView pipSurface;
    private TextView pipSpeaker;

    // Browser rico: uma única superfície viva. O Rust decide identidade,
    // geometria, floating/stop e ciclo de vida; a Activity só hospeda a View.
    private FrameLayout webEmbedLayer;
    private FrameLayout webEmbedClip;
    private TimelineWebView webEmbedView;
    private String webEmbedId;
    private String webEmbedInitialUrl;
    private FrameLayout webEmbedFullscreenLayer;
    private View webEmbedFullscreenView;
    private WebChromeClient.CustomViewCallback webEmbedFullscreenCallback;

    private void ensurePipLayer() {
        if (pipLayer != null) {
            return;
        }

        pipLayer = new FrameLayout(this);
        pipLayer.setBackgroundColor(Color.BLACK);
        pipLayer.setVisibility(View.GONE);

        pipSurface = new SurfaceView(this);
        pipSurface.setZOrderMediaOverlay(true);
        pipSurface.getHolder().setFormat(android.graphics.PixelFormat.RGBA_8888);
        pipSurface.getHolder().addCallback(new SurfaceHolder.Callback() {
            @Override
            public void surfaceCreated(SurfaceHolder holder) {
                nativeSetPipSurface(holder.getSurface());
            }

            @Override
            public void surfaceChanged(
                    SurfaceHolder holder,
                    int format,
                    int width,
                    int height) {
                nativeSetPipSurface(holder.getSurface());
            }

            @Override
            public void surfaceDestroyed(SurfaceHolder holder) {
                nativeSetPipSurface(null);
            }
        });
        pipLayer.addView(
                pipSurface,
                new FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT));

        pipSpeaker = new TextView(this);
        pipSpeaker.setTextColor(Color.WHITE);
        pipSpeaker.setTextSize(14);
        pipSpeaker.setShadowLayer(6.0f, 0.0f, 1.0f, Color.BLACK);
        pipSpeaker.setPadding(18, 8, 18, 8);
        pipSpeaker.setBackgroundColor(0x66000000);
        final FrameLayout.LayoutParams speakerParams = new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.START | Gravity.BOTTOM);
        speakerParams.setMargins(12, 0, 12, 12);
        pipLayer.addView(pipSpeaker, speakerParams);

        addContentView(
                pipLayer,
                new ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT));
        pipLayer.bringToFront();
    }

    public void setPipSpeaker(String name) {
        runOnUiThread(() -> {
            ensurePipLayer();
            pipSpeaker.setText(name == null ? "" : name);
            pipSpeaker.setVisibility(
                    name == null || name.isBlank() ? View.GONE : View.VISIBLE);
        });
    }

    public void setPipVideoVisible(String visible) {
        runOnUiThread(() -> {
            ensurePipLayer();
            final boolean hasVideo = "1".equals(visible);
            final FrameLayout.LayoutParams params = new FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    hasVideo ? (Gravity.START | Gravity.BOTTOM) : Gravity.CENTER);
            if (hasVideo) {
                params.setMargins(12, 0, 12, 12);
            }
            pipSpeaker.setLayoutParams(params);
            pipSpeaker.setBackgroundColor(hasVideo ? 0x66000000 : 0x00000000);
            pipSpeaker.setTextSize(hasVideo ? 14 : 18);
        });
    }

    public void closeCallPictureInPicture() {
        runOnUiThread(() -> {
            callPresentation = "off";
            if (pipLayer != null) {
                pipLayer.setVisibility(View.GONE);
            }
            if (isInPictureInPictureMode()) {
                finish();
            }
        });
    }


    private final class TimelineWebView extends WebView {
        private final int touchSlop;
        private float downX;
        private float downY;
        private float lastY;
        private boolean timelineDrag;

        TimelineWebView(Context context) {
            super(context);
            touchSlop = ViewConfiguration.get(context).getScaledTouchSlop();
        }

        @Override
        public boolean onTouchEvent(MotionEvent event) {
            switch (event.getActionMasked()) {
                case MotionEvent.ACTION_DOWN:
                    downX = event.getX();
                    downY = event.getY();
                    lastY = downY;
                    timelineDrag = false;
                    break;
                case MotionEvent.ACTION_MOVE:
                    final float dx = event.getX() - downX;
                    final float dy = event.getY() - downY;
                    if (!timelineDrag
                            && Math.abs(dy) > touchSlop
                            && Math.abs(dy) > Math.abs(dx) * 1.25f) {
                        timelineDrag = true;
                        final MotionEvent cancel = MotionEvent.obtain(event);
                        cancel.setAction(MotionEvent.ACTION_CANCEL);
                        super.onTouchEvent(cancel);
                        cancel.recycle();
                    }
                    if (timelineDrag) {
                        final float delta = event.getY() - lastY;
                        lastY = event.getY();
                        if (webEmbedId != null && Math.abs(delta) >= 0.5f) {
                            nativeWebEmbedScroll(webEmbedId, delta);
                        }
                        return true;
                    }
                    lastY = event.getY();
                    break;
                case MotionEvent.ACTION_UP:
                case MotionEvent.ACTION_CANCEL:
                    if (timelineDrag) {
                        timelineDrag = false;
                        return true;
                    }
                    timelineDrag = false;
                    break;
                default:
                    break;
            }
            return super.onTouchEvent(event);
        }
    }

    private void ensureWebEmbedLayer() {
        if (webEmbedLayer != null) {
            return;
        }
        webEmbedLayer = new FrameLayout(this);
        webEmbedLayer.setClipChildren(true);
        webEmbedLayer.setClipToPadding(true);
        webEmbedLayer.setClickable(false);
        webEmbedLayer.setVisibility(View.GONE);
        addContentView(
                webEmbedLayer,
                new ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT));

        webEmbedClip = new FrameLayout(this);
        webEmbedClip.setClipChildren(true);
        webEmbedClip.setClipToPadding(true);
        webEmbedClip.setClickable(false);
        webEmbedLayer.addView(webEmbedClip, new FrameLayout.LayoutParams(1, 1));
    }

    private boolean isHttps(String raw) {
        if (raw == null) {
            return false;
        }
        try {
            return "https".equalsIgnoreCase(Uri.parse(raw).getScheme());
        } catch (RuntimeException error) {
            return false;
        }
    }

    private WebViewClient webEmbedClient(final String id) {
        return new WebViewClient() {
            @Override
            public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                final Uri uri = request.getUrl();
                final String url = uri == null ? null : uri.toString();
                if (!isHttps(url)) {
                    return true;
                }
                // Redirects/player navigation stay in the isolated browser.
                // A main-frame navigation caused by an actual user gesture is
                // a request to leave the player and belongs in the system browser.
                if (request.isForMainFrame() && request.hasGesture()) {
                    nativeWebEmbedExternal(id, url);
                    return true;
                }
                return false;
            }

            @Override
            public void onReceivedError(
                    WebView view,
                    WebResourceRequest request,
                    WebResourceError error) {
                super.onReceivedError(view, request, error);
                if (request.isForMainFrame()) {
                    nativeWebEmbedFailed(id);
                }
            }

            @Override
            public boolean onRenderProcessGone(WebView view, RenderProcessGoneDetail detail) {
                nativeWebEmbedFailed(id);
                return true;
            }
        };
    }

    private WebChromeClient webEmbedChrome(final String id) {
        return new WebChromeClient() {
            @Override
            public void onPermissionRequest(PermissionRequest request) {
                request.deny();
            }

            @Override
            public void onGeolocationPermissionsShowPrompt(
                    String origin,
                    GeolocationPermissions.Callback callback) {
                callback.invoke(origin, false, false);
            }

            @Override
            public boolean onShowFileChooser(
                    WebView webView,
                    ValueCallback<Uri[]> filePathCallback,
                    FileChooserParams fileChooserParams) {
                return false;
            }

            @Override
            public void onShowCustomView(View view, CustomViewCallback callback) {
                runOnUiThread(() -> showWebEmbedFullscreen(view, callback));
            }

            @Override
            public void onHideCustomView() {
                runOnUiThread(PapoActivity.this::hideWebEmbedFullscreen);
            }
        };
    }

    private void configureWebEmbed(TimelineWebView view, String id) {
        final WebSettings settings = view.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setAllowFileAccess(false);
        settings.setAllowContentAccess(false);
        settings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        settings.setGeolocationEnabled(false);
        settings.setSupportMultipleWindows(false);
        settings.setJavaScriptCanOpenWindowsAutomatically(false);
        settings.setMediaPlaybackRequiresUserGesture(true);

        view.setBackgroundColor(Color.BLACK);
        view.setSaveEnabled(false);
        view.setWebViewClient(webEmbedClient(id));
        view.setWebChromeClient(webEmbedChrome(id));
        CookieManager.getInstance().setAcceptThirdPartyCookies(view, false);
    }

    /**
     * The app's own web identity, used as the embed's `Referer`. Google
     * requires an embedded player request to be identified; naming the app
     * (package id) matches what Android WebView attests via Media Integrity,
     * instead of pretending to be the provider.
     */
    private String webEmbedReferrer() {
        return "https://" + getPackageName() + "/";
    }

    public void createWebEmbed(String id, String url) {
        if (id == null || id.isBlank() || !isHttps(url)) {
            return;
        }
        runOnUiThread(() -> {
            ensureWebEmbedLayer();
            if (webEmbedView != null) {
                destroyWebEmbedNow();
            }

            webEmbedId = id;
            webEmbedInitialUrl = url;
            webEmbedView = new TimelineWebView(this);
            configureWebEmbed(webEmbedView, id);
            webEmbedClip.addView(webEmbedView, new FrameLayout.LayoutParams(1, 1));
            final Map<String, String> headers = new HashMap<>();
            headers.put("Referer", webEmbedReferrer());
            webEmbedView.loadUrl(url, headers);
            webEmbedLayer.setVisibility(View.GONE);
        });
    }

    public void presentWebEmbed(
            String id,
            int left,
            int top,
            int width,
            int height,
            int clipLeft,
            int clipTop,
            int clipWidth,
            int clipHeight) {
        runOnUiThread(() -> {
            if (webEmbedView == null || webEmbedId == null || !webEmbedId.equals(id)) {
                return;
            }

            final FrameLayout.LayoutParams clipParams =
                    (FrameLayout.LayoutParams) webEmbedClip.getLayoutParams();
            clipParams.width = Math.max(1, clipWidth);
            clipParams.height = Math.max(1, clipHeight);
            clipParams.leftMargin = clipLeft;
            clipParams.topMargin = clipTop;
            webEmbedClip.setLayoutParams(clipParams);

            final FrameLayout.LayoutParams viewParams =
                    (FrameLayout.LayoutParams) webEmbedView.getLayoutParams();
            viewParams.width = Math.max(1, width);
            viewParams.height = Math.max(1, height);
            viewParams.leftMargin = left - clipLeft;
            viewParams.topMargin = top - clipTop;
            webEmbedView.setLayoutParams(viewParams);

            webEmbedView.onResume();
            webEmbedView.setVisibility(View.VISIBLE);
            webEmbedClip.setVisibility(View.VISIBLE);
            webEmbedLayer.setVisibility(View.VISIBLE);
            webEmbedLayer.bringToFront();
            webEmbedView.bringToFront();

            // Native text is intentionally above the browser when both exist.
            if (nativeEditorLayer != null && nativeEditor != null
                    && nativeEditor.getVisibility() == View.VISIBLE) {
                nativeEditorLayer.bringToFront();
                nativeEditor.bringToFront();
            }
            if (nativeFieldLayer != null) {
                nativeFieldLayer.bringToFront();
            }
            if (pipLayer != null && isInPictureInPictureMode()) {
                pipLayer.bringToFront();
            }
        });
    }

    public void suspendWebEmbed(String id) {
        runOnUiThread(() -> {
            if (webEmbedView == null || webEmbedId == null || !webEmbedId.equals(id)) {
                return;
            }
            webEmbedView.onPause();
            webEmbedLayer.setVisibility(View.GONE);
        });
    }

    public void resumeWebEmbed(String id) {
        runOnUiThread(() -> {
            if (webEmbedView == null || webEmbedId == null || !webEmbedId.equals(id)) {
                return;
            }
            webEmbedView.onResume();
        });
    }

    public void destroyWebEmbed(String id) {
        runOnUiThread(() -> {
            if (webEmbedId == null || !webEmbedId.equals(id)) {
                return;
            }
            destroyWebEmbedNow();
        });
    }

    private void destroyWebEmbedNow() {
        hideWebEmbedFullscreen();
        if (webEmbedView != null) {
            webEmbedView.stopLoading();
            webEmbedView.onPause();
            if (webEmbedClip != null) {
                webEmbedClip.removeView(webEmbedView);
            }
            webEmbedView.destroy();
        }
        webEmbedView = null;
        webEmbedId = null;
        webEmbedInitialUrl = null;
        if (webEmbedLayer != null) {
            webEmbedLayer.setVisibility(View.GONE);
        }
    }

    private void showWebEmbedFullscreen(View view, WebChromeClient.CustomViewCallback callback) {
        if (view == null || webEmbedFullscreenView != null) {
            if (callback != null) {
                callback.onCustomViewHidden();
            }
            return;
        }
        webEmbedFullscreenView = view;
        webEmbedFullscreenCallback = callback;
        webEmbedFullscreenLayer = new FrameLayout(this);
        webEmbedFullscreenLayer.setBackgroundColor(Color.BLACK);
        webEmbedFullscreenLayer.addView(
                view,
                new FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT));
        addContentView(
                webEmbedFullscreenLayer,
                new ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT));
        webEmbedFullscreenLayer.bringToFront();
    }

    private void hideWebEmbedFullscreen() {
        if (webEmbedFullscreenView == null) {
            return;
        }
        if (webEmbedFullscreenLayer != null) {
            webEmbedFullscreenLayer.removeView(webEmbedFullscreenView);
            final ViewGroup parent = (ViewGroup) webEmbedFullscreenLayer.getParent();
            if (parent != null) {
                parent.removeView(webEmbedFullscreenLayer);
            }
        }
        webEmbedFullscreenView = null;
        webEmbedFullscreenLayer = null;
        if (webEmbedFullscreenCallback != null) {
            webEmbedFullscreenCallback.onCustomViewHidden();
            webEmbedFullscreenCallback = null;
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
                    | (nativeEditorMode == NATIVE_EDITOR_EDIT || nativeEditorAutocomplete
                            ? EditorInfo.IME_ACTION_DONE
                            : EditorInfo.IME_ACTION_NONE);
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
            if ((nativeEditorMode != NATIVE_EDITOR_EDIT && !nativeEditorAutocomplete)
                    || nativeEditorKey == null) {
                return false;
            }
            final boolean done = actionId == EditorInfo.IME_ACTION_DONE;
            final boolean enter = event != null
                    && event.getKeyCode() == KeyEvent.KEYCODE_ENTER
                    && event.getAction() == KeyEvent.ACTION_DOWN
                    && !event.isShiftPressed();
            if (done || enter) {
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
            boolean autocomplete,
            boolean focus) {
        runOnUiThread(() -> {
            ensureNativeEditor();

            final boolean keyChanged = !key.equals(nativeEditorKey);
            final boolean modeChanged = nativeEditorMode != mode;
            final boolean autocompleteChanged = nativeEditorAutocomplete != autocomplete;
            nativeEditorKey = key;
            nativeEditorMode = mode;
            nativeEditorAutocomplete = autocomplete;

            nativeEditor.setTextSize(TypedValue.COMPLEX_UNIT_PX, textSizePx);
            nativeEditor.setTextColor(textColor);
            nativeEditor.setHintTextColor(hintColor);
            nativeEditor.setHint(hint);
            nativeEditor.setMinLines(1);
            nativeEditor.setMaxLines(Math.max(1, maxLines));
            nativeEditor.setImeOptions(
                    EditorInfo.IME_FLAG_NO_EXTRACT_UI
                            | EditorInfo.IME_FLAG_NO_FULLSCREEN
                            | (mode == NATIVE_EDITOR_EDIT || autocomplete
                                    ? EditorInfo.IME_ACTION_DONE
                                    : EditorInfo.IME_ACTION_NONE));

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
            if (imm != null && (keyChanged || modeChanged || autocompleteChanged)
                    && nativeEditor.hasFocus()) {
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


    private static final int NATIVE_FIELD_TEXT = 0;
    private static final int NATIVE_FIELD_PASSWORD = 1;
    private static final int NATIVE_FIELD_SEARCH = 2;

    private FrameLayout nativeFieldLayer;
    private final Map<String, NativeFieldEditText> nativeFields = new HashMap<>();

    /**
     * Campos Android comuns ficam completamente separados do editor do chat.
     * Não existe "campo ativo" compartilhado: cada chave possui sua própria
     * EditText, Editable, seleção, composição e ciclo de foco.
     */
    private final class NativeFieldEditText extends EditText {
        final String key;
        boolean mutating;
        int imeAction = EditorInfo.IME_ACTION_DONE;

        NativeFieldEditText(Context context, String key) {
            super(context);
            this.key = key;
            setBackground(null);
            setBackgroundColor(Color.TRANSPARENT);
            setIncludeFontPadding(false);
            setPadding(0, 0, 0, 0);
            setGravity(Gravity.START | Gravity.CENTER_VERTICAL);
            setSingleLine(true);
            setHorizontallyScrolling(true);
            setSelectAllOnFocus(false);
            setSaveEnabled(false);
            setVerticalScrollBarEnabled(false);
            setOverScrollMode(View.OVER_SCROLL_NEVER);

            addTextChangedListener(new TextWatcher() {
                @Override
                public void beforeTextChanged(CharSequence text, int start, int count, int after) {}

                @Override
                public void onTextChanged(CharSequence text, int start, int before, int count) {}

                @Override
                public void afterTextChanged(Editable text) {
                    if (!mutating) {
                        nativeFieldTextChanged(key, text.toString());
                    }
                }
            });

            setOnFocusChangeListener((view, focused) ->
                    nativeFieldFocusChanged(key, focused));

            setOnEditorActionListener((view, actionId, event) -> {
                final boolean action = actionId == EditorInfo.IME_ACTION_DONE
                        || actionId == EditorInfo.IME_ACTION_SEARCH;
                final boolean enter = event != null
                        && event.getKeyCode() == KeyEvent.KEYCODE_ENTER
                        && event.getAction() == KeyEvent.ACTION_DOWN
                        && !event.isShiftPressed();
                if (action || enter) {
                    nativeFieldSubmit(key);
                    return true;
                }
                return false;
            });
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
                    | imeAction;
            return connection;
        }
    }

    private void ensureNativeFieldLayer() {
        if (nativeFieldLayer != null) {
            return;
        }
        nativeFieldLayer = new FrameLayout(this);
        nativeFieldLayer.setClipChildren(false);
        nativeFieldLayer.setClipToPadding(false);
        nativeFieldLayer.setClickable(false);
        addContentView(
                nativeFieldLayer,
                new ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT));
    }

    public void showNativeField(
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
            int maxChars,
            boolean focus) {
        runOnUiThread(() -> {
            ensureNativeFieldLayer();
            NativeFieldEditText editor = nativeFields.get(key);
            final boolean created = editor == null;
            if (created) {
                editor = new NativeFieldEditText(this, key);
                nativeFields.put(key, editor);
                nativeFieldLayer.addView(editor, new FrameLayout.LayoutParams(1, 1));
            }

            // Tudo abaixo é configuração/programmatic sync. Algumas versões
            // do Android disparam TextWatcher ao trocar inputType,
            // transformationMethod ou filtros. Esses eventos NÃO são edição do
            // usuário e jamais podem voltar ao Rust como texto novo.
            editor.mutating = true;

            editor.setTextSize(TypedValue.COMPLEX_UNIT_PX, textSizePx);
            editor.setTextColor(textColor);
            editor.setHintTextColor(hintColor);
            editor.setHint(hint);
            editor.setFilters(maxChars > 0
                    ? new InputFilter[] {new InputFilter.LengthFilter(maxChars)}
                    : new InputFilter[0]);

            final int inputType;
            final int action;
            if (mode == NATIVE_FIELD_PASSWORD) {
                inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD;
                action = EditorInfo.IME_ACTION_DONE;
                editor.setTransformationMethod(PasswordTransformationMethod.getInstance());
            } else if (mode == NATIVE_FIELD_SEARCH) {
                inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_NORMAL;
                action = EditorInfo.IME_ACTION_SEARCH;
                editor.setTransformationMethod(null);
            } else {
                inputType = InputType.TYPE_CLASS_TEXT
                        | InputType.TYPE_TEXT_VARIATION_NORMAL
                        | InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
                        | InputType.TYPE_TEXT_FLAG_AUTO_CORRECT;
                action = EditorInfo.IME_ACTION_DONE;
                editor.setTransformationMethod(null);
            }
            if (editor.getInputType() != inputType) {
                editor.setInputType(inputType);
            }
            editor.imeAction = action;
            editor.setImeOptions(
                    EditorInfo.IME_FLAG_NO_EXTRACT_UI
                            | EditorInfo.IME_FLAG_NO_FULLSCREEN
                            | action);

            final FrameLayout.LayoutParams params =
                    (FrameLayout.LayoutParams) editor.getLayoutParams();
            params.width = Math.max(1, width);
            params.height = Math.max(1, height);
            params.leftMargin = left;
            params.topMargin = top;
            editor.setLayoutParams(params);
            editor.setVisibility(View.VISIBLE);
            nativeFieldLayer.bringToFront();
            editor.bringToFront();

            // Enquanto o usuário digita, Android é a fonte de verdade. Fora
            // disso, Rust repopula a View quando ela nasce ou perde foco.
            if ((created || !editor.hasFocus()) && !editor.getText().toString().equals(text)) {
                editor.setText(text);
                editor.setSelection(editor.getText().length());
            }

            editor.mutating = false;

            if (focus) {
                final NativeFieldEditText target = editor;
                target.post(() -> {
                    target.requestFocus();
                    final InputMethodManager keyboard =
                            (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
                    if (keyboard != null) {
                        keyboard.showSoftInput(target, InputMethodManager.SHOW_IMPLICIT);
                    }
                });
            }
        });
    }

    public void hideNativeField(String key) {
        runOnUiThread(() -> {
            final NativeFieldEditText editor = nativeFields.remove(key);
            if (editor == null) {
                return;
            }
            if (editor.hasFocus()) {
                final InputMethodManager keyboard =
                        (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
                if (keyboard != null) {
                    keyboard.hideSoftInputFromWindow(editor.getWindowToken(), 0);
                }
            }
            editor.clearFocus();
            if (nativeFieldLayer != null) {
                nativeFieldLayer.removeView(editor);
            }
        });
    }

    /**
     * O mesmo GameTextInput do GameActivity, mas observando cada operação no
     * ponto em que o teclado realmente a executa.
     *
     * <p>Alguns IMEs agrupam movimentos de seleção e repetição de apagar antes
     * de o Listener de alto nível publicar o State. Para um editor desenhado
     * pelo egui isso é tarde demais: a barra de espaço parece "teleportar" o
     * cursor e apagar segurado pode parecer um único toque. Aqui cada mutação
     * publica imediatamente o Editable completo, sem inventar KeyEvents no Rust.
     */
    private final class TrackingInputConnection extends InputConnection {
        TrackingInputConnection(View target, Settings settings) {
            super(PapoActivity.this, target, settings);
            setListener(PapoActivity.this);
        }

        private void publishEditorState() {
            final Editable editable = getEditable();
            if (editable == null) {
                return;
            }
            nativeSetText(
                    editable.toString(),
                    Selection.getSelectionStart(editable),
                    Selection.getSelectionEnd(editable),
                    BaseInputConnection.getComposingSpanStart(editable),
                    BaseInputConnection.getComposingSpanEnd(editable));
        }

        private boolean publish(boolean result) {
            publishEditorState();
            return result;
        }

        @Override
        public boolean setSelection(int start, int end) {
            return publish(super.setSelection(start, end));
        }

        @Override
        public boolean deleteSurroundingText(int beforeLength, int afterLength) {
            return publish(super.deleteSurroundingText(beforeLength, afterLength));
        }

        @Override
        public boolean deleteSurroundingTextInCodePoints(int beforeLength, int afterLength) {
            return publish(super.deleteSurroundingTextInCodePoints(beforeLength, afterLength));
        }

        @Override
        public boolean setComposingText(CharSequence text, int newCursorPosition) {
            return publish(super.setComposingText(text, newCursorPosition));
        }

        @Override
        public boolean setComposingRegion(int start, int end) {
            return publish(super.setComposingRegion(start, end));
        }

        @Override
        public boolean finishComposingText() {
            return publish(super.finishComposingText());
        }

        @Override
        public boolean commitText(CharSequence text, int newCursorPosition) {
            return publish(super.commitText(text, newCursorPosition));
        }

        @Override
        public boolean sendKeyEvent(KeyEvent event) {
            return publish(super.sendKeyEvent(event));
        }

        @Override
        public boolean endBatchEdit() {
            return publish(super.endBatchEdit());
        }
    }

    /**
     * GameActivity 4.4.0 cria uma conexão concreta dentro do SurfaceView e
     * entrega exatamente essa instância tanto ao Android quanto ao lado nativo.
     * O campo é package-private, então substituímos uma única vez por reflexão
     * antes de GameActivity registrar a conexão no C/Rust.
     */
    @Override
    protected InputEnabledSurfaceView createSurfaceView() {
        final InputEnabledSurfaceView view = new InputEnabledSurfaceView(this);
        final EditorInfo editorInfo = getImeEditorInfo();
        final TrackingInputConnection connection = new TrackingInputConnection(
                view,
                new Settings(
                        editorInfo,
                        editorInfo.inputType == InputType.TYPE_NULL));
        try {
            final Field field =
                    InputEnabledSurfaceView.class.getDeclaredField("mInputConnection");
            field.setAccessible(true);
            field.set(view, connection);
        } catch (ReflectiveOperationException error) {
            Log.e("papo-ime", "não deu para instalar a conexão de texto rastreada", error);
        }
        return view;
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
    private void ensureMessageNotificationChannel() {
        MessageNotifications.ensureChannel(this);
    }

    /** Garante o canal e, no Android 13+, pede a permissão de notificações. */
    public void ensureMessageNotificationPermission() {
        runOnUiThread(() -> {
            ensureMessageNotificationChannel();
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
                    && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                    != PackageManager.PERMISSION_GRANTED) {
                requestPermissions(
                        new String[] {Manifest.permission.POST_NOTIFICATIONS},
                        PERMISSION_REQUEST);
            }
        });
    }

    /**
     * Publica uma mensagem que o cliente já recebeu pela rede.
     *
     * O JSON vem do Rust porque a Activity não conhece servidor/canal/usuário.
     * Não há transporte em background aqui: esta função só transforma um
     * evento já recebido em notificação nativa.
     */
    public void showMessageNotification(String payload) {
        runOnUiThread(() -> MessageNotifications.show(this, payload));
    }

    /** Remove da gaveta as notificações do canal que acabou de ser lido. */
    public void clearMessageNotifications(String payload) {
        runOnUiThread(() -> MessageNotifications.clear(this, payload));
    }

    /** Synchronizes persistent periodic work with the current Rust settings. */
    public boolean syncBackgroundReconcile(String payload) {
        return PapoWorkScheduler.sync(this, payload);
    }

    private void handleMessageNotificationIntent(Intent intent) {
        if (intent == null || !intent.hasExtra(EXTRA_MESSAGE_ID)) {
            return;
        }
        try {
            final JSONObject target = new JSONObject()
                    .put("server_url", intent.getStringExtra(EXTRA_MESSAGE_SERVER))
                    .put("channel_id", intent.getStringExtra(EXTRA_MESSAGE_CHANNEL))
                    .put("message_id", intent.getStringExtra(EXTRA_MESSAGE_ID))
                    .put("notification_id", intent.getStringExtra(EXTRA_NOTIFICATION_ID));
            nativeMessageNotificationTapped(target.toString());
        } catch (Exception error) {
            Log.e("papo-notify", "alvo inválido da notificação", error);
        } finally {
            intent.removeExtra(EXTRA_MESSAGE_SERVER);
            intent.removeExtra(EXTRA_MESSAGE_CHANNEL);
            intent.removeExtra(EXTRA_MESSAGE_ID);
            intent.removeExtra(EXTRA_NOTIFICATION_ID);
        }
    }

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

    /** Inicia a execução em primeiro plano da call enquanto a Activity está visível. */
    public void startCallService(String title) {
        runOnUiThread(() -> {
            // Android 13+ permite o FGS sem esta permissão, mas esconde a
            // notificação da gaveta. Pedimos aqui, depois da decisão do
            // microfone, sem bloquear o início da call.
            if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                    != PackageManager.PERMISSION_GRANTED) {
                requestPermissions(
                        new String[] {Manifest.permission.POST_NOTIFICATIONS},
                        PERMISSION_REQUEST);
            }

            final Intent intent = new Intent(this, CallService.class)
                    .setAction(CallService.ACTION_START)
                    .putExtra(CallService.EXTRA_TITLE, title)
                    .putExtra(CallService.EXTRA_MUTED, true)
                    .putExtra(CallService.EXTRA_CAMERA, false);
            startForegroundService(intent);
        });
    }

    /** Atualiza os tipos/controles do foreground service. */
    public void updateCallService(String state) {
        runOnUiThread(() -> {
            try {
                final JSONObject json = new JSONObject(state);
                final Intent intent = new Intent(this, CallService.class)
                        .setAction(CallService.ACTION_UPDATE)
                        .putExtra(CallService.EXTRA_MUTED, json.optBoolean("muted", true))
                        .putExtra(CallService.EXTRA_CAMERA, json.optBoolean("camera", false))
                        .putExtra(CallService.EXTRA_MEMBERS, json.optInt("members", 0))
                        .putExtra(CallService.EXTRA_SPEAKER, json.optString("speaker", ""));
                startService(intent);
            } catch (Exception error) {
                Log.e("papo-call", "estado inválido do serviço da call", error);
            }
        });
    }

    /** Encerra o foreground service quando a call termina. */
    public void stopCallService() {
        runOnUiThread(() -> stopService(new Intent(this, CallService.class)));
    }

    /**
     * Mantém os parâmetros do PiP sincronizados com o estado da call.
     * Android 12+ faz a transição automática ao gesto Home quando há vídeo.
     */
    public void setCallPresentation(String state) {
        runOnUiThread(() -> {
            if (state.equals(callPresentation)) {
                return;
            }
            callPresentation = state;

            if ("off".equals(state) && isInPictureInPictureMode()) {
                // A call terminou enquanto o usuário estava no Home. Fechar a
                // Activity faz a bolha desaparecer em vez de deixar um PiP
                // vazio; abrir o Papo depois cria a Activity normalmente.
                finish();
                return;
            }

            final boolean video = state.startsWith("video");
            final boolean muted = state.contains("muted=1");
            final boolean camera = state.contains("camera=1");
            final List<RemoteAction> actions = new ArrayList<>();

            final PendingIntent mute = PendingIntent.getService(
                    this,
                    21,
                    new Intent(this, CallService.class)
                            .setAction(CallService.ACTION_TOGGLE_MUTE),
                    PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
            actions.add(new RemoteAction(
                    Icon.createWithResource(
                            this,
                            muted ? R.drawable.ic_mic_off_notification : R.drawable.ic_mic_notification),
                    muted ? "Ativar microfone" : "Silenciar",
                    "Alternar microfone",
                    mute));

            final PendingIntent cameraAction = PendingIntent.getService(
                    this,
                    22,
                    new Intent(this, CallService.class)
                            .setAction(CallService.ACTION_TOGGLE_CAMERA),
                    PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
            actions.add(new RemoteAction(
                    Icon.createWithResource(
                            this,
                            camera ? R.drawable.ic_camera_off_notification : R.drawable.ic_camera_notification),
                    camera ? "Desligar câmera" : "Ligar câmera",
                    "Alternar câmera",
                    cameraAction));

            final PendingIntent hangup = PendingIntent.getService(
                    this,
                    23,
                    new Intent(this, CallService.class)
                            .setAction(CallService.ACTION_HANGUP),
                    PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
            actions.add(new RemoteAction(
                    Icon.createWithResource(this, R.drawable.ic_hangup_notification),
                    "Desligar",
                    "Sair da chamada",
                    hangup));

            final PictureInPictureParams params = new PictureInPictureParams.Builder()
                    .setAspectRatio(new Rational(16, 9))
                    .setAutoEnterEnabled(video)
                    .setSeamlessResizeEnabled(false)
                    .setActions(actions)
                    .build();
            setPictureInPictureParams(params);
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
     * Fallback/espelho do Listener do GameTextInput.
     *
     * <p>As operações interativas já são publicadas diretamente pela
     * TrackingInputConnection; este callback continua cobrindo qualquer
     * mudança que a biblioteca produza por outro caminho.
     */
    @Override
    public void stateChanged(State state, boolean dismissed) {
        super.stateChanged(state, dismissed);
        nativeSetText(
                state.text == null ? "" : state.text,
                state.selectionStart,
                state.selectionEnd,
                state.composingRegionStart,
                state.composingRegionEnd);
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

    private String networkTransport(NetworkCapabilities capabilities) {
        if (capabilities == null) {
            return "other";
        }
        if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
            return "wifi";
        }
        if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) {
            return "cellular";
        }
        if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) {
            return "ethernet";
        }
        if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) {
            return "vpn";
        }
        return "other";
    }

    /**
     * Publica a identidade do default network, não "Internet validada".
     *
     * <p>Um Wi-Fi sem NET_CAPABILITY_VALIDATED ainda pode alcançar um backend
     * Papo na LAN. Reachability continua sendo responsabilidade de cada
     * runtime WebSocket/REST.
     */
    private void publishCurrentNetwork() {
        if (connectivityManager == null) {
            return;
        }

        final Network active = connectivityManager.getActiveNetwork();
        if (active == null) {
            if (networkSnapshotKnown && !networkAvailable) {
                return;
            }
            networkSnapshotKnown = true;
            networkAvailable = false;
            currentNetworkHandle = -1L;
            networkEpoch++;
            nativeNetworkChanged(false, networkEpoch, "none");
            return;
        }

        final long handle = active.getNetworkHandle();
        if (networkSnapshotKnown && networkAvailable && currentNetworkHandle == handle) {
            return;
        }

        networkSnapshotKnown = true;
        networkAvailable = true;
        currentNetworkHandle = handle;
        networkEpoch++;
        nativeNetworkChanged(
                true,
                networkEpoch,
                networkTransport(connectivityManager.getNetworkCapabilities(active)));
    }

    private void registerNetworkCallback() {
        connectivityManager = getSystemService(ConnectivityManager.class);
        if (connectivityManager == null) {
            return;
        }

        networkCallback = new ConnectivityManager.NetworkCallback() {
            @Override
            public void onAvailable(Network network) {
                // Query the process default rather than trusting callback
                // ordering during Wi-Fi/cellular handover.
                publishCurrentNetwork();
            }

            @Override
            public void onLost(Network network) {
                // Give an arriving replacement default network one main-loop
                // turn to become active. This avoids A-lost/B-available
                // handovers looking like a false no-network interval.
                getWindow().getDecorView().post(PapoActivity.this::publishCurrentNetwork);
            }

            @Override
            public void onCapabilitiesChanged(
                    Network network, NetworkCapabilities networkCapabilities) {
                // Same Network handle is intentionally deduplicated.
                publishCurrentNetwork();
            }
        };

        try {
            connectivityManager.registerDefaultNetworkCallback(networkCallback);
            publishCurrentNetwork();
        } catch (RuntimeException error) {
            Log.e("papo-network", "não foi possível observar a rede padrão", error);
            networkCallback = null;
        }
    }

    private void unregisterNetworkCallback() {
        if (connectivityManager == null || networkCallback == null) {
            return;
        }
        try {
            connectivityManager.unregisterNetworkCallback(networkCallback);
        } catch (RuntimeException error) {
            Log.w("papo-network", "callback de rede já não estava registrado", error);
        }
        networkCallback = null;
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
        // libpapo já foi carregado pelo GameActivity; JNI pode ser chamado
        // com segurança a partir deste ponto.
        registerNetworkCallback();
        ensurePipLayer();
        ensureMessageNotificationChannel();
        handleMessageNotificationIntent(getIntent());

        final View root = getWindow().getDecorView();
        root.setOnApplyWindowInsetsListener((view, insets) -> {
            publish(insets);

            // Fechar o teclado pelo Back não tira foco de um EditText. Sem
            // isso o compositor continuava com o cursor piscando e seguia
            // sendo um alvo Android invisível por cima da UI.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                final boolean imeVisible = insets.isVisible(WindowInsets.Type.ime());
                if (imeWasVisible && !imeVisible) {
                    if (nativeEditor != null && nativeEditor.hasFocus()) {
                        nativeEditor.clearFocus();
                    }
                    for (NativeFieldEditText field : nativeFields.values()) {
                        if (field.hasFocus()) {
                            field.clearFocus();
                        }
                    }
                }
                imeWasVisible = imeVisible;
            }

            return view.onApplyWindowInsets(insets);
        });
        root.requestApplyInsets();
    }

    @Override
    public void onBackPressed() {
        if (webEmbedFullscreenView != null) {
            hideWebEmbedFullscreen();
            return;
        }
        super.onBackPressed();
    }

    @Override
    protected void onDestroy() {
        unregisterNetworkCallback();
        destroyWebEmbedNow();
        super.onDestroy();
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        handleMessageNotificationIntent(intent);
    }

    /**
     * A Activity está saindo de cena. O eframe grava aqui (o laço de quadros
     * para no onPause), então este é o último instante em que um trim
     * publicado ainda pode ser aplicado antes da suspensão — o onTrimMemory
     * real do sistema chega junto do onStop, tarde demais para este quadro.
     * Em multi-janela a Activity continua visível, e aí não se trata como
     * escondida.
     */
    @Override
    protected void onPause() {
        if (!isInMultiWindowMode()) {
            nativeTrimMemory(ComponentCallbacks2.TRIM_MEMORY_UI_HIDDEN);
            if (webEmbedView != null) {
                webEmbedView.onPause();
                if (webEmbedLayer != null) {
                    webEmbedLayer.setVisibility(View.GONE);
                }
            }
        }
        super.onPause();
    }

    @Override
    protected void onStart() {
        super.onStart();
        nativeLifecycleChanged(true);
        if (webEmbedView != null) {
            webEmbedView.onResume();
        }
    }

    @Override
    protected void onStop() {
        nativeLifecycleChanged(false);
        if (webEmbedView != null) {
            webEmbedView.onPause();
            if (webEmbedLayer != null) {
                webEmbedLayer.setVisibility(View.GONE);
            }
        }
        super.onStop();
    }

    /**
     * Pressão de memória do sistema.
     *
     * O Rust decide a política: aqui só se repassa o inteiro do Android, numa
     * thread que não pode tocar no MediaStore. O latch coalesce e a thread
     * normal do Papo aplica no quadro seguinte.
     *
     * Do Android 14 em diante só chegam UI_HIDDEN e BACKGROUND (e acima); os
     * RUNNING_* antigos deixaram de ser enviados de propósito — não são
     * esperados em aparelho novo. O `onLowMemory` não é usado: o sistema
     * moderno não o entrega de forma útil e o `onTrimMemory` cobre o caso.
     */
    @Override
    public void onTrimMemory(int level) {
        super.onTrimMemory(level);
        nativeTrimMemory(level);
    }

    @Override
    public void onPictureInPictureModeChanged(
            boolean isInPictureInPictureMode,
            Configuration newConfig) {
        super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig);
        ensurePipLayer();
        pipLayer.setVisibility(isInPictureInPictureMode ? View.VISIBLE : View.GONE);
        if (isInPictureInPictureMode) {
            pipLayer.bringToFront();
        }
        nativeSetPictureInPictureMode(isInPictureInPictureMode);
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
