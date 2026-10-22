package io.github.chriexpe.papo.compose

import android.content.Context
import coil3.ImageLoader
import coil3.decode.DataSource
import coil3.decode.ImageSource
import coil3.fetch.Fetcher
import coil3.fetch.SourceFetchResult
import coil3.gif.AnimatedImageDecoder
import coil3.key.Keyer
import coil3.memory.MemoryCache
import coil3.request.Options
import okio.Buffer

/**
 * Stable resource identities let the shared decoder find an existing bitmap before doing
 * disk/native work. Encoded bytes still live only in MediaCache.
 */
internal class CachedImage(val identity: String, val load: suspend () -> ByteArray) {
    override fun equals(other: Any?) = other is CachedImage && identity == other.identity

    override fun hashCode() = identity.hashCode()
}

private class CachedImageKeyer : Keyer<CachedImage> {
    override fun key(data: CachedImage, options: Options) = data.identity
}

private class CachedImageFetcher : Fetcher.Factory<CachedImage> {
    override fun create(data: CachedImage, options: Options, imageLoader: ImageLoader) =
        object : Fetcher {
            override suspend fun fetch() =
                SourceFetchResult(
                    ImageSource(Buffer().write(data.load()), options.fileSystem),
                    null,
                    DataSource.DISK,
                )
        }
}

/**
 * One decoder and a bounded cache for shareable, immutable bitmaps. Coil never caches non-shareable
 * animated drawables, so their clocks remain independent.
 */
internal object MediaImages {
    @Volatile private var shared: ImageLoader? = null

    @Synchronized
    fun loader(context: Context): ImageLoader =
        shared
            ?: ImageLoader.Builder(context.applicationContext)
                .components {
                    add(CachedImageKeyer())
                    add(CachedImageFetcher())
                    add(AnimatedImageDecoder.Factory())
                }
                .memoryCache {
                    MemoryCache.Builder()
                        .maxSizeBytes(24L * 1024 * 1024)
                        .weakReferencesEnabled(false)
                        .build()
                }
                .diskCache(null)
                .build()
                .also { shared = it }
}
