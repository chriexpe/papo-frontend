package io.github.chriexpe.papo.compose

import androidx.compose.material3.Typography
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal enum class AccountFontSize(val value: String, val scale: Float, val labelId: Int) {
    Small("small", .875f, R.string.text_small),
    Normal("medium", 1f, R.string.text_normal),
    Large("huge", 1.25f, R.string.text_large);

    companion object {
        fun from(value: String?) = entries.firstOrNull { it.value == value } ?: Normal
    }
}

internal enum class MessageDensity(val value: String, val labelId: Int) {
    Compact("compact", R.string.density_compact),
    Normal("normal", R.string.density_normal),
    Comfortable("comfortable", R.string.density_comfortable),
    ExtraCompact("extra-compact", R.string.density_extra_compact);

    val localPreview get() = this == ExtraCompact

    companion object {
        fun from(value: String?) =
            entries.firstOrNull { it.value == value } ?:
                if (value?.startsWith("extra-compact") == true) ExtraCompact else Normal
    }
}

/** Account typography composes with Android fontScale; it never replaces LocalDensity. */
@Immutable
internal data class MessageAppearance(
    val font: AccountFontSize = AccountFontSize.Normal,
    val density: MessageDensity = MessageDensity.Normal,
    val stacked: Boolean = true,
    val grouped: Boolean = true,
) {
    private fun <T> choose(compact: T, normal: T, comfortable: T) =
        when (density) {
            MessageDensity.Compact,
            MessageDensity.ExtraCompact -> compact
            MessageDensity.Normal -> normal
            MessageDensity.Comfortable -> comfortable
        }

    val lineHeight
        get() = ((if (extraCompact) 20f else choose(22f, 24f, 28f)) * font.scale).sp

    val extraCompact
        get() = density.localPreview

    val inlineAuthor
        get() = extraCompact && !stacked

    val continuationGap
        get() = choose(0.dp, 2.dp, 6.dp)

    val authorGap
        get() = if (extraCompact) 6.dp else choose(4.dp, 8.dp, 16.dp)

    val headingGap
        get() = if (inlineAuthor) 0.dp else choose(2.dp, 2.dp, 4.dp)

    val contentGap
        get() = choose(2.dp, 4.dp, 8.dp)
}

internal val LocalMessageAppearance = staticCompositionLocalOf { MessageAppearance() }

private val baseTypography = Typography()

internal fun messageTypography(font: AccountFontSize): Typography {
    if (font == AccountFontSize.Normal) return baseTypography
    fun TextStyle.scaled() =
        copy(
            fontSize = (fontSize.value * font.scale).coerceAtLeast(11f).sp,
            lineHeight = (lineHeight.value * font.scale).coerceAtLeast(14f).sp,
        )
    return baseTypography.run {
        copy(
            displayLarge = displayLarge.scaled(),
            displayMedium = displayMedium.scaled(),
            displaySmall = displaySmall.scaled(),
            headlineLarge = headlineLarge.scaled(),
            headlineMedium = headlineMedium.scaled(),
            headlineSmall = headlineSmall.scaled(),
            titleLarge = titleLarge.scaled(),
            titleMedium = titleMedium.scaled(),
            titleSmall = titleSmall.scaled(),
            bodyLarge = bodyLarge.scaled(),
            bodyMedium = bodyMedium.scaled(),
            bodySmall = bodySmall.scaled(),
            labelLarge = labelLarge.scaled(),
            labelMedium = labelMedium.scaled(),
            labelSmall = labelSmall.scaled(),
        )
    }
}
