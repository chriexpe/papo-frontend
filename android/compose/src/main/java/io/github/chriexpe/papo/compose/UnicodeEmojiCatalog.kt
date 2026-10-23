package io.github.chriexpe.papo.compose

import android.content.Context
import java.text.Normalizer
import java.util.Locale
import org.json.JSONArray

internal data class UnicodeEmoji(val glyph: String, val keywords: String) {
    internal val searchable = emojiSearchText(keywords)
}

internal fun matchingUnicodeEmoji(catalog: List<UnicodeEmoji>, query: String): List<UnicodeEmoji> {
    val search = emojiSearchText(query)
    return catalog
        .filter { it.glyph == query || it.searchable.contains(search) }
        .distinctBy { it.glyph }
}

/** Picker and autocomplete share one immutable catalog, including its bilingual search aliases. */
internal object UnicodeEmojiCatalog {
    @Volatile private var cached: List<UnicodeEmoji>? = null

    fun load(context: Context): List<UnicodeEmoji> =
        cached
            ?: synchronized(this) {
                cached
                    ?: decode(
                            context.assets.open("emoji.json").bufferedReader().use { it.readText() }
                        )
                        .also { cached = it }
            }

    internal fun decode(json: String): List<UnicodeEmoji> {
        val entries = JSONArray(json)
        return (0 until entries.length())
            .map { index ->
                entries.getJSONObject(index).let {
                    UnicodeEmoji(it.getString("glyph"), it.getString("keywords"))
                }
            }
            .groupBy { it.glyph }
            .map { (glyph, entries) ->
                // One result per glyph, without losing aliases from repeated catalog entries.
                UnicodeEmoji(glyph, entries.map { it.keywords }.distinct().joinToString(" "))
            }
    }
}

// Shortcode separators and accents are search syntax, never changes to the inserted glyph/name.
private val emojiAccents = Regex("\\p{M}+")
private val emojiSeparators = Regex("[_\\-\\s]+")

private fun emojiSearchText(value: String): String =
    Normalizer.normalize(value.trim().lowercase(Locale.ROOT), Normalizer.Form.NFD)
        .replace(emojiAccents, "")
        .replace(emojiSeparators, " ")
