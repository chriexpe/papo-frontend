package io.github.chriexpe.papo.compose

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.RectF
import android.graphics.SurfaceTexture
import android.view.TextureView
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import io.github.chriexpe.papo.core.CallVideoFrame
import java.nio.ByteBuffer
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive

/**
 * Latest-frame rendering has its own lifecycle and never repolls chat/message JSON at video rate.
 */
@Composable
internal fun CallVideoImage(
    user: String,
    screen: Boolean,
    modifier: Modifier,
    crop: Boolean = false,
) {
    val calls = LocalVoiceCall.current
    val session = calls?.state?.session
    if (calls == null || session == null || session.demo) {
        ManagedAsyncImage(
            if (screen) "file:///android_asset/demo/landscape.png"
            else "file:///android_asset/demo/portrait.png",
            null,
            MediaImages.loader(LocalContext.current),
            modifier,
            if (crop) ContentScale.Crop else ContentScale.Fit,
        )
        return
    }
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val view =
        remember(session.route, session.sessionId, user, screen) { CallTextureView(context, crop) }
    var received by remember(view) { mutableStateOf(false) }
    LaunchedEffect(view, lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            var after = 0uL
            while (isActive) {
                val frame =
                    calls.model.callVideoFrame(
                        session,
                        user,
                        if (screen) "screen" else "video",
                        after,
                    )
                if (frame != null && view.render(frame)) {
                    after = frame.sequence
                    received = true
                }
                delay(66)
            }
        }
    }
    DisposableEffect(view) { onDispose { view.release() } }
    Box(modifier, contentAlignment = Alignment.Center) {
        AndroidView(factory = { view }, modifier = Modifier.fillMaxSize())
        if (!received) CircularProgressIndicator()
    }
}

/** A reusable bitmap and canvas avoid allocating a new Compose image/texture every frame. */
private class CallTextureView(context: Context, private val crop: Boolean) :
    TextureView(context), TextureView.SurfaceTextureListener {
    private val gate = Any()
    private var bitmap: Bitmap? = null
    private var released = false

    init {
        surfaceTextureListener = this
    }

    fun render(frame: CallVideoFrame) =
        synchronized(gate) {
            if (released || !isAvailable || frame.width == 0u || frame.height == 0u)
                return@synchronized false
            val width = frame.width.toInt()
            val height = frame.height.toInt()
            if (frame.rgba.size.toLong() != width.toLong() * height * 4) return@synchronized false
            if (bitmap?.width != width || bitmap?.height != height) {
                bitmap?.recycle()
                bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
            }
            val image = bitmap ?: return@synchronized false
            image.copyPixelsFromBuffer(ByteBuffer.wrap(frame.rgba))
            val canvas = lockCanvas() ?: return@synchronized false
            try {
                canvas.drawColor(Color.BLACK)
                val xScale = canvas.width.toFloat() / width
                val yScale = canvas.height.toFloat() / height
                val scale = if (crop) maxOf(xScale, yScale) else minOf(xScale, yScale)
                val drawWidth = width * scale
                val drawHeight = height * scale
                val x = (canvas.width - drawWidth) / 2
                val y = (canvas.height - drawHeight) / 2
                canvas.drawBitmap(image, null, RectF(x, y, x + drawWidth, y + drawHeight), null)
            } finally {
                unlockCanvasAndPost(canvas)
            }
            true
        }

    fun release() =
        synchronized(gate) {
            released = true
            bitmap?.recycle()
            bitmap = null
        }

    override fun onSurfaceTextureAvailable(surface: SurfaceTexture, width: Int, height: Int) = Unit

    override fun onSurfaceTextureSizeChanged(surface: SurfaceTexture, width: Int, height: Int) =
        Unit

    override fun onSurfaceTextureUpdated(surface: SurfaceTexture) = Unit

    override fun onSurfaceTextureDestroyed(surface: SurfaceTexture): Boolean =
        synchronized(gate) {
            bitmap?.recycle()
            bitmap = null
            true
        }
}
