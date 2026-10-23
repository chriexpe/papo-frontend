@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package io.github.chriexpe.papo.compose

import android.content.Context
import android.media.MediaDataSource
import android.net.Uri
import androidx.media3.common.C
import androidx.media3.database.StandaloneDatabaseProvider
import androidx.media3.datasource.ByteArrayDataSource
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DataSourceInputStream
import androidx.media3.datasource.DataSpec
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.ResolvingDataSource
import androidx.media3.datasource.cache.CacheDataSink
import androidx.media3.datasource.cache.CacheDataSource
import androidx.media3.datasource.cache.CacheWriter
import androidx.media3.datasource.cache.ContentMetadata
import androidx.media3.datasource.cache.ContentMetadataMutations
import androidx.media3.datasource.cache.LeastRecentlyUsedCacheEvictor
import androidx.media3.datasource.cache.SimpleCache
import androidx.media3.datasource.okhttp.OkHttpDataSource
import io.github.chriexpe.papo.core.MediaRequest
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import okhttp3.Request

/**
 * One encoded-media store and one LRU budget for images, avatars, stickers, audio, video ranges and
 * derived waveforms. Decoding remains with Coil/Media3. Public images use stale-while-revalidate;
 * immutable core blobs use revisions.
 */
internal class MediaCache
private constructor(
    private val context: Context,
    directory: File,
    budget: Long,
    private val now: () -> Long,
    private val freshFor: Long,
) : java.io.Closeable {
    private val cache =
        SimpleCache(
            directory,
            LeastRecentlyUsedCacheEvictor(budget),
            StandaloneDatabaseProvider(context),
        )
    private val locks = ConcurrentHashMap<String, Mutex>()

    private fun hash(key: String) =
        MessageDigest.getInstance("SHA-256").digest(key.toByteArray()).joinToString("") {
            "%02x".format(it)
        }

    private val generations =
        context.getSharedPreferences("compose-media-scopes", Context.MODE_PRIVATE)
    private val readers = ConcurrentHashMap<String, MutableSet<DataSource>>()
    // Publish revocation atomically with blob writes and streaming-reader registration.
    private val revocations = Any()

    internal fun allowServer(server: String) {
        generations.edit().remove("blocked/${hash(server)}").commit()
    }

    private fun scopedIdentity(identity: String, server: String): String {
        val scope = hash(server)
        check(!generations.getBoolean("blocked/$scope", false)) { ui(R.string.server_is_closed) }
        return "papo:$scope:${generations.getLong("epoch/$scope", 0)}:$identity"
    }

    internal fun scoped(source: MediaRequest, server: String) =
        source.copy(cacheKey = scopedIdentity(source.cacheKey, server))

    internal fun derivedIdentity(source: MediaRequest, kind: String) = "${source.cacheKey}/$kind"

    private fun storageKey(identity: String): String {
        val parts = identity.split(':', limit = 4)
        return if (parts.size == 4 && parts[0] == "papo")
            "${parts[0]}:${parts[1]}:${parts[2]}:${hash(parts[3])}"
        else hash(identity)
    }

    private fun allowed(key: String): Boolean {
        val parts = key.split(':', limit = 4)
        return parts.size != 4 ||
            parts[0] != "papo" ||
            (!generations.getBoolean("blocked/${parts[1]}", false) &&
                parts[2].toLongOrNull() == generations.getLong("epoch/${parts[1]}", 0))
    }

    private fun checkAllowed(key: String) {
        check(allowed(key)) { ui(R.string.server_is_closed) }
    }

    private fun key(source: MediaRequest) = storageKey(source.cacheKey)

    /** Revoke before closing readers: late image loads and range writers cannot repopulate it. */
    internal suspend fun forgetServer(server: String) =
        withContext(Dispatchers.IO) {
            val scope = hash(server)
            val activeReaders =
                synchronized(revocations) {
                    check(
                        generations
                            .edit()
                            .putBoolean("blocked/$scope", true)
                            .putLong("epoch/$scope", generations.getLong("epoch/$scope", 0) + 1)
                            .commit()
                    )
                    readers[scope]?.toList().orEmpty()
                }
            activeReaders.forEach { runCatching { it.close() } }
            cache.keys
                .toList()
                .filter { it.startsWith("papo:$scope:") }
                .forEach { key ->
                    locks.getOrPut(key) { Mutex() }.withLock { cache.removeResource(key) }
                }
            // v1 hashes had no owner. Discard the old disposable cache rather than
            // retain untraceable media after promising a complete server removal.
            context.cacheDir.resolve("media-cache-v1").deleteRecursively()
        }

    private fun spec(key: String, uri: String = "cache://media") =
        DataSpec.Builder().setUri(uri).setKey(key).build()

    private fun readOnly() =
        CacheDataSource.Factory().setCache(cache).setFlags(CacheDataSource.FLAG_BLOCK_ON_CACHE)

    private fun peek(key: String, limit: Int = MAX_IMAGE): ByteArray? {
        val length = ContentMetadata.getContentLength(cache.getContentMetadata(key))
        if (length !in 1..limit.toLong() || !cache.isCached(key, 0, length)) return null
        return runCatching {
            DataSourceInputStream(readOnly().createDataSource(), spec(key)).use {
                readBounded(it, limit)
            }
        }
            .getOrNull()
    }

    private fun store(key: String, bytes: ByteArray) =
        synchronized(revocations) {
            checkAllowed(key)
            require(bytes.isNotEmpty()) { ui(R.string.empty_media) }
            cache.removeResource(key)
            val factory =
                CacheDataSource.Factory()
                    .setCache(cache)
                    .setFlags(CacheDataSource.FLAG_BLOCK_ON_CACHE)
                    .setUpstreamDataSourceFactory { ByteArrayDataSource(bytes) }
            CacheWriter(factory.createDataSource(), spec(key), null, null).cache()
        }

    suspend fun blob(
        identity: String,
        limit: Int = MAX_IMAGE,
        server: String = "",
        load: suspend () -> ByteArray,
    ): ByteArray {
        val key = storageKey(if (server.isEmpty()) identity else scopedIdentity(identity, server))
        return withContext(Dispatchers.IO) {
            checkAllowed(key)
            peek(key, limit)
                ?: locks
                    .getOrPut(key) { Mutex() }
                    .withLock {
                        peek(key, limit)
                            ?: load().also {
                                require(it.size <= limit) { ui(R.string.media_is_too_large) }
                                currentCoroutineContext().ensureActive()
                                store(key, it)
                            }
                    }
        }
    }

    fun images(source: MediaRequest): Flow<ByteArray> = flow {
        val key = key(source)
        checkAllowed(key)
        val cached = peek(key)
        cached?.let { emit(it) }
        val mutable =
            source.headers.isEmpty() && source.origin != "demo" && source.url.startsWith("http")
        val checked = cache.getContentMetadata(key).get(CHECKED, 0L)
        if (cached != null && (!mutable || now() - checked in 0 until freshFor)) return@flow
        val bytes =
            try {
                locks
                    .getOrPut(key) { Mutex() }
                    .withLock {
                        val latest = peek(key)
                        val metadata = cache.getContentMetadata(key)
                        if (
                            latest != null &&
                                (!mutable || now() - metadata.get(CHECKED, 0L) in 0 until freshFor)
                        )
                            latest
                        else if (mutable) {
                            val request =
                                Request.Builder()
                                    .url(source.url)
                                    .apply {
                                        metadata
                                            .get(ETAG, "")
                                            .orEmpty()
                                            .takeIf { it.isNotBlank() }
                                            ?.let { header("If-None-Match", it) }
                                        metadata
                                            .get(MODIFIED, "")
                                            .orEmpty()
                                            .takeIf { it.isNotBlank() }
                                            ?.let { header("If-Modified-Since", it) }
                                    }
                                    .build()
                            cancellableRequest(source, request) { response ->
                                val data =
                                    if (response.code == 304 && latest != null) latest
                                    else {
                                        if (!response.isSuccessful)
                                            throw IOException(
                                                ui(R.string.media_unavailable_1_s, response.code)
                                            )
                                        val body =
                                            response.body
                                                ?: throw IOException(ui(R.string.empty_media))
                                        if (body.contentLength() > MAX_IMAGE)
                                            throw IOException(ui(R.string.image_is_too_large))
                                        body.byteStream().use { readBounded(it, MAX_IMAGE) }
                                    }
                                currentCoroutineContext().ensureActive()
                                if (latest == null || !latest.contentEquals(data)) store(key, data)
                                cache.applyContentMetadataMutations(
                                    key,
                                    ContentMetadataMutations()
                                        .set(CHECKED, now())
                                        .set(
                                            ETAG,
                                            response.header("ETag")
                                                ?: metadata.get(ETAG, "").orEmpty(),
                                        )
                                        .set(
                                            MODIFIED,
                                            response.header("Last-Modified")
                                                ?: metadata.get(MODIFIED, "").orEmpty(),
                                        ),
                                )
                                data
                            }
                        } else
                            DataSourceInputStream(
                                    factory(source).createDataSource(),
                                    spec(key, source.url),
                                )
                                .use { readBounded(it, MAX_IMAGE) }
                    }
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                if (cached == null) throw e else null
            }
        checkAllowed(key)
        if (bytes != null && (cached == null || !cached.contentEquals(bytes))) emit(bytes)
    }
        .flowOn(Dispatchers.IO)

    private suspend fun <T> cancellableRequest(
        source: MediaRequest,
        request: Request,
        read: suspend (okhttp3.Response) -> T,
    ): T =
        kotlinx.coroutines.coroutineScope {
            val call = MediaNetwork.client(source).newCall(request)
            val watcher =
                launch(start = kotlinx.coroutines.CoroutineStart.UNDISPATCHED) {
                    try {
                        kotlinx.coroutines.awaitCancellation()
                    } finally {
                        call.cancel()
                    }
                }
            try {
                call.execute().use { read(it) }
            } finally {
                watcher.cancel()
            }
        }

    /**
     * A streaming player and an image/download of the same source use the same key. Segmented
     * providers get a distinct key for every child URI.
     */
    fun factory(source: MediaRequest): DataSource.Factory {
        val upstream =
            DefaultDataSource.Factory(
                context,
                OkHttpDataSource.Factory(MediaNetwork.client(source)),
            )
        val factory =
            CacheDataSource.Factory()
                .setCache(cache)
                .setUpstreamDataSourceFactory(upstream)
                .setCacheWriteDataSinkFactory(
                    CacheDataSink.Factory().setCache(cache).setFragmentSize(1024 * 1024L)
                )
                .setFlags(CacheDataSource.FLAG_BLOCK_ON_CACHE)
                .setCacheKeyFactory { data ->
                    if (data.key != null) data.key!!
                    else if (data.uri.toString() == source.url) key(source)
                    else storageKey("${source.cacheKey}/${data.uri}")
                }
        val cached =
            ResolvingDataSource.Factory(factory) { data ->
                data
                    .buildUpon()
                    .setFlags(data.flags or DataSpec.FLAG_ALLOW_CACHE_FRAGMENTATION)
                    .build()
            }
        return DataSource.Factory {
            object : DataSource {
                private var delegate: DataSource? = null
                private val listeners = mutableListOf<androidx.media3.datasource.TransferListener>()

                override fun addTransferListener(
                    listener: androidx.media3.datasource.TransferListener
                ) {
                    listeners += listener
                    delegate?.addTransferListener(listener)
                }

                @Synchronized
                override fun open(data: DataSpec): Long {
                    synchronized(revocations) {
                        checkAllowed(key(source))
                        val scope = key(source).split(':').getOrNull(1).orEmpty()
                        readers.getOrPut(scope) { ConcurrentHashMap.newKeySet() }.add(this)
                    }
                    // Live playlists change while their segment URLs identify media.
                    // Never freeze a playlist in the disk cache.
                    val path = data.uri.path.orEmpty().lowercase()
                    delegate =
                        (if (path.endsWith(".m3u8") || path.endsWith(".mpd")) upstream else cached)
                            .createDataSource()
                    listeners.forEach { delegate!!.addTransferListener(it) }
                    return delegate!!.open(data)
                }

                @Synchronized
                override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
                    checkAllowed(key(source))
                    return requireNotNull(delegate).read(buffer, offset, length)
                }

                override fun getUri(): Uri? = delegate?.uri

                override fun getResponseHeaders(): Map<String, List<String>> =
                    delegate?.responseHeaders.orEmpty()

                @Synchronized
                override fun close() {
                    try {
                        delegate?.close()
                    } finally {
                        delegate = null
                        val scope = key(source).split(':').getOrNull(1).orEmpty()
                        readers[scope]?.remove(this)
                        if (!allowed(key(source)))
                            cache.keys
                                .toList()
                                .filter { it.startsWith("papo:$scope:") && !allowed(it) }
                                .forEach(cache::removeResource)
                    }
                }
            }
        }
    }

    suspend fun export(source: MediaRequest, target: File) =
        withContext(Dispatchers.IO) {
            val work = currentCoroutineContext()
            DataSourceInputStream(factory(source).createDataSource(), spec(key(source), source.url))
                .use { input ->
                    target.outputStream().use { output ->
                        val buffer = ByteArray(64 * 1024)
                        while (true) {
                            work.ensureActive()
                            val count = input.read(buffer)
                            if (count < 0) break
                            output.write(buffer, 0, count)
                        }
                    }
                }
        }

    /**
     * Native waveform decoding seeks through the same range cache as playback; it never needs an
     * independent full-file audio cache.
     */
    fun extractorSource(source: MediaRequest): MediaDataSource =
        object : MediaDataSource() {
            private var reader: DataSource? = null
            private var cursor = -1L
            private var length =
                ContentMetadata.getContentLength(cache.getContentMetadata(key(source)))

            @Synchronized
            override fun readAt(position: Long, buffer: ByteArray, offset: Int, size: Int): Int {
                if (size == 0) return 0
                if (length >= 0 && position >= length) return -1
                if (reader == null || cursor != position) {
                    reader?.close()
                    reader = factory(source).createDataSource()
                    val remaining =
                        reader!!.open(
                            DataSpec.Builder()
                                .setUri(source.url)
                                .setKey(key(source))
                                .setPosition(position)
                                .build()
                        )
                    if (remaining != C.LENGTH_UNSET.toLong()) length = position + remaining
                    cursor = position
                }
                val count = reader!!.read(buffer, offset, size)
                if (count > 0) cursor += count
                return count
            }

            @Synchronized override fun getSize() = length

            @Synchronized
            override fun close() {
                reader?.close()
                reader = null
            }
        }

    internal suspend fun clear() =
        withContext(Dispatchers.IO) {
            // Active readers retain their open span; removal is safe in SimpleCache.
            cache.keys.toList().forEach { key ->
                locks.getOrPut(key) { Mutex() }.withLock { cache.removeResource(key) }
            }
        }

    internal val size
        get() = cache.cacheSpace

    override fun close() {
        cache.release()
    }

    companion object {
        private const val MAX_IMAGE = 32 * 1024 * 1024
        private const val CHECKED = "custom_papo_checked"
        private const val ETAG = "custom_papo_etag"
        private const val MODIFIED = "custom_papo_modified"
        @Volatile private var shared: MediaCache? = null

        @Synchronized
        fun get(context: Context): MediaCache =
            shared
                ?: MediaCache(
                        context.applicationContext,
                        context.cacheDir.resolve("media-cache-v2"),
                        256 * 1024 * 1024L,
                        System::currentTimeMillis,
                        15 * 60 * 1000L,
                    )
                    .also {
                        shared = it
                        // These were disposable caches from earlier development builds.
                        // User-owned GIF favorites, downloads and upload drafts stay intact.
                        kotlinx.coroutines
                            .CoroutineScope(kotlinx.coroutines.SupervisorJob() + Dispatchers.IO)
                            .launch {
                                listOf(
                                        "audio",
                                        "waveforms",
                                        "media",
                                        "coil3_disk_cache",
                                        "media-cache-v1",
                                    )
                                    .forEach { name ->
                                        context.cacheDir.resolve(name).deleteRecursively()
                                    }
                                context.filesDir.resolve("external-avatars").deleteRecursively()
                            }
                    }

        internal fun isolated(
            context: Context,
            directory: File,
            budget: Long = 256 * 1024 * 1024L,
            now: () -> Long = System::currentTimeMillis,
            freshFor: Long = 15 * 60 * 1000L,
        ) = MediaCache(context.applicationContext, directory, budget, now, freshFor)

        private fun readBounded(input: java.io.InputStream, limit: Int): ByteArray {
            val output = ByteArrayOutputStream()
            val buffer = ByteArray(8192)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                if (output.size() + count > limit)
                    throw IOException(ui(R.string.image_is_too_large))
                output.write(buffer, 0, count)
            }
            return output.toByteArray().also {
                if (it.isEmpty()) throw IOException(ui(R.string.empty_image))
            }
        }
    }
}
