package io.github.chriexpe.papo.compose

import android.content.Context
import android.media.AudioFormat
import android.media.MediaCodec
import android.media.MediaExtractor
import android.media.MediaFormat
import android.util.LruCache
import io.github.chriexpe.papo.core.MediaRequest
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.File
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.security.MessageDigest
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.sqrt
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.async
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext

internal data class AudioWaveform(val levels: FloatArray, val durationMs: Long)

/** Decodes PCM in bounded buffers, including all channels. No microphone
 * permission, playback or platform visualizer is needed to inspect a file. */
internal object AudioWaveformDecoder {
    suspend fun decode(file:File):AudioWaveform=decode {it.setDataSource(file.absolutePath)}
    suspend fun decode(source:android.media.MediaDataSource):AudioWaveform=decode {it.setDataSource(source)}
    private suspend fun decode(configure:(MediaExtractor)->Unit):AudioWaveform = withContext(Dispatchers.IO) {
        val extractor = MediaExtractor()
        var codec: MediaCodec? = null
        var started = false
        try {
            configure(extractor)
            val track = (0 until extractor.trackCount).firstOrNull {
                extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true
            } ?: throw IOException("No audio track")
            extractor.selectTrack(track)
            val format = extractor.getTrackFormat(track)
            var rate = format.getInteger(MediaFormat.KEY_SAMPLE_RATE)
            var channels = format.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
            val durationUs = if (format.containsKey(MediaFormat.KEY_DURATION)) format.getLong(MediaFormat.KEY_DURATION).coerceAtLeast(0) else 0
            val peaks = PeakEnvelope(durationUs * rate / 1_000_000)
            val decoder = MediaCodec.createDecoderByType(requireNotNull(format.getString(MediaFormat.KEY_MIME)))
            codec = decoder
            format.setInteger(MediaFormat.KEY_PCM_ENCODING, AudioFormat.ENCODING_PCM_16BIT)
            decoder.configure(format, null, null, 0)
            decoder.start(); started = true
            var encoding = AudioFormat.ENCODING_PCM_16BIT
            var inputEnded = false
            var outputEnded = false
            val info = MediaCodec.BufferInfo()
            var lastProgress = android.os.SystemClock.elapsedRealtime()
            while (!outputEnded) {
                currentCoroutineContext().ensureActive()
                if (!inputEnded) {
                    val index = decoder.dequeueInputBuffer(10_000)
                    if (index >= 0) {
                        val input = requireNotNull(decoder.getInputBuffer(index))
                        val length = extractor.readSampleData(input, 0)
                        if (length < 0) {
                            decoder.queueInputBuffer(index, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                            inputEnded = true
                        } else {
                            decoder.queueInputBuffer(index, 0, length, extractor.sampleTime.coerceAtLeast(0), 0)
                            extractor.advance()
                        }
                        lastProgress = android.os.SystemClock.elapsedRealtime()
                    }
                }
                when (val index = decoder.dequeueOutputBuffer(info, 10_000)) {
                    MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                        val output = decoder.outputFormat
                        rate = output.getInteger(MediaFormat.KEY_SAMPLE_RATE)
                        channels = output.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
                        encoding = if (output.containsKey(MediaFormat.KEY_PCM_ENCODING)) output.getInteger(MediaFormat.KEY_PCM_ENCODING) else AudioFormat.ENCODING_PCM_16BIT
                    }
                    else -> if (index >= 0) {
                        try {
                            if (info.size > 0 && info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0) {
                                val buffer = requireNotNull(decoder.getOutputBuffer(index)).duplicate().order(ByteOrder.LITTLE_ENDIAN)
                                buffer.position(info.offset); buffer.limit(info.offset + info.size)
                                addPcm(buffer, encoding, channels, peaks)
                            }
                            outputEnded = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0
                            lastProgress = android.os.SystemClock.elapsedRealtime()
                        } finally { decoder.releaseOutputBuffer(index, false) }
                    }
                }
                if (android.os.SystemClock.elapsedRealtime() - lastProgress > 10_000) throw IOException("Audio decoder stalled")
            }
            check(peaks.frames > 0 && rate > 0) { "Audio has no decoded samples" }
            AudioWaveform(peaks.finish(), if (durationUs > 0) durationUs / 1000 else (peaks.frames * 1000 / rate).coerceAtLeast(1))
        } finally {
            if (started) runCatching { codec?.stop() }
            try { codec?.release() } finally { extractor.release() }
        }
    }

    private fun addPcm(buffer: ByteBuffer, encoding: Int, channels: Int, peaks: PeakEnvelope) {
        require(channels > 0) { "Invalid channel count" }
        val bytes = when (encoding) {
            AudioFormat.ENCODING_PCM_8BIT -> 1
            AudioFormat.ENCODING_PCM_16BIT -> 2
            AudioFormat.ENCODING_PCM_24BIT_PACKED -> 3
            AudioFormat.ENCODING_PCM_FLOAT, AudioFormat.ENCODING_PCM_32BIT -> 4
            else -> throw IOException("Unsupported PCM encoding")
        }
        while (buffer.remaining() >= channels * bytes) {
            var peak = 0f
            repeat(channels) {
                val sample = when (encoding) {
                    AudioFormat.ENCODING_PCM_8BIT -> ((buffer.get().toInt() and 255) - 128) / 128f
                    AudioFormat.ENCODING_PCM_16BIT -> buffer.short / 32768f
                    AudioFormat.ENCODING_PCM_FLOAT -> buffer.float
                    AudioFormat.ENCODING_PCM_32BIT -> buffer.int / 2147483648f
                    else -> {
                        val value = (buffer.get().toInt() and 255) or ((buffer.get().toInt() and 255) shl 8) or (buffer.get().toInt() shl 16)
                        value / 8388608f
                    }
                }
                if (sample.isFinite()) peak = max(peak, abs(sample).coerceAtMost(1f))
            }
            peaks.add(peak)
        }
    }
}

/** Fixed memory even when duration is missing or incorrect. Compress adjacent
 * time windows when full, then resample against the actual decoded length. */
internal class PeakEnvelope(expectedFrames: Long = 0) {
    private val peaks = FloatArray(256)
    private var window = ((expectedFrames + peaks.size - 1) / peaks.size).coerceAtLeast(1)
    var frames = 0L
        private set
    fun add(peak: Float) {
        if (frames / window >= peaks.size) {
            for (i in 0 until peaks.size / 2) peaks[i] = max(peaks[i * 2], peaks[i * 2 + 1])
            peaks.fill(0f, peaks.size / 2)
            window *= 2
        }
        val index = (frames / window).toInt()
        peaks[index] = max(peaks[index], peak)
        frames++
    }
    fun finish(): FloatArray {
        val maximum = peaks.maxOrNull() ?: 0f
        if (maximum < 0.0001f || frames == 0L) return FloatArray(96)
        return FloatArray(96) { i ->
            val start = (frames * i / 96 / window).toInt().coerceIn(peaks.indices)
            val end = ((frames * (i + 1) / 96).coerceAtLeast(1) - 1).div(window).toInt().coerceIn(peaks.indices)
            var peak = 0f
            for (p in start..max(start, end)) peak = max(peak, peaks[p])
            sqrt(peak / maximum).coerceIn(0f, 1f)
        }
    }
}

/** Account-scoped cache with shared work and a maximum of two decoders.
 * A scrolled-away row cancels work unless another row still needs it. */
internal class AudioWaveforms(private val context: Context, private val scope: CoroutineScope) {
    private class Work(val result: Deferred<AudioWaveform>, var readers: Int = 0)
    private val lock = Mutex()
    private val jobs = mutableMapOf<String, Work>()
    private val memory = LruCache<String, AudioWaveform>(128)
    private val decoders = Semaphore(2)
    private val mediaCache=MediaCache.get(context)
    suspend fun load(source: MediaRequest): AudioWaveform {
        val key = MessageDigest.getInstance("SHA-256").digest(("v1/" + source.cacheKey).toByteArray()).joinToString("") { "%02x".format(it) }
        val work = lock.withLock {
            memory.get(key)?.let { return it }
            jobs.getOrPut(key) {
                Work(scope.async(Dispatchers.IO) {
                    decoders.withPermit {
                        val bytes=mediaCache.blob("waveform/$key",4096) {
                            val decoded=mediaCache.extractorSource(source).use {AudioWaveformDecoder.decode(it)}
                            val output=java.io.ByteArrayOutputStream()
                            DataOutputStream(output).use {stream ->
                                stream.writeInt(1);stream.writeLong(decoded.durationMs);stream.writeInt(decoded.levels.size);decoded.levels.forEach(stream::writeFloat)
                            }
                            output.toByteArray()
                        }
                        val result=requireNotNull(read(bytes))
                        lock.withLock { memory.put(key, result) }
                        result
                    }
                })
            }.also { it.readers++ }
        }
        try { return work.result.await() }
        finally {
            withContext(NonCancellable) {
                lock.withLock {
                    work.readers--
                    if (work.readers == 0 && jobs[key] === work) {
                        jobs.remove(key)
                        if (!work.result.isCompleted) work.result.cancel()
                    }
                }
            }
        }
    }
    private fun read(bytes:ByteArray):AudioWaveform?=runCatching {
        DataInputStream(bytes.inputStream()).use {input ->
            require(input.readInt()==1)
            val duration=input.readLong();require(duration>0 && input.readInt()==96)
            AudioWaveform(FloatArray(96) {input.readFloat().also {require(it.isFinite() && it in 0f..1f)}},duration)
        }
    }.getOrNull()
}
