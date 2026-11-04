package io.github.chriexpe.papo.compose

import org.junit.Assert.*
import org.junit.Test

class ViewportStateTest {
    @Test
    fun restoringLocksUntilFirstPositionHasBeenDecided() {
        val initial = ViewportPhase.Restoring
        assertTrue(initial.locked)
        val free = reduceViewport(initial, ViewportEvent.Restored(following = true))
        assertEquals(ViewportPhase.Free(following = true), free)
        assertTrue(viewportCanReport(free))
    }

    @Test
    fun latestIgnoresCancellationUntilScrollSettles() {
        var phase: ViewportPhase = ViewportPhase.Free(following = false)
        phase = reduceViewport(phase, ViewportEvent.LatestRequested)
        assertEquals(ViewportPhase.ReturningToLatest, phase)
        assertTrue(phase.locked)
        phase = reduceViewport(phase, ViewportEvent.Terminal(5uL, NavigationResult.Cancelled))
        assertEquals(ViewportPhase.ReturningToLatest, phase)
        assertFalse(viewportCanReport(phase))
        phase = reduceViewport(phase, ViewportEvent.Settled)
        assertEquals(ViewportPhase.Free(following = true), phase)
    }

    @Test
    fun previousNavigationTerminalCannotUnlockNewerJump() {
        var phase: ViewportPhase = ViewportPhase.Free(following = true)
        phase = reduceViewport(phase, ViewportEvent.JumpRequested(6uL, "old"))
        phase = reduceViewport(phase, ViewportEvent.JumpRequested(7uL, "new"))
        phase = reduceViewport(phase, ViewportEvent.Terminal(6uL, NavigationResult.Failed))
        assertEquals(ViewportPhase.Navigating(7uL, "new", true), phase)
        phase = reduceViewport(phase, ViewportEvent.Terminal(7uL, NavigationResult.Located))
        assertTrue(phase.locked)
        phase = reduceViewport(phase, ViewportEvent.Positioned(7uL))
        assertEquals(ViewportPhase.Free(following = false), phase)
    }

    @Test
    fun changedChannelCancelsJumpAndRestoresWithoutStuckLock() {
        var phase: ViewportPhase = ViewportPhase.Free(false)
        phase = reduceViewport(phase, ViewportEvent.JumpRequested(11uL, "gone"))
        phase = reduceViewport(phase, ViewportEvent.ChannelChanged)
        assertEquals(ViewportPhase.Restoring, phase)
        phase = reduceViewport(phase, ViewportEvent.Terminal(11uL, NavigationResult.Cancelled))
        assertEquals(ViewportPhase.Restoring, phase)
        phase = reduceViewport(phase, ViewportEvent.Restored(following = true))
        assertEquals(ViewportPhase.Free(following = true), phase)
    }

    @Test
    fun deletedTargetReleasesLockButPreservesFollowIntent() {
        var phase: ViewportPhase = ViewportPhase.Free(following = true)
        phase = reduceViewport(phase, ViewportEvent.JumpRequested(19uL, "deleted"))
        phase = reduceViewport(phase, ViewportEvent.Terminal(19uL, NavigationResult.Failed))
        assertEquals(ViewportPhase.Free(following = true), phase)
    }

    @Test
    fun autoScrollingNeverBlocksReadReportsOnlyNearBottomIntent() {
        var phase: ViewportPhase = ViewportPhase.Free(following = true)
        phase = reduceViewport(phase, ViewportEvent.AutoScrollStarted)
        assertTrue(viewportCanReport(phase))
        phase = reduceViewport(phase, ViewportEvent.NearBottomChanged(false))
        assertEquals(ViewportPhase.Free(following = true, autoScrolling = true), phase)
        phase = reduceViewport(phase, ViewportEvent.AutoScrollFinished)
        phase = reduceViewport(phase, ViewportEvent.NearBottomChanged(false))
        assertEquals(ViewportPhase.Free(following = false), phase)
        assertTrue(viewportCanReport(phase))
    }

    @Test
    fun disposedRestartsFromUnreadyPhase() {
        val next = reduceViewport(ViewportPhase.ReturningToLatest, ViewportEvent.Disposed)
        assertEquals(ViewportPhase.Restoring, next)
    }
}
