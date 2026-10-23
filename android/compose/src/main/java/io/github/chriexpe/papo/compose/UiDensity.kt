package io.github.chriexpe.papo.compose

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.unit.dp

internal enum class UiDensity(val value: String) {
    Compact("compact"),
    Normal("normal"),
    Comfortable("comfortable");

    val label: String
        get() =
            ui(
                when (this) {
                    Compact -> R.string.density_compact
                    Normal -> R.string.density_normal
                    Comfortable -> R.string.density_comfortable
                }
            )
}

/**
 * Spacing changes independently of Android density/fontScale. Hit regions never shrink below 48dp.
 */
@Immutable
internal data class UiDimensions(val density: UiDensity = UiDensity.Normal) {
    private fun <T> choose(compact: T, normal: T, comfortable: T): T =
        when (density) {
            UiDensity.Compact -> compact
            UiDensity.Normal -> normal
            UiDensity.Comfortable -> comfortable
        }

    val touchSize = 48.dp
    val screenPadding = choose(12.dp, 20.dp, 24.dp)
    val floatingPadding = choose(8.dp, 12.dp, 16.dp)
    val surfaceInset = choose(2.dp, 6.dp, 10.dp)
    val contentGap = choose(8.dp, 12.dp, 16.dp)
    val headerHeight = touchSize + surfaceInset * 2
    val headerIcon = choose(36.dp, 40.dp, 44.dp)
    val avatarInset = choose(4.dp, 2.dp, 0.dp)
    val rowPadding = choose(12.dp, 16.dp, 20.dp)
    val rowVerticalPadding = choose(4.dp, 8.dp, 12.dp)
    val rowHeight = choose(48.dp, 56.dp, 64.dp)
    val listIcon = choose(36.dp, 44.dp, 48.dp)
    val navigationPadding = choose(4.dp, 6.dp, 8.dp)
    val navigationItemHeight = choose(48.dp, 58.dp, 66.dp)
    val navigationWidth = choose(64.dp, 72.dp, 80.dp)
    val globalActionHeight = choose(72.dp, 88.dp, 104.dp)
    val settingsLinkHeight = choose(64.dp, 80.dp, 96.dp)
    val composerPadding = choose(8.dp, 12.dp, 16.dp)
    val composerGap = choose(6.dp, 10.dp, 14.dp)
    val composerTextVertical = choose(8.dp, 12.dp, 16.dp)
    val attachmentSize = headerHeight
}

internal val LocalUiDimensions = staticCompositionLocalOf { UiDimensions() }
