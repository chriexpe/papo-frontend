package io.github.chriexpe.papo;

import android.net.Uri;
import android.webkit.*;
import androidx.webkit.ProfileStore;
import androidx.webkit.WebViewCompat;
import androidx.webkit.WebViewFeature;
import java.io.*;
import java.net.HttpURLConnection;
import java.net.URL;
import java.util.*;

/** Isolated app policy. No JS bridge, native credentials, or default profile. */
final class AppSurfaceClient extends WebViewClient {
    interface Events { void emit(int kind, String url); }
    private static final String PREFIX = "papo-app-";
    private final String entry;
    private final boolean bundled;
    private final Events events;
    private final String profile;
    private volatile boolean closed;
    private volatile boolean failed;

    private AppSurfaceClient(String entry, boolean bundled, Events events, String profile) {
        this.entry = entry; this.bundled = bundled; this.events = events; this.profile = profile;
    }
    static AppSurfaceClient attach(WebView view, String entry, boolean bundled, Events events) {
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)) {
            throw new UnsupportedOperationException("MULTI_PROFILE unavailable");
        }
        // Never load Profile objects with getProfile/getOrCreateProfile: those
        // retain profiles in memory and make deletion illegal until process exit.
        ProfileStore store = ProfileStore.getInstance();
        for (String name : store.getAllProfileNames()) {
            if (name.startsWith(PREFIX)) store.deleteProfile(name);
        }
        String profile = PREFIX + UUID.randomUUID();
        WebViewCompat.setProfile(view, profile); // before any load or settings access
        AppSurfaceClient client = new AppSurfaceClient(entry, bundled, events, profile);
        WebSettings settings = view.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setCacheMode(WebSettings.LOAD_NO_CACHE);
        settings.setAllowFileAccess(false);
        settings.setAllowContentAccess(false);
        settings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        settings.setGeolocationEnabled(false);
        settings.setSupportMultipleWindows(true);
        settings.setJavaScriptCanOpenWindowsAutomatically(false);
        settings.setMediaPlaybackRequiresUserGesture(true);
        view.setSaveEnabled(false);
        view.setWebViewClient(client);
        view.setDownloadListener((url, ua, cd, mime, length) -> {});
        view.setWebChromeClient(new WebChromeClient() {
            @Override public void onPermissionRequest(PermissionRequest request) { request.deny(); }
            @Override public void onGeolocationPermissionsShowPrompt(String origin, GeolocationPermissions.Callback callback) {
                callback.invoke(origin, false, false);
            }
            @Override public boolean onShowFileChooser(WebView webView, android.webkit.ValueCallback<Uri[]> callback, FileChooserParams params) {
                callback.onReceiveValue(null); return true;
            }
            @Override public boolean onCreateWindow(WebView view, boolean dialog, boolean gesture, android.os.Message result) { return false; }
        });
        return client;
    }
    // Called only after the associated WebView has been destroyed, on the UI thread.
    void destroy() {
        closed = true;
        ProfileStore.getInstance().deleteProfile(profile);
    }
    private void emit(int kind, String url) { if (!closed) events.emit(kind, url); }
    private void fail() { failed = true; emit(2, ""); }
    static boolean sameOrigin(String left, String right) {
        Uri a = Uri.parse(left), b = Uri.parse(right);
        if (a.getHost() == null || b.getHost() == null || a.getScheme() == null || b.getScheme() == null) return false;
        int ap = a.getPort() < 0 ? ("https".equalsIgnoreCase(a.getScheme()) ? 443 : 80) : a.getPort();
        int bp = b.getPort() < 0 ? ("https".equalsIgnoreCase(b.getScheme()) ? 443 : 80) : b.getPort();
        return a.getScheme().equalsIgnoreCase(b.getScheme()) && a.getHost().equalsIgnoreCase(b.getHost()) && ap == bp;
    }
    private boolean allowedNavigation(String url) {
        if (!sameOrigin(entry, url)) return false;
        if (!bundled) return true;
        String path = Uri.parse(entry).getPath();
        int end = path.indexOf("/ui/");
        String target = Uri.parse(url).getPath();
        return end >= 0 && target != null && target.startsWith(path.substring(0, end + 4))
            && !Arrays.asList(target.split("/")).contains("..")
            && !Arrays.asList(target.split("/")).contains(".");
    }
    @Override public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
        String url = request.getUrl().toString();
        if (!allowedNavigation(url)) {
            if ("https".equalsIgnoreCase(request.getUrl().getScheme()) || "http".equalsIgnoreCase(request.getUrl().getScheme())) emit(1, url);
            return true;
        }
        return false;
    }
    private static WebResourceResponse blocked() {
        return new WebResourceResponse("text/plain", "UTF-8", 403, "Blocked", Collections.emptyMap(), new ByteArrayInputStream(new byte[0]));
    }
    @Override public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
        if (closed || failed) return blocked();
        String url = request.getUrl().toString();
        // Intercept also covers POST/redirect paths omitted by shouldOverride.
        // Conservatively reject cross-origin resource requests as well.
        if (!sameOrigin(entry, url)) { if (request.isForMainFrame()) fail(); return blocked(); }
        if (!request.isForMainFrame()) return null;
        if (!allowedNavigation(url)) { fail(); return blocked(); }
        HttpURLConnection connection = null;
        try {
            if (!bundled) {
                AppSurfacePolicy.preflight(url);
                return null;
            }
            // WebView exposes no response-header callback. For bundled main
            // documents return the actual credential-less response after checking
            // its sandbox. Subresources remain ordinary isolated WebView requests.
            connection = AppSurfacePolicy.connect(url, "GET");
            AppSurfacePolicy.checkStatus(connection);
            if (!AppSurfacePolicy.sandboxed(AppSurfacePolicy.header(connection, "Content-Security-Policy"))) throw new IOException("Missing sandbox");
            Map<String, String> headers = new HashMap<>();
            for (Map.Entry<String, List<String>> h : connection.getHeaderFields().entrySet()) {
                if (h.getKey() != null) headers.put(h.getKey(), String.join(", ", h.getValue()));
            }
            HttpURLConnection owner = connection;
            InputStream stream = new FilterInputStream(connection.getInputStream()) {
                @Override public void close() throws IOException { try { super.close(); } finally { owner.disconnect(); } }
            };
            String contentType = AppSurfacePolicy.header(connection, "Content-Type");
            connection = null; // response stream now owns the connection
            return new WebResourceResponse(contentType.split(";")[0], "UTF-8", 200, "OK", headers, stream);
        } catch (Exception error) {
            fail(); return blocked();
        } finally { if (connection != null) connection.disconnect(); }
    }
    @Override public void onPageFinished(WebView view, String url) {
        if (!failed) emit(3, "");
    }
    @Override public void onReceivedError(WebView view, WebResourceRequest request, WebResourceError error) { if (request.isForMainFrame()) fail(); }
    @Override public void onReceivedHttpError(WebView view, WebResourceRequest request, WebResourceResponse response) { if (request.isForMainFrame()) fail(); }
    @Override public void onReceivedSslError(WebView view, android.webkit.SslErrorHandler handler, android.net.http.SslError error) { handler.cancel(); fail(); }
    @Override public void onReceivedHttpAuthRequest(WebView view, HttpAuthHandler handler, String host, String realm) { handler.cancel(); fail(); }
    @Override public boolean onRenderProcessGone(WebView view, RenderProcessGoneDetail detail) { fail(); return true; }
}
