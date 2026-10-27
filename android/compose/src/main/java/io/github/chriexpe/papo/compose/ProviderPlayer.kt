package io.github.chriexpe.papo.compose

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.ActivityInfo
import android.graphics.Color
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner

@Composable
internal fun ProviderPlayer(url: String, onPlayer: (String?) -> Unit) {
    val activity = rememberMediaActivity()
    Box(Modifier.fillMaxSize().then(activity.modifier)) {
        if (activity.retained) ProviderPlayerContent(url, onPlayer, activity.visible)
    }
}

/**
 * Own the complete gesture so timeline scrolling, reply swipes and long presses cannot steal it.
 */
internal class ProviderWebView(context: Context) : WebView(context) {
    init {
        // AndroidView otherwise supplies WRAP_CONTENT. Chromium treats that as an unbounded
        // document: CSS viewport/percentage heights (including YouTube's video) collapse to 0.
        layoutParams =
            ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
        setBackgroundColor(Color.BLACK)
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN)
            parent?.requestDisallowInterceptTouchEvent(true)
        val handled = super.onTouchEvent(event)
        if (
            event.actionMasked == MotionEvent.ACTION_UP ||
                event.actionMasked == MotionEvent.ACTION_CANCEL
        )
            parent?.requestDisallowInterceptTouchEvent(false)
        return handled
    }
}

private tailrec fun Context.providerActivity(): Activity? =
    when (this) {
        is Activity -> this
        is ContextWrapper -> baseContext.providerActivity()
        else -> null
    }

/** Chromium supplies the fullscreen view; keep its WebView and playback session alive inline. */
internal class ProviderFullscreen(private val activity: Activity) : WebChromeClient() {
    var showing by mutableStateOf(false)
        private set

    private var layer: FrameLayout? = null
    private var callback: CustomViewCallback? = null
    private var previousOrientation = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
    private var previousBars = 0
    private var previousBehavior = 0

    override fun onShowCustomView(view: View, callback: CustomViewCallback) {
        if (showing) {
            callback.onCustomViewHidden()
            return
        }
        val decor = activity.window.decorView as ViewGroup
        val controller = WindowCompat.getInsetsController(activity.window, decor)
        val insets = ViewCompat.getRootWindowInsets(decor)
        previousBars = 0
        for (type in
            listOf(
                WindowInsetsCompat.Type.statusBars(),
                WindowInsetsCompat.Type.navigationBars(),
            )) {
            if (insets?.isVisible(type) != false) previousBars = previousBars or type
        }
        previousBehavior = controller.systemBarsBehavior
        previousOrientation = activity.requestedOrientation
        this.callback = callback
        layer =
            FrameLayout(activity).apply {
                setBackgroundColor(Color.BLACK)
                keepScreenOn = true
                addView(
                    view,
                    FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT,
                    ),
                )
                decor.addView(
                    this,
                    ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT,
                    ),
                )
            }
        showing = true
        activity.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR
        controller.systemBarsBehavior =
            WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        controller.hide(WindowInsetsCompat.Type.systemBars())
    }

    override fun onHideCustomView() = hide()

    fun hide() {
        val oldLayer = layer ?: return
        val oldCallback = callback
        layer = null
        callback = null
        showing = false
        oldLayer.removeAllViews()
        (oldLayer.parent as? ViewGroup)?.removeView(oldLayer)
        activity.requestedOrientation = previousOrientation
        WindowCompat.getInsetsController(activity.window, activity.window.decorView).apply {
            systemBarsBehavior = previousBehavior
            hide(WindowInsetsCompat.Type.systemBars() and previousBars.inv())
            show(previousBars)
        }
        oldCallback?.onCustomViewHidden()
    }
}

@Suppress("SetJavaScriptEnabled")
@Composable
private fun ProviderPlayerContent(url: String, onPlayer: (String?) -> Unit, visible: Boolean) {
    val context = LocalContext.current
    val externalLinks by rememberUpdatedState(LocalUriHandler.current)
    val stopPlayer by rememberUpdatedState(onPlayer)
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val activity = remember(context) { requireNotNull(context.providerActivity()) }
    val fullscreen = remember(url, activity) { ProviderFullscreen(activity) }
    val webView =
        remember(url, context) {
            ProviderWebView(context).apply {
                settings.javaScriptEnabled = true
                settings.domStorageEnabled = true
                settings.allowFileAccess = false
                settings.allowContentAccess = false
                settings.mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
                settings.mediaPlaybackRequiresUserGesture = true
                settings.setGeolocationEnabled(false)
                CookieManager.getInstance().setAcceptThirdPartyCookies(this, false)
                webChromeClient = fullscreen
                webViewClient =
                    object : WebViewClient() {
                        override fun shouldOverrideUrlLoading(
                            view: WebView?,
                            request: WebResourceRequest?,
                        ): Boolean {
                            val target = request?.url ?: return true
                            if (!request.isForMainFrame) return false
                            if (
                                target.scheme == "https" &&
                                    target.host in
                                        listOf("www.youtube.com", "www.youtube-nocookie.com")
                            )
                                return false
                            if (isPublicLink(target.toString()))
                                runCatching { externalLinks.openUri(target.toString()) }
                            return true
                        }
                    }
                // Identify the Android app to YouTube; never send the Papo server/session identity.
                loadUrl(url, mapOf("Referer" to "https://${context.packageName}/"))
            }
        }
    BackHandler(enabled = fullscreen.showing) { fullscreen.hide() }
    val registry = LocalMediaVisibilityRegistry.current
    val row = LocalMediaRowId.current
    val owner = remember { Any() }
    DisposableEffect(webView, registry, row) {
        registry?.register(owner, row) {
            fullscreen.hide()
            pauseProviderMedia(webView)
        }
        onDispose { registry?.unregister(owner) }
    }
    DisposableEffect(webView, lifecycle) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_PAUSE) {
                fullscreen.hide()
                pauseProviderMedia(webView)
                stopPlayer(null)
            }
        }
        lifecycle.addObserver(observer)
        onDispose {
            lifecycle.removeObserver(observer)
            fullscreen.hide()
            webView.stopLoading()
            webView.onPause()
            webView.destroy()
        }
    }
    LaunchedEffect(webView, visible) {
        if (visible) webView.onResume()
        else {
            fullscreen.hide()
            pauseProviderMedia(webView)
        }
    }
    AndroidView(factory = { webView }, modifier = Modifier.fillMaxSize())
}

private fun pauseProviderMedia(webView: WebView) {
    webView.evaluateJavascript(
        "document.querySelectorAll('video,audio').forEach(function(media){media.pause();});",
        null,
    )
    webView.onPause()
}
