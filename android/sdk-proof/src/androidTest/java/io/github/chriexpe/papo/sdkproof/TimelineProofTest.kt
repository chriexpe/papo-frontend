package io.github.chriexpe.papo.sdkproof

import android.os.Handler
import android.os.Looper
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.util.concurrent.atomic.AtomicReference
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertSame
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.papo_ffi.ProofBatch
import uniffi.papo_ffi.ProofItem
import uniffi.papo_ffi.ProofListener
import uniffi.papo_ffi.runTimelineProof

@RunWith(AndroidJUnit4::class)
class TimelineProofTest {
    @Test
    fun rustWorkerBatchesReachTheMainThreadInOrderAndKeepTheAnchorById() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val main = Handler(Looper.getMainLooper())
        val caller = Thread.currentThread()
        val failure = AtomicReference<Throwable?>()
        val versions = mutableListOf<ULong>()
        lateinit var list: ProofList
        lateinit var unchanged: ProofItem
        lateinit var edited: ProofItem
        instrumentation.runOnMainSync { list = ProofList() }

        fun enqueue(apply: () -> Unit) {
            try {
                assertNotSame(caller, Thread.currentThread())
                assertNotSame(Looper.getMainLooper().thread, Thread.currentThread())
                assertTrue(
                    main.post {
                        try {
                            assertSame(Looper.getMainLooper(), Looper.myLooper())
                            apply()
                        } catch (error: Throwable) {
                            failure.compareAndSet(null, error)
                        }
                    }
                )
            } catch (error: Throwable) {
                failure.compareAndSet(null, error)
            }
        }

        // This call joins the Rust worker. A returned worker error fails the test.
        // Callbacks only post; they never wait for main-thread application.
        runTimelineProof(
            object : ProofListener {
                override fun onSnapshot(version: ULong, items: List<ProofItem>) {
                    enqueue {
                        list.snapshot(version, items)
                        versions.add(version)
                        assertEquals(listOf(30uL, 40uL), list.items.map { it.id })
                        unchanged = list.items[1]
                        edited = list.items[0]
                        assertEquals(Anchor.Kept(30uL, 0, 17), list.anchor())
                    }
                }

                override fun onBatch(batch: ProofBatch) {
                    enqueue {
                        list.apply(batch)
                        versions.add(batch.version)
                        val expectedIds =
                            when (batch.version) {
                                41uL,
                                42uL -> listOf(30uL, 40uL, 50uL)
                                43uL -> listOf(10uL, 20uL, 30uL, 40uL, 50uL)
                                44uL -> listOf(5uL, 10uL, 20uL, 30uL, 40uL, 50uL)
                                45uL -> listOf(1uL, 5uL, 10uL, 20uL, 30uL, 40uL, 50uL)
                                46uL -> listOf(20uL, 30uL, 40uL, 50uL)
                                47uL -> listOf(50uL)
                                else -> error("unexpected proof version")
                            }
                        assertEquals(expectedIds, list.items.map { it.id })
                        val anchor =
                            when (batch.version) {
                                41uL,
                                42uL -> Anchor.Kept(30uL, 0, 17)
                                43uL -> Anchor.Kept(30uL, 2, 17)
                                44uL -> Anchor.Kept(30uL, 3, 17)
                                45uL -> Anchor.Kept(30uL, 4, 17)
                                46uL -> Anchor.Kept(30uL, 1, 17)
                                else -> Anchor.Missing(30uL)
                            }
                        assertEquals(anchor, list.anchor())
                        if (batch.version <= 45uL) {
                            assertSame(unchanged, list.items.first { it.id == 40uL })
                        }
                        if (batch.version == 41uL) assertSame(edited, list.items[0])
                        if (batch.version == 42uL) {
                            assertNotSame(edited, list.items[0])
                            assertEquals(30uL, list.items[0].id)
                            assertEquals(1u, list.items[0].revision)
                            edited = list.items[0]
                        }
                        if (batch.version in 43uL..45uL) {
                            assertSame(edited, list.items.first { it.id == 30uL })
                        }
                        if (batch.version == 46uL) {
                            assertNotSame(edited, list.items[1])
                            assertEquals(2u, list.items[1].revision)
                        }
                    }
                }
            }
        )
        instrumentation.waitForIdleSync()
        failure.get()?.let { throw AssertionError("worker/main-thread proof failed", it) }
        instrumentation.runOnMainSync {
            assertEquals((40uL..47uL).toList(), versions)
            assertEquals(47uL, list.version)
            assertEquals(Anchor.Missing(30uL), list.anchor())
        }
    }

    @Test
    fun unsignedIndicesNeverWrapIntoSignedKotlinListIndices() {
        assertEquals(Int.MAX_VALUE, Int.MAX_VALUE.toUInt().listIndex(Int.MAX_VALUE, true))
        assertThrows(ListApplyError.IndexOverflow::class.java) {
            UInt.MAX_VALUE.listIndex(3, true)
        }
        assertThrows(ListApplyError.IndexOutOfBounds::class.java) {
            4u.listIndex(3, true)
        }
        assertThrows(ListApplyError.IndexOutOfBounds::class.java) {
            3u.listIndex(3, false)
        }
        assertEquals(3, 3u.listIndex(3, true))
    }
}
