package io.github.chriexpe.papo.compose

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class StateMachineRegressionTest {
    @Test
    fun navigationTerminalOnlyUnlocksTheMatchingActiveJump() {
        var phase: ViewportPhase = ViewportPhase.Free(following = true)
        phase = reduceViewport(phase, ViewportEvent.JumpRequested(9uL, "target"))
        phase = reduceViewport(phase, ViewportEvent.Terminal(8uL, NavigationResult.Failed))
        assertTrue(phase.locked)
        phase = reduceViewport(phase, ViewportEvent.Terminal(9uL, NavigationResult.Located))
        assertTrue(phase.locked)
        phase = reduceViewport(phase, ViewportEvent.Terminal(9uL, NavigationResult.Unavailable))
        assertEquals(ViewportPhase.Free(following = true), phase)
        phase = reduceViewport(phase, ViewportEvent.LatestRequested)
        phase = reduceViewport(phase, ViewportEvent.Terminal(10uL, NavigationResult.Cancelled))
        assertEquals(ViewportPhase.ReturningToLatest, phase)
        phase = reduceViewport(phase, ViewportEvent.Settled)
        assertTrue(viewportCanReport(phase))
    }

    @Test
    fun typingIsScopedToServerAccountAndChannel() {
        val timestamps = mutableMapOf<Triple<String, String, String>, Long>()
        val first = Triple("server", "alice", "channel-a")
        assertTrue(shouldSendTyping(timestamps, first, 10000))
        assertFalse(shouldSendTyping(timestamps, first, 11000))
        assertTrue(shouldSendTyping(timestamps, Triple("server", "alice", "channel-b"), 11000))
        assertTrue(shouldSendTyping(timestamps, Triple("server", "bob", "channel-a"), 11000))
        assertTrue(shouldSendTyping(timestamps, Triple("other", "alice", "channel-a"), 11000))
        assertTrue(shouldSendTyping(timestamps, first, 13001))
        timestamps.keys.removeAll { it.first == "server" }
        assertTrue(shouldSendTyping(timestamps, first, 13002))
    }
}
