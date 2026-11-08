package io.github.chriexpe.papo.sdkproof

import android.os.Looper
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.snapshots.Snapshot
import uniffi.papo_ffi.ProofBatch
import uniffi.papo_ffi.ProofDiff
import uniffi.papo_ffi.ProofItem

/** Test-only consumer; list state and the scroll anchor belong to the client. */
internal class ProofList {
    val items = mutableStateListOf<ProofItem>()
    var version = 0uL
        private set

    val anchorId = 30uL
    val offsetPx = 17

    fun snapshot(version: ULong, values: List<ProofItem>) {
        checkMainThread()
        Snapshot.withMutableSnapshot {
            items.clear()
            items.addAll(values)
        }
        this.version = version
    }

    fun apply(batch: ProofBatch) {
        checkMainThread()
        if (batch.version != version + 1uL) throw ListApplyError.UnexpectedVersion()
        Snapshot.withMutableSnapshot {
            batch.diffs.forEach { diff ->
                when (diff) {
                    is ProofDiff.PushBack -> items.add(diff.value)
                    is ProofDiff.Set -> items[diff.index.listIndex(items.size, false)] = diff.value
                    is ProofDiff.Insert ->
                        items.add(diff.index.listIndex(items.size, true), diff.value)
                    is ProofDiff.Reset -> {
                        items.clear()
                        items.addAll(diff.values)
                    }
                }
            }
        }
        version = batch.version
    }

    fun anchor(): Anchor {
        checkMainThread()
        val index = items.indexOfFirst { it.id == anchorId }
        return if (index < 0) Anchor.Missing(anchorId) else Anchor.Kept(anchorId, index, offsetPx)
    }

    private fun checkMainThread() {
        check(Looper.myLooper() == Looper.getMainLooper())
    }
}

internal sealed interface Anchor {
    data class Kept(val id: ULong, val index: Int, val offsetPx: Int) : Anchor

    data class Missing(val id: ULong) : Anchor
}

internal sealed class ListApplyError : Exception() {
    class IndexOverflow : ListApplyError()

    class IndexOutOfBounds : ListApplyError()

    class UnexpectedVersion : ListApplyError()
}

/** No unsigned-to-signed wrapping or unchecked list indexing at the boundary. */
internal fun UInt.listIndex(size: Int, insertion: Boolean): Int {
    if (this > Int.MAX_VALUE.toUInt()) throw ListApplyError.IndexOverflow()
    val index = toInt()
    if (index > size || (!insertion && index == size)) throw ListApplyError.IndexOutOfBounds()
    return index
}
