package io.github.chriexpe.papo.compose

/**
 * The only owner of the viewport read-report lock. Free.autoScrolling guards user intent updates,
 * never read reporting; history loading and overlays are independent gates at the call site.
 */
internal sealed interface ViewportPhase {
    val locked: Boolean
        get() = this !is Free

    data object Restoring : ViewportPhase

    data class Free(val following: Boolean, val autoScrolling: Boolean = false) : ViewportPhase

    data class Navigating(
        val serial: ULong,
        val target: String,
        val previousFollowing: Boolean = false,
    ) : ViewportPhase

    data object ReturningToLatest : ViewportPhase
}

internal enum class NavigationResult {
    Located,
    Unavailable,
    Failed,
    Cancelled,
}

internal sealed interface ViewportEvent {
    data class Restored(val following: Boolean) : ViewportEvent

    data class JumpRequested(val serial: ULong, val target: String) : ViewportEvent

    data class Positioned(val serial: ULong) : ViewportEvent

    data class Terminal(val serial: ULong, val result: NavigationResult) : ViewportEvent

    data object LatestRequested : ViewportEvent

    data object Settled : ViewportEvent

    data class NearBottomChanged(val nearBottom: Boolean) : ViewportEvent

    data object AutoScrollStarted : ViewportEvent

    data object AutoScrollFinished : ViewportEvent

    data object ChannelChanged : ViewportEvent

    data object Disposed : ViewportEvent
}

/** Pure transition function; it never starts animations, timers or core commands. */
internal fun reduceViewport(phase: ViewportPhase, event: ViewportEvent): ViewportPhase =
    when (event) {
        is ViewportEvent.Restored ->
            if (phase is ViewportPhase.Restoring) ViewportPhase.Free(event.following) else phase
        is ViewportEvent.JumpRequested -> {
            val previousFollowing =
                when (phase) {
                    is ViewportPhase.Free -> phase.following
                    is ViewportPhase.Navigating -> phase.previousFollowing
                    ViewportPhase.ReturningToLatest -> true
                    ViewportPhase.Restoring -> false
                }
            ViewportPhase.Navigating(event.serial, event.target, previousFollowing)
        }
        is ViewportEvent.Positioned ->
            if (phase is ViewportPhase.Navigating && phase.serial == event.serial)
                ViewportPhase.Free(following = false)
            else phase
        is ViewportEvent.Terminal ->
            if (
                phase is ViewportPhase.Navigating &&
                    phase.serial == event.serial &&
                    event.result != NavigationResult.Located
            )
                ViewportPhase.Free(following = phase.previousFollowing)
            else phase
        ViewportEvent.LatestRequested -> ViewportPhase.ReturningToLatest
        ViewportEvent.Settled ->
            if (phase is ViewportPhase.ReturningToLatest) ViewportPhase.Free(following = true)
            else phase
        is ViewportEvent.NearBottomChanged ->
            if (phase is ViewportPhase.Free && !phase.autoScrolling)
                phase.copy(following = event.nearBottom)
            else phase
        ViewportEvent.AutoScrollStarted ->
            if (phase is ViewportPhase.Free) phase.copy(autoScrolling = true) else phase
        ViewportEvent.AutoScrollFinished ->
            if (phase is ViewportPhase.Free) phase.copy(autoScrolling = false) else phase
        ViewportEvent.ChannelChanged,
        ViewportEvent.Disposed -> ViewportPhase.Restoring
    }

/** Auto-scrolling does not lock read reporting; the caller still checks layout and loading. */
internal fun viewportCanReport(phase: ViewportPhase): Boolean = phase is ViewportPhase.Free

internal fun navigationResult(value: String): NavigationResult? =
    when (value) {
        "located" -> NavigationResult.Located
        "unavailable" -> NavigationResult.Unavailable
        "failed" -> NavigationResult.Failed
        "cancelled" -> NavigationResult.Cancelled
        else -> null
    }
