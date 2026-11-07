package io.github.chriexpe.papo.sdkproof

import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.mutableStateListOf
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.chriexpe.papo.sdk.proof.ProofChange
import io.github.chriexpe.papo.sdk.proof.ProofEmitter
import io.github.chriexpe.papo.sdk.proof.ProofItem
import io.github.chriexpe.papo.sdk.proof.ProofListener
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** An actual Rust worker → UniFFI → Android main Looper → Compose list round trip. */
@RunWith(AndroidJUnit4::class)
class NativeDiffProofTest {
    @Test
    fun nativeBatchesReachComposeStateOnMainThreadInOrder() {
        val main = Handler(Looper.getMainLooper())
        val received = CountDownLatch(3)
        val firstError = AtomicReference<Throwable>()
        val state = mutableStateListOf<ProofItem>()
        val snapshots = mutableListOf<List<String>>()
        val callback =
            object : ProofListener {
                override fun onBatch(changes: List<ProofChange>) {
                    // The native producer must not call the UI directly on its worker.
                    if (Looper.myLooper() == Looper.getMainLooper()) {
                        firstError.compareAndSet(null, AssertionError("callback ran on UI thread"))
                    }
                    main.post {
                        try {
                            assertTrue(Looper.myLooper() == Looper.getMainLooper())
                            for (change in changes) {
                                when (change) {
                                    is ProofChange.Insert -> state.add(change.index.toInt(), change.value)
                                    is ProofChange.Set -> state[change.index.toInt()] = change.value
                                    is ProofChange.PushBack -> state.add(change.value)
                                    is ProofChange.Reset -> {
                                        state.clear()
                                        state.addAll(change.values)
                                    }
                                }
                            }
                            snapshots.add(state.map { it.id + ":" + it.title })
                        } catch (error: Throwable) {
                            firstError.compareAndSet(null, error)
                        } finally {
                            received.countDown()
                        }
                    }
                }
            }

        val emitter = ProofEmitter(callback)
        try {
            emitter.emitFromWorker()
            assertTrue("Timed out waiting for the three native diff batches", received.await(15, TimeUnit.SECONDS))
            firstError.get()?.let { throw it }
            assertEquals(
                listOf(
                    listOf("a:Alpha", "c:Charlie"),
                    listOf("a:Alpha edited", "b:Bravo", "c:Charlie", "d:Delta"),
                    listOf("z:After reset"),
                ),
                snapshots,
            )
            assertEquals(listOf("z"), state.map { it.id })
            assertNull(firstError.get())
        } finally {
            emitter.close()
        }
    }
}
