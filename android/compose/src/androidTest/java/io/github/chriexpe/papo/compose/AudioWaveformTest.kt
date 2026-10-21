package io.github.chriexpe.papo.compose

import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.sin

class AudioWaveformTest {
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private fun wav(silent: Boolean): File {
        val rate=16000; val frames=rate*2; val bytes=frames*4
        val b=ByteBuffer.allocate(44+bytes).order(ByteOrder.LITTLE_ENDIAN)
        b.put("RIFF".toByteArray()).putInt(36+bytes).put("WAVEfmt ".toByteArray()).putInt(16).putShort(1).putShort(2).putInt(rate).putInt(rate*4).putShort(4).putShort(16).put("data".toByteArray()).putInt(bytes)
        repeat(frames) { i ->
            val amplitude=if(silent) 0.0 else if(i<rate) .02 else .8
            val sample=(sin(i*2*Math.PI*440/rate)*amplitude*32767).toInt().toShort()
            b.putShort(sample); b.putShort((-sample).toShort()) // opposite phases must not cancel
        }
        return File.createTempFile("wave-test-", ".wav",context.cacheDir).apply {writeBytes(b.array())}
    }
    @Test fun decoderMeasuresTemporalPeaksAcrossStereoChannels() = runBlocking {
        val file=wav(false)
        try {
            val data=AudioWaveformDecoder.decode(file)
            assertEquals(96,data.levels.size); assertEquals(2000L,data.durationMs)
            assertTrue(data.levels.all {it.isFinite() && it in 0f..1f})
            assertTrue(data.levels.take(35).average()<.3)
            assertTrue(data.levels.takeLast(35).average()>.8)
        } finally {file.delete()}
    }
    @Test fun silenceStaysSilentAndActualM4aDecodes() = runBlocking {
        val silent=wav(true)
        val aac=File.createTempFile("wave-aac-", ".m4a",context.cacheDir)
        try {
            assertTrue(AudioWaveformDecoder.decode(silent).levels.all {it==0f})
            context.assets.open("demo/voice-note.m4a").use {input->aac.outputStream().use {input.copyTo(it)}}
            val data=AudioWaveformDecoder.decode(aac)
            assertTrue(data.durationMs>1500); assertTrue(data.levels.any {it>.5f})
        } finally {silent.delete(); aac.delete()}
    }
    @Test fun unknownDurationEnvelopeRetainsTimeOrderWithBoundedStorage() {
        val peaks=PeakEnvelope()
        repeat(300_000) {peaks.add(if(it<150_000) .01f else 1f)}
        val levels=peaks.finish()
        assertEquals(96,levels.size)
        assertTrue(levels.take(30).average()<.2)
        assertTrue(levels.takeLast(30).average()>.9)
    }
}
