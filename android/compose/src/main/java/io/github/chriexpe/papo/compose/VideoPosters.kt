package io.github.chriexpe.papo.compose

import android.graphics.Bitmap
import android.media.MediaDataSource
import android.media.MediaMetadataRetriever
import android.os.SystemClock
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import coil3.request.ImageRequest
import io.github.chriexpe.papo.core.EmbedView
import io.github.chriexpe.papo.core.MediaRequest
import io.github.chriexpe.papo.core.MediaResource
import java.io.ByteArrayOutputStream
import java.io.IOException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext

/** A frame from the video cannot accidentally turn a provider's profile artwork into its cover. */
internal object VideoPosters {
    private val decoders = Semaphore(2)

    suspend fun load(cache: MediaCache, source: MediaRequest, revision: String = ""): ByteArray =
        cache.blob(
            cache.derivedIdentity(source, "video-poster-v2/$revision"),
            limit = 2 * 1024 * 1024,
        ) {
            decoders.withPermit {
                withContext(Dispatchers.IO) {
                    currentCoroutineContext().ensureActive()
                    val work = currentCoroutineContext()
                    val started = SystemClock.elapsedRealtime()
                    var read = 0
                    val original = cache.extractorSource(source)
                    val bounded =
                        object : MediaDataSource() {
                            override fun getSize() = original.size

                            override fun close() = original.close()

                            override fun readAt(
                                position: Long,
                                buffer: ByteArray,
                                offset: Int,
                                size: Int,
                            ): Int {
                                work.ensureActive()
                                if (
                                    SystemClock.elapsedRealtime() - started > 15_000 ||
                                        read >= 8 * 1024 * 1024
                                )
                                    throw IOException("Video poster read budget exceeded")
                                val count =
                                    original.readAt(
                                        position,
                                        buffer,
                                        offset,
                                        minOf(size, 8 * 1024 * 1024 - read),
                                    )
                                if (count > 0) read += count
                                return count
                            }
                        }
                    bounded.use { data ->
                        val retriever = MediaMetadataRetriever()
                        try {
                            retriever.setDataSource(data)
                            val width =
                                retriever
                                    .extractMetadata(
                                        MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH
                                    )
                                    ?.toIntOrNull() ?: 640
                            val height =
                                retriever
                                    .extractMetadata(
                                        MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT
                                    )
                                    ?.toIntOrNull() ?: 360
                            val scale = minOf(1f, 720f / maxOf(width, height).coerceAtLeast(1))
                            val durationMs =
                                retriever
                                    .extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)
                                    ?.toLongOrNull() ?: 0
                            val sampleUs = minOf(1_000L, durationMs / 3) * 1_000L
                            val frame =
                                requireNotNull(
                                    retriever.getScaledFrameAtTime(
                                        sampleUs,
                                        MediaMetadataRetriever.OPTION_CLOSEST,
                                        (width * scale).toInt().coerceAtLeast(1),
                                        (height * scale).toInt().coerceAtLeast(1),
                                    )
                                )
                            try {
                                currentCoroutineContext().ensureActive()
                                ByteArrayOutputStream()
                                    .also {
                                        check(frame.compress(Bitmap.CompressFormat.JPEG, 85, it))
                                    }
                                    .toByteArray()
                            } finally {
                                frame.recycle()
                            }
                        } finally {
                            retriever.release()
                        }
                    }
                }
            }
        }
}

@Composable
internal fun BoxScope.VideoPoster(
    server: ServerState,
    model: PapoViewModel,
    resource: MediaResource?,
    publicUrl: String,
    embed: EmbedView,
) {
    val context = LocalContext.current
    val activity = rememberMediaActivity()
    var failed by
        remember(server.url, server.me, embed.id, embed.revision) { mutableStateOf(false) }
    val bytes by
        produceState<ByteArray?>(
            null,
            server.url,
            server.me,
            embed.id,
            embed.revision,
            resource,
            publicUrl,
            activity.retained,
        ) {
            if (activity.retained && value == null && !failed) {
                try {
                    val source =
                        if (publicUrl.isNotBlank()) model.publicMediaRequest(server, publicUrl)
                        else model.mediaRequest(server.url, requireNotNull(resource))
                    value = VideoPosters.load(MediaCache.get(context), source, embed.revision)
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    failed = true
                }
            }
        }
    Box(
        Modifier.fillMaxSize()
            .background(MaterialTheme.colorScheme.surfaceContainerHighest)
            .then(activity.modifier)
    ) {
        if (bytes != null && !failed) {
            var decoded by remember(bytes) { mutableStateOf(false) }
            val request = remember(bytes) { ImageRequest.Builder(context).data(bytes).build() }
            ManagedAsyncImage(
                request,
                embed.title,
                imageLoader = MediaImages.loader(context),
                contentScale = ContentScale.Fit,
                modifier =
                    Modifier.fillMaxSize().testTag("video-poster-${embed.id}").semantics {
                        stateDescription =
                            if (decoded) ui(R.string.loaded) else ui(R.string.loading)
                    },
                onSuccess = { decoded = true },
                onError = { failed = true },
            )
        } else if (failed && embed.hasImage) {
            MediaImage(server, model, embed.title, embed = embed, viewer = false)
        }
    }
}
